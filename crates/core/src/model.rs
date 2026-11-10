use std::path::PathBuf;

use sha2::{Digest, Sha256};

use crate::error::Result;

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
}

impl Kind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Kind::Memory => "memory",
            Kind::Instruction => "instruction",
            Kind::Session => "session",
        }
    }

    pub fn parse(s: &str) -> Option<Kind> {
        match s {
            "memory" => Some(Kind::Memory),
            "instruction" => Some(Kind::Instruction),
            "session" => Some(Kind::Session),
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

pub trait Adapter {
    fn id(&self) -> AgentId;
    /// Whether this agent can store an item of `kind` in `scope`. The engine never plans
    /// writes, write-backs or removals for an agent that cannot.
    fn supports(&self, _kind: Kind, _scope: &Scope) -> bool {
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
    fn scan(&self) -> Result<Vec<Observed>>;
    /// Create or update the replica of `key` in this agent. Returns its native_id and path.
    fn write_replica(&self, key: &ItemKey, item: &Item) -> Result<(String, PathBuf)>;
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
        }
    }

    #[test]
    fn kind_text_round_trips() {
        for k in [Kind::Memory, Kind::Instruction, Kind::Session] {
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
