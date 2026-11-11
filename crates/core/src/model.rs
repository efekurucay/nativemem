use std::path::PathBuf;

use sha2::{Digest, Sha256};

use crate::adapters::skills::SkillReport;
use crate::error::{Error, Result};
use crate::session::SessionScan;

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AgentId(pub String);

impl AgentId {
    pub fn new(id: impl Into<String>) -> Self {
        AgentId(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for AgentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// `Project` holds the absolute working directory.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Scope {
    Global,
    Project(PathBuf),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    Memory,
    Instruction,
    Session,
    Skill,
}

impl Kind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Kind::Memory => "memory",
            Kind::Instruction => "instruction",
            Kind::Session => "session",
            Kind::Skill => "skill",
        }
    }

    pub fn parse(s: &str) -> Option<Kind> {
        match s {
            "memory" => Some(Kind::Memory),
            "instruction" => Some(Kind::Instruction),
            "session" => Some(Kind::Session),
            "skill" => Some(Kind::Skill),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Item {
    pub kind: Kind,
    pub scope: Scope,
    pub name: String,
    pub description: String,
    pub category: Option<String>,
    pub body: String,
    /// The directory a skill's files are copied from; the body of a skill is only a manifest.
    /// Not part of the content hash.
    pub source_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ItemKey {
    pub origin: AgentId,
    pub native_id: String,
}

/// One entry found on disk in an agent's native store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observed {
    pub agent: AgentId,
    pub native_id: String,
    pub path: PathBuf,
    pub item: Item,
    pub replica_of: Option<ItemKey>,
}

/// One fresh read of an agent's entries and the diagnostics produced by that same read.
#[derive(Debug, Default)]
pub struct ScanReport {
    pub observed: Vec<Observed>,
    /// `(human-output prefix, native label, reason)`; JSON keeps the native label unchanged.
    pub skipped: Vec<(&'static str, String, String)>,
    pub shadowed_skills: Vec<String>,
    pub automated_sessions: usize,
}

impl ScanReport {
    pub fn skip(&mut self, prefix: &'static str, entries: Vec<(String, String)>) {
        self.skipped.extend(
            entries
                .into_iter()
                .map(|(label, reason)| (prefix, label, reason)),
        );
    }

    pub fn sessions(&mut self, scan: SessionScan) {
        self.observed.extend(scan.observed);
        self.skip("session ", scan.skipped);
        self.automated_sessions = scan.automated;
    }

    pub fn skills(&mut self, observed: Vec<Observed>, report: SkillReport) {
        self.observed.extend(observed);
        self.skip("skill ", report.skipped);
        self.shadowed_skills.extend(report.shadowed);
    }
}

pub trait Adapter {
    fn id(&self) -> AgentId;
    /// The installed version of the agent, when it can be found without running it.
    fn version(&self) -> Option<String> {
        None
    }
    /// Read-only, fast comparisons of the agent's on-disk formats with the ones this adapter was
    /// written against; see `compat`.
    fn probes(&self) -> Vec<crate::compat::Check> {
        Vec::new()
    }
    /// Whether this agent can store an item of `kind` in `scope`. The engine never plans
    /// writes, write-backs or removals for an agent that cannot.
    fn supports(&self, _kind: Kind, _scope: &Scope) -> bool {
        true
    }
    /// Whether this agent keeps `item` for as long as it takes to be useful, for example a
    /// session older than the agent's retention is deleted by the agent soon after it is written.
    /// The engine plans nothing for an item the agent does not accept: no write, no note, no
    /// tombstone for a copy that is missing.
    fn accepts(&self, _item: &Item) -> bool {
        true
    }
    /// Version of this adapter's output format for `kind`. A replica written with a lower
    /// revision is rewritten while it is unmodified; bump it when the written format changes.
    fn writer_revision(&self, _kind: Kind) -> u32 {
        0
    }
    /// Whether `replica` was written by something other than this tool (another program's own
    /// import). Such a replica is adopted into the ledger as it is and never rewritten.
    fn is_foreign_replica(&self, _replica: &Observed) -> bool {
        false
    }
    /// Work deferred until the run's writes are done, for example a database row that must follow
    /// a written file. Returns `(label, reason)` for what had to be left for the next run.
    fn finish(&self) -> Result<Vec<(String, String)>> {
        Ok(Vec::new())
    }
    /// Hash of a copy of the skill `native_id` that this agent reads from a location outside its
    /// own skills directory, when there is one. The engine writes nothing into an agent whose
    /// visible copy already matches.
    fn visible_copy(&self, _native_id: &str) -> Option<String> {
        None
    }
    /// Skills this agent cannot take part with and copies that shadow or are shadowed by a
    /// visible copy with other content.
    fn skill_report(&self) -> Result<SkillReport> {
        Ok(SkillReport::default())
    }
    fn scan(&self) -> Result<Vec<Observed>>;

    /// Read entries and their diagnostics together; the result is local to this call, never cached.
    fn inspect(&self) -> Result<ScanReport> {
        Ok(ScanReport {
            observed: self.scan()?,
            ..ScanReport::default()
        })
    }
    /// Create or update the replica of `key` in this agent. Returns its native_id and path.
    fn write_replica(&self, key: &ItemKey, item: &Item) -> Result<(String, PathBuf)>;
    /// Create a new original in this agent's own store. Returns its native_id and path, and fails
    /// with `Error::Exists` when that native_id is taken.
    fn create_original(&self, _item: &Item) -> Result<(String, PathBuf)> {
        Err(Error::unsupported(self.id().as_str(), "create"))
    }
    /// Update an entry that originated in this agent (an edit made on a replica elsewhere).
    fn write_original(&self, native_id: &str, item: &Item) -> Result<()>;
    /// Remove an entry (original or replica). Must back up first.
    fn remove(&self, native_id: &str) -> Result<()>;
}

fn normalise(s: &str) -> String {
    s.replace("\r\n", "\n").trim_end().to_string()
}

/// SHA-256 hex over name, description, category, body. Each field is length-prefixed so
/// different field splits cannot produce the same byte stream. A session hashes its content
/// only (working directory, title, turns), so a replica hashes like its original.
pub fn content_hash(item: &Item) -> String {
    if item.kind == Kind::Session {
        return crate::session::Session::from_body(&item.body).map_or_else(
            || crate::session::sha_hex(item.body.as_bytes()),
            |s| s.hash(),
        );
    }
    let mut h = Sha256::new();
    let mut field = |s: &str| {
        h.update((s.len() as u64).to_le_bytes());
        h.update(s.as_bytes());
    };
    field(&normalise(&item.name));
    field(&normalise(&item.description));
    match &item.category {
        Some(c) => {
            field("some");
            field(&normalise(c));
        }
        None => field("none"),
    }
    field(&normalise(&item.body));
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item() -> Item {
        Item {
            kind: Kind::Memory,
            scope: Scope::Global,
            name: "n".into(),
            description: "d".into(),
            category: Some("user".into()),
            body: "line1\nline2".into(),
            source_dir: None,
        }
    }

    #[test]
    fn kind_text_round_trips() {
        for k in [Kind::Memory, Kind::Instruction, Kind::Session, Kind::Skill] {
            assert_eq!(Kind::parse(k.as_str()), Some(k));
        }
        assert_eq!(Kind::parse("other"), None);
    }

    #[test]
    fn stable_hex() {
        let h = content_hash(&item());
        assert_eq!(h, content_hash(&item()));
        assert_eq!(h.len(), 64);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn normalises_line_endings_and_trailing_whitespace() {
        let mut b = item();
        b.body = "line1\r\nline2  \n\n".into();
        assert_eq!(content_hash(&item()), content_hash(&b));
    }

    #[test]
    fn scope_is_not_hashed() {
        let mut b = item();
        b.scope = Scope::Project("/x".into());
        assert_eq!(content_hash(&item()), content_hash(&b));
    }

    #[test]
    fn differs_per_field() {
        let base = content_hash(&item());
        let mut a = item();
        a.name = "n2".into();
        let mut b = item();
        b.description = "d2".into();
        let mut c = item();
        c.category = None;
        let mut d = item();
        d.body = "other".into();
        for x in [a, b, c, d] {
            assert_ne!(base, content_hash(&x));
        }
    }

    #[test]
    fn field_split_does_not_collide() {
        let mut a = item();
        a.name = "ab".into();
        a.description = "c".into();
        let mut b = item();
        b.name = "a".into();
        b.description = "bc".into();
        assert_ne!(content_hash(&a), content_hash(&b));
    }
}
