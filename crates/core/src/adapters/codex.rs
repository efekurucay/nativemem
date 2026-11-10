use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};

use crate::compat::Check;
use crate::error::{Error, Result};
use crate::fsx::{self, BackupSession};
use crate::model::{Adapter, AgentId, Item, ItemKey, Kind, Observed, Scope};
use crate::session::{
    is_session_id, set_mtime, Session, SessionOptions, SessionReport, SessionScan,
};
use crate::slug::slugify;

use super::codex_imports;
use super::codex_probes;
use super::codex_rollout;
use super::codex_sessions::CodexSessions;
use super::codex_threads;
#[cfg(test)]
use super::memory_store::{parse_entry, render};
use super::memory_store::{read_opt, MemoryStore};
use super::sections::InstructionFile;
#[cfg(test)]
use super::sections::{BLOCK_BEGIN, BLOCK_END};
use super::skills::{skill_name, SkillReport, SkillStore};

const AGENT: &str = "codex";
const INSTRUCTION_FILE: &str = "AGENTS.md";
const TASK_GROUP: &str = "# Task Group:";

/// Paths inside the Codex home. The replica store layout follows Codex's memory-extension
/// convention and is confined here so it can change in one place.
mod layout {
    use super::*;

    pub fn resources(home: &Path) -> PathBuf {
        home.join("memories/extensions/nativemem/resources")
    }

    pub fn originals(home: &Path) -> PathBuf {
        home.join("memories").join("MEMORY.md")
    }

    #[cfg(test)]
    pub use super::super::memory_store::project_key;

    #[cfg(test)]
    pub fn project_dir(home: &Path, key: &str) -> PathBuf {
        resources(home).join(key)
    }
}

pub struct CodexAdapter {
    home: PathBuf,
    state_dir: PathBuf,
    backups: BackupSession,
    instructions: InstructionFile,
    memory: MemoryStore,
    skills: SkillStore,
    sessions: Option<CodexSessions>,
    claude_home: Option<PathBuf>,
    /// Rollouts written in this run whose `threads` row `finish` refreshes.
    written: RefCell<Vec<PathBuf>>,
    blocked: Vec<Kind>,
}

/// Bump when the written rollout format changes; unmodified replicas are then rewritten.
const SESSION_WRITER_REVISION: u32 = 1;

struct OriginalBlock {
    title: String,
    cwd: String,
    text: String,
}

struct Originals {
    blocks: Vec<OriginalBlock>,
    skipped: Vec<(String, String)>,
}

fn parse_originals(text: &str) -> Originals {
    let mut chunks: Vec<Vec<&str>> = vec![Vec::new()];
    for line in text.split_inclusive('\n') {
        if line.starts_with(TASK_GROUP) {
            chunks.push(Vec::new());
        }
        chunks.last_mut().expect("never empty").push(line);
    }
    let mut out = Originals {
        blocks: Vec::new(),
        skipped: Vec::new(),
    };
    let preamble = chunks.remove(0).concat();
    if !preamble.trim().is_empty() {
        out.skipped.push((
            "memories/MEMORY.md (preamble)".into(),
            "content before the first task group".into(),
        ));
    }
    for chunk in chunks {
        let block = chunk.concat();
        let title = chunk[0][TASK_GROUP.len()..].trim().to_string();
        let cwd = chunk
            .iter()
            .filter_map(|l| l.strip_prefix("applies_to:"))
            .find_map(|v| {
                let rest = &v[v.find("cwd=")? + 4..];
                let path = rest.split(';').next()?.trim();
                (!path.is_empty()).then(|| path.to_string())
            });
        match cwd {
            Some(cwd) => out.blocks.push(OriginalBlock {
                title,
                cwd,
                text: block.trim_end().to_string(),
            }),
            None => out
                .skipped
                .push((title, "task group has no applies_to cwd".into())),
        }
    }
    out
}

impl CodexAdapter {
    pub fn new(codex_home: PathBuf, state_dir: PathBuf, backups: BackupSession) -> Self {
        CodexAdapter {
            instructions: InstructionFile::new(
                Self::agent(),
                codex_home.clone(),
                state_dir.clone(),
                backups.clone(),
                INSTRUCTION_FILE,
            ),
            memory: MemoryStore::new(
                Self::agent(),
                codex_home.clone(),
                state_dir.clone(),
                backups.clone(),
                layout::resources(&codex_home),
                layout::resources(&codex_home),
                codex_home.join(INSTRUCTION_FILE),
            ),
            skills: SkillStore::new(
                Self::agent(),
                codex_home.clone(),
                state_dir.clone(),
                backups.clone(),
            ),
            home: codex_home,
            state_dir,
            backups,
            sessions: None,
            claude_home: None,
            written: RefCell::new(Vec::new()),
            blocked: Vec::new(),
        }
    }

    /// Directories Codex also reads skills from; copies there are never written.
    pub fn with_visible_roots(mut self, roots: Vec<PathBuf>) -> Self {
        self.skills = self.skills.with_visible_roots(roots);
        self
    }

    /// Kinds this adapter neither reads nor writes while its compatibility checks fail.
    pub fn with_blocked_kinds(mut self, kinds: &[Kind]) -> Self {
        self.blocked = kinds.to_vec();
        self
    }

    /// Where Claude Code's transcripts live. Codex's importer ledger records the bytes of the
    /// transcript a thread came from, so a replica of a Claude session needs to read it.
    pub fn with_claude_home(mut self, claude_home: PathBuf) -> Self {
        self.claude_home = Some(claude_home);
        self
    }

    /// Writes the replica rollout below `sessions/`, and for a Claude original the matching
    /// record in Codex's importer ledger so Codex does not import the same session again.
    fn write_session_replica(&self, key: &ItemKey, item: &Item) -> Result<(String, PathBuf)> {
        let session = Session::from_body(&item.body)
            .ok_or_else(|| Error::Parse("session body is not a session".into()))?;
        let thread = codex_rollout::thread_id(&session, key);
        let root = self.home.join("sessions");
        let rel = codex_rollout::relative_path(&session, &thread);
        let path = root.join(&rel);
        let suffix = format!("-{thread}.jsonl");
        let existing = match fs::read_dir(path.parent().unwrap_or(&root)) {
            Ok(entries) => entries
                .flatten()
                .map(|e| e.path())
                .find(|p| p.to_string_lossy().ends_with(&suffix)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(Error::io(&root, e)),
        };
        let path = existing.unwrap_or(path);
        if path.exists() {
            fsx::backup(
                &self.backups,
                &self.state_dir,
                &Self::agent(),
                &self.home,
                &path,
            )?;
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64);
        let text = codex_rollout::render(&session, key, &thread, now);
        fsx::atomic_write(&path, text.as_bytes())?;
        set_mtime(&path, session.updated_ms);
        if key.origin.as_str() == "claude" {
            let source = self
                .claude_home
                .as_ref()
                .map(|h| h.join("projects").join(&key.native_id));
            if let Some(record) =
                source.and_then(|s| codex_imports::record_for(&s, &thread, &session.title))
            {
                codex_imports::upsert(&self.home, &self.state_dir, &self.backups, record)?;
            }
        }
        self.written.borrow_mut().push(path.clone());
        let native_id = path
            .strip_prefix(&root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        Ok((native_id, path))
    }

    /// Reads rollouts as session originals and lets replicas be written; without this call
    /// sessions stay out. Codex lists a thread only when its state database has a row for it;
    /// `finish` maintains that row.
    pub fn with_sessions(mut self, options: SessionOptions) -> Self {
        self.sessions = Some(CodexSessions::new(self.home.clone(), options));
        self
    }

    fn scan_sessions(&self) -> Result<SessionScan> {
        if self.blocked.contains(&Kind::Session) {
            return Ok(SessionScan::default());
        }
        match &self.sessions {
            Some(s) => s.scan(&Self::agent()),
            None => Ok(SessionScan::default()),
        }
    }

    /// Rollouts that cannot be synced and how many are automated.
    pub fn session_report(&self) -> Result<SessionReport> {
        Ok(self.scan_sessions()?.report())
    }

    /// Codex-generated content that cannot be mirrored: `(label, reason)`.
    pub fn skipped(&self) -> Result<Vec<(String, String)>> {
        Ok(self.originals()?.skipped)
    }

    fn agent() -> AgentId {
        AgentId::new(AGENT)
    }

    fn originals(&self) -> Result<Originals> {
        let text = read_opt(&layout::originals(&self.home))?.unwrap_or_default();
        Ok(parse_originals(&text))
    }
}

impl Adapter for CodexAdapter {
    fn id(&self) -> AgentId {
        Self::agent()
    }

    fn version(&self) -> Option<String> {
        codex_probes::version(&self.home)
    }

    fn probes(&self) -> Vec<Check> {
        let mut checks = codex_probes::probes(&self.home);
        checks.push(self.skills.layout_check());
        checks
    }

    fn supports(&self, kind: Kind, scope: &Scope) -> bool {
        if self.blocked.contains(&kind) {
            return false;
        }
        match (kind, scope) {
            (Kind::Session, Scope::Project(_)) => self.sessions.is_some(),
            (Kind::Memory, Scope::Project(_))
            | (Kind::Instruction | Kind::Skill, Scope::Global) => true,
            _ => false,
        }
    }

    fn writer_revision(&self, kind: Kind) -> u32 {
        if kind == Kind::Session {
            SESSION_WRITER_REVISION
        } else {
            0
        }
    }

    /// A rollout without this tool's `nativemem_origin` is Codex's own import.
    fn is_foreign_replica(&self, replica: &Observed) -> bool {
        replica.item.kind == Kind::Session
            && fs::read_to_string(&replica.path)
                .ok()
                .and_then(|t| t.lines().next().map(str::to_string))
                .is_some_and(|first| !first.contains(codex_rollout::ORIGIN_KEY))
    }

    /// Writes the `threads` rows of the replica rollouts; a busy database is left for the next run.
    fn finish(&self) -> Result<Vec<(String, String)>> {
        let written = self.written.take();
        if self.sessions.is_none() || self.blocked.contains(&Kind::Session) {
            return Ok(Vec::new());
        }
        let busy = codex_threads::sync_rows(
            &self.home,
            &self.state_dir,
            &self.backups,
            &written,
            codex_threads::BUSY_TIMEOUT,
        )?;
        Ok(busy
            .map(|reason| ("state database".to_string(), reason))
            .into_iter()
            .collect())
    }

    fn scan(&self) -> Result<Vec<Observed>> {
        let mut out = Vec::new();
        if !self.blocked.contains(&Kind::Memory) {
            out.extend(self.memory.scan()?);
        }
        let originals = if self.blocked.contains(&Kind::Memory) {
            Originals {
                blocks: Vec::new(),
                skipped: Vec::new(),
            }
        } else {
            self.originals()?
        };
        let path = layout::originals(&self.home);
        let mut seen: Vec<String> = Vec::new();
        for b in originals.blocks {
            let slug = slugify(&b.title);
            let base = format!("{}#{slug}", b.cwd);
            let mut native_id = base.clone();
            let mut n = 2;
            while seen.contains(&native_id) {
                native_id = format!("{base}-{n}");
                n += 1;
            }
            seen.push(native_id.clone());
            out.push(Observed {
                agent: Self::agent(),
                native_id,
                path: path.clone(),
                item: Item {
                    kind: Kind::Memory,
                    scope: Scope::Project(PathBuf::from(&b.cwd)),
                    name: slug,
                    description: b.title,
                    category: None,
                    body: b.text,
                    source_dir: None,
                },
                replica_of: None,
            });
        }
        if !self.blocked.contains(&Kind::Instruction) {
            out.extend(self.instructions.scan()?);
        }
        out.extend(self.scan_sessions()?.observed);
        if !self.blocked.contains(&Kind::Skill) {
            out.extend(self.skills.scan()?);
        }
        Ok(out)
    }

    fn visible_copy(&self, native_id: &str) -> Option<String> {
        self.skills.visible_copy(native_id)
    }

    fn skill_report(&self) -> Result<SkillReport> {
        self.skills.report()
    }

    fn write_replica(&self, key: &ItemKey, item: &Item) -> Result<(String, PathBuf)> {
        if item.kind == Kind::Skill {
            return self.skills.write(item);
        }
        if item.kind == Kind::Instruction {
            return self.instructions.write_replica(key, item);
        }
        if item.kind == Kind::Session {
            return self.write_session_replica(key, item);
        }
        self.memory.write_replica(key, item)
    }

    fn create_original(&self, item: &Item) -> Result<(String, PathBuf)> {
        match item.kind {
            Kind::Instruction => self.instructions.create_original(item),
            Kind::Memory => self.memory.create_original(item),
            _ => Err(Error::unsupported(AGENT, "create")),
        }
    }

    fn write_original(&self, native_id: &str, item: &Item) -> Result<()> {
        if skill_name(native_id).is_some() {
            return self.skills.write(item).map(|_| ());
        }
        if is_session_id(native_id) {
            return Err(Error::unsupported(AGENT, "write_original session"));
        }
        if self.instructions.owns(native_id) {
            return self.instructions.write_original(native_id, item);
        }
        self.memory.write_original(native_id, item)
    }

    fn remove(&self, native_id: &str) -> Result<()> {
        if let Some(name) = skill_name(native_id) {
            return self.skills.remove(name);
        }
        if is_session_id(native_id) {
            return Err(Error::unsupported(AGENT, "remove session"));
        }
        if self.instructions.owns(native_id) {
            return self.instructions.remove(native_id);
        }
        self.memory.remove(native_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::content_hash;

    struct Fx {
        _t: tempfile::TempDir,
        home: PathBuf,
        state: PathBuf,
        a: CodexAdapter,
    }

    fn fx() -> Fx {
        let t = tempfile::tempdir().unwrap();
        let home = t.path().join("codex");
        let state = t.path().join("state");
        fs::create_dir_all(&home).unwrap();
        let a = CodexAdapter::new(
            home.clone(),
            state.clone(),
            BackupSession::with_stamp("20260101T000000Z"),
        );
        Fx {
            _t: t,
            home,
            state,
            a,
        }
    }

    fn item(cwd: &str, name: &str, category: Option<&str>, body: &str) -> Item {
        Item {
            kind: Kind::Memory,
            scope: Scope::Project(PathBuf::from(cwd)),
            name: name.into(),
            description: format!("about {name}"),
            category: category.map(String::from),
            body: body.into(),
            source_dir: None,
        }
    }

    fn key(id: &str) -> ItemKey {
        ItemKey {
            origin: AgentId::new("claude"),
            native_id: id.into(),
        }
    }

    fn agents(f: &Fx) -> String {
        fs::read_to_string(f.home.join("AGENTS.md")).unwrap()
    }

    #[test]
    fn replica_round_trips_tricky_bodies() {
        let f = fx();
        let bodies = [
            "# Heading first\n\ntext",
            "Category: not really\nmore",
            "---\nyaml-like\n---\n",
            "\nstarts blank",
            "see [[wiki-link]] and [[other]]\n\n\nspaced",
            "",
            "# Title\n\nCategory: x\n\n- a\n- b",
        ];
        for (i, body) in bodies.iter().enumerate() {
            for cat in [None, Some("feedback")] {
                let id = format!("proj/mem {i}#{}--x>y", cat.is_some());
                let it = item(
                    "/work/app",
                    &format!("item {i} {}", cat.is_some()),
                    cat,
                    body,
                );
                f.a.write_replica(&key(&id), &it).unwrap();
                let found =
                    f.a.scan()
                        .unwrap()
                        .into_iter()
                        .find(|o| o.replica_of == Some(key(&id)))
                        .unwrap();
                assert_eq!(
                    content_hash(&found.item),
                    content_hash(&it),
                    "body {body:?}"
                );
                assert_eq!(found.item.category, it.category);
                assert_eq!(found.item.scope, it.scope);
            }
        }
    }

    #[test]
    fn multiline_name_and_description_round_trip() {
        let f = fx();
        // Newlines in name/description would shift the parse anchors; render collapses
        // them to spaces so the replica still scans back.
        let it = Item {
            kind: Kind::Memory,
            scope: Scope::Project(PathBuf::from("/w/app")),
            name: "with\nnewline".into(),
            description: "first line\nsecond line".into(),
            category: None,
            body: "body".into(),
            source_dir: None,
        };
        f.a.write_replica(&key("k"), &it).unwrap();
        let got =
            f.a.scan()
                .unwrap()
                .into_iter()
                .find(|o| o.replica_of == Some(key("k")))
                .unwrap();
        assert_eq!(got.item.name, "with newline");
        assert_eq!(got.item.description, "first line second line");
    }

    #[test]
    fn marker_id_round_trips_odd_characters() {
        for id in ["a/b c#d--e>f", "100% sure %41", "ünï/çode", "x -->y"] {
            let k = ItemKey {
                origin: AgentId::new("claude"),
                native_id: id.into(),
            };
            let text = render(Some(&k), &item("/p", "n", None, "b"));
            let r = parse_entry(&text).unwrap();
            assert_eq!(r.marker, Some(("claude".to_string(), id.to_string())));
        }
    }

    #[test]
    fn update_in_place_keeps_file() {
        let f = fx();
        let (id1, p1) =
            f.a.write_replica(&key("k"), &item("/w/a", "first", None, "one"))
                .unwrap();
        let (id2, p2) =
            f.a.write_replica(&key("k"), &item("/w/a", "renamed", None, "two"))
                .unwrap();
        assert_eq!((id1, &p1), (id2, &p2));
        let all: Vec<_> = f.a.scan().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].item.name, "renamed");
        assert_eq!(all[0].item.body, "two");
    }

    #[test]
    fn filename_collision_gets_suffix() {
        let f = fx();
        let (a, _) =
            f.a.write_replica(&key("k1"), &item("/w/a", "Same Name", None, "1"))
                .unwrap();
        let (b, _) =
            f.a.write_replica(&key("k2"), &item("/w/a", "same name", None, "2"))
                .unwrap();
        let (c, _) =
            f.a.write_replica(&key("k3"), &item("/w/a", "SAME-name", None, "3"))
                .unwrap();
        assert!(a.ends_with("/same-name.md"));
        assert!(b.ends_with("/same-name-2.md"));
        assert!(c.ends_with("/same-name-3.md"));
    }

    #[test]
    fn memory_slug_does_not_clobber_index() {
        let f = fx();
        let (id, _) =
            f.a.write_replica(&key("k"), &item("/w/a", "memory", None, "x"))
                .unwrap();
        assert!(id.ends_with("/memory-2.md"));
        assert_eq!(f.a.scan().unwrap().len(), 1);
    }

    #[test]
    fn project_key_is_stable_and_distinct() {
        let k = layout::project_key(Path::new("/work/My App"));
        assert_eq!(k, layout::project_key(Path::new("/work/My App")));
        assert!(k.starts_with("my-app-"));
        assert_eq!(k.len(), "my-app-".len() + 8);
        assert_ne!(k, layout::project_key(Path::new("/other/My App")));
    }

    #[test]
    fn replica_layout_scope_and_index() {
        let f = fx();
        f.a.write_replica(&key("b"), &item("/w/app", "Beta", None, "b"))
            .unwrap();
        let (id, path) =
            f.a.write_replica(&key("a"), &item("/w/app", "Alpha", Some("user"), "a"))
                .unwrap();
        let (project, file) = id.split_once('/').unwrap();
        assert_eq!(file, "alpha.md");
        let dir = layout::project_dir(&f.home, project);
        assert_eq!(path, dir.join("alpha.md"));
        let scope: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(dir.join("scope.json")).unwrap()).unwrap();
        assert_eq!(scope["cwd"], "/w/app");
        assert_eq!(
            fs::read_to_string(dir.join("MEMORY.md")).unwrap(),
            "# Memory Index\n\n- [Alpha](alpha.md) — about Alpha\n- [Beta](beta.md) — about Beta\n"
        );
        assert_eq!(
            fs::read_to_string(path).unwrap(),
            "<!-- nativemem origin=claude id=a -->\n# Alpha\n\nabout Alpha\n\nCategory: user\n\na"
        );
    }

    #[test]
    fn global_scope_unsupported() {
        let f = fx();
        let mut it = item("/w", "n", None, "b");
        it.scope = Scope::Global;
        assert!(matches!(
            f.a.write_replica(&key("k"), &it),
            Err(Error::Unsupported { .. })
        ));
    }

    #[test]
    fn routing_block_insert_update_remove_preserves_surroundings() {
        let f = fx();
        let agents_path = f.home.join("AGENTS.md");
        let user = "# My rules\n\nbe nice\n";
        fs::write(&agents_path, user).unwrap();

        let (id_a, _) =
            f.a.write_replica(&key("a"), &item("/w/a", "A", None, "a"))
                .unwrap();
        let first = agents(&f);
        assert!(first.starts_with(&format!("{user}\n{BLOCK_BEGIN}\n")));
        assert!(first.contains("## Project memory (managed by nativemem)"));
        assert!(first.contains("- /w/a → "));
        assert!(first.ends_with(&format!("{BLOCK_END}\n")));
        assert!(f
            .state
            .join("backups/20260101T000000Z/codex/AGENTS.md")
            .exists());

        let (id_b, _) =
            f.a.write_replica(&key("b"), &item("/w/b", "B", None, "b"))
                .unwrap();
        let second = agents(&f);
        assert!(second.starts_with(user));
        assert!(second.contains("- /w/a → ") && second.contains("- /w/b → "));
        assert!(second.find("- /w/a").unwrap() < second.find("- /w/b").unwrap());
        assert_eq!(second.matches(BLOCK_BEGIN).count(), 1);

        f.a.remove(&id_a).unwrap();
        let third = agents(&f);
        assert!(!third.contains("/w/a") && third.contains("/w/b"));
        f.a.remove(&id_b).unwrap();
        assert_eq!(agents(&f), user);
    }

    #[test]
    fn routing_block_keeps_text_without_trailing_newline_and_after_block() {
        let f = fx();
        let agents_path = f.home.join("AGENTS.md");
        fs::write(&agents_path, "top").unwrap();
        f.a.write_replica(&key("a"), &item("/w/a", "A", None, "a"))
            .unwrap();
        assert!(agents(&f).starts_with("top\n\n"));

        let text = agents(&f);
        let tail = format!("{text}tail without newline");
        fs::write(&agents_path, &tail).unwrap();
        f.a.write_replica(&key("b"), &item("/w/b", "B", None, "b"))
            .unwrap();
        let updated = agents(&f);
        assert!(updated.starts_with("top\n\n"));
        assert!(updated.ends_with(&format!("{BLOCK_END}\ntail without newline")));
        assert!(updated.contains("/w/b"));
    }

    #[test]
    fn routing_block_created_when_agents_md_absent() {
        let f = fx();
        assert!(!f.home.join("AGENTS.md").exists());
        let (id, _) =
            f.a.write_replica(&key("a"), &item("/w/a", "A", None, "a"))
                .unwrap();
        let text = agents(&f);
        assert!(text.starts_with(BLOCK_BEGIN) && text.ends_with(&format!("{BLOCK_END}\n")));
        assert!(!f.state.join("backups").exists());
        f.a.remove(&id).unwrap();
        assert_eq!(agents(&f), "");
    }

    #[test]
    fn unchanged_routing_is_not_rewritten() {
        let f = fx();
        let it = item("/w/a", "A", None, "a");
        f.a.write_replica(&key("a"), &it).unwrap();
        let p = f.home.join("AGENTS.md");
        let before = fs::metadata(&p).unwrap().modified().unwrap();
        fs::remove_dir_all(f.state.join("backups")).ok();
        std::thread::sleep(std::time::Duration::from_millis(20));
        f.a.write_replica(&key("a"), &it).unwrap();
        assert_eq!(fs::metadata(&p).unwrap().modified().unwrap(), before);
        assert!(!f.state.join("backups").exists());
    }

    #[test]
    fn remove_backs_up_and_cleans_empty_project() {
        let f = fx();
        let (id1, p1) =
            f.a.write_replica(&key("a"), &item("/w/a", "One", None, "1"))
                .unwrap();
        let (id2, p2) =
            f.a.write_replica(&key("b"), &item("/w/a", "Two", None, "2"))
                .unwrap();
        let dir = p1.parent().unwrap().to_path_buf();

        f.a.remove(&id1).unwrap();
        assert!(!p1.exists());
        assert!(f.state.join("backups/20260101T000000Z/codex").exists());
        let index = fs::read_to_string(dir.join("MEMORY.md")).unwrap();
        assert!(!index.contains("One") && index.contains("Two"));

        f.a.remove(&id2).unwrap();
        assert!(!p2.exists() && !dir.exists());
        assert!(f.a.scan().unwrap().is_empty());
    }

    #[test]
    fn remove_rejects_traversal() {
        let f = fx();
        assert!(matches!(f.a.remove("../x/y.md"), Err(Error::Parse(_))));
        assert!(matches!(f.a.remove("nodir"), Err(Error::Parse(_))));
    }

    fn write_originals(f: &Fx, text: &str) {
        let p = f.home.join("memories/MEMORY.md");
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    }

    #[test]
    fn task_groups_parse_with_unscoped_and_duplicates() {
        let f = fx();
        write_originals(
            &f,
            "Intro text\n\n\
             # Task Group: Fix Login Bug\napplies_to: cwd=/w/app; reuse_rule=x\nnotes\n\n\
             # Task Group: No scope here\nnothing\n\n\
             # Task Group: Fix login bug\n- applies_to: nope\napplies_to: cwd=/w/app\nmore\n\n\
             # Task Group: Elsewhere\napplies_to: cwd=/w/other \n",
        );
        let obs = f.a.scan().unwrap();
        let ids: Vec<_> = obs.iter().map(|o| o.native_id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "/w/app#fix-login-bug",
                "/w/app#fix-login-bug-2",
                "/w/other#elsewhere"
            ]
        );
        let first = &obs[0];
        assert_eq!(first.item.name, "fix-login-bug");
        assert_eq!(first.item.description, "Fix Login Bug");
        assert_eq!(first.item.scope, Scope::Project("/w/app".into()));
        assert_eq!(first.item.category, None);
        assert!(first.item.body.starts_with("# Task Group: Fix Login Bug"));
        assert!(first.item.body.ends_with("notes"));
        assert!(first.replica_of.is_none());
        assert_eq!(first.path, f.home.join("memories/MEMORY.md"));

        let skipped = f.a.skipped().unwrap();
        assert_eq!(skipped.len(), 2);
        assert!(skipped[0].0.contains("preamble"));
        assert_eq!(skipped[1].0, "No scope here");
    }

    #[test]
    fn missing_originals_file_is_empty() {
        let f = fx();
        assert!(f.a.scan().unwrap().is_empty());
        assert!(f.a.skipped().unwrap().is_empty());
    }

    #[test]
    fn originals_are_read_only() {
        let f = fx();
        assert!(matches!(
            f.a.write_original("/w#x", &item("/w", "x", None, "b")),
            Err(Error::Unsupported { .. })
        ));
        assert!(matches!(
            f.a.remove("/w/app#x"),
            Err(Error::Unsupported { .. })
        ));
    }

    #[test]
    fn created_memory_lives_in_the_resources_store_and_is_editable() {
        let f = fx();
        let (id, path) =
            f.a.create_original(&item("/w/app", "Alpha", Some("user"), "b"))
                .unwrap();
        let dir = layout::project_dir(&f.home, id.split_once('/').unwrap().0);
        assert_eq!(path, dir.join("alpha.md"));
        assert!(agents(&f).contains("- /w/app → "));
        let found = f.a.scan().unwrap();
        assert_eq!(found.len(), 1);
        assert!(found[0].replica_of.is_none());
        f.a.write_original(&id, &item("/w/app", "Alpha", None, "edited"))
            .unwrap();
        assert_eq!(f.a.scan().unwrap()[0].item.body, "edited");
        f.a.remove(&id).unwrap();
        assert!(f.a.scan().unwrap().is_empty());
    }

    #[test]
    fn only_memory_and_instructions_can_be_created() {
        let f = fx();
        let session = Item {
            kind: Kind::Session,
            ..item("/w", "s", None, "b")
        };
        assert!(matches!(
            f.a.create_original(&session),
            Err(Error::Unsupported { .. })
        ));
    }

    fn instruction(heading: &str, body: &str) -> Item {
        Item {
            kind: Kind::Instruction,
            scope: Scope::Global,
            name: slugify(heading),
            description: heading.into(),
            category: None,
            body: body.into(),
            source_dir: None,
        }
    }

    #[test]
    fn instruction_originals_skip_the_routing_block_and_the_region() {
        let f = fx();
        fs::write(
            f.home.join("AGENTS.md"),
            format!(
                "# Mine\n\n## Style\nbe terse\n\n{BLOCK_BEGIN}\n## Project memory (managed by nativemem)\n- /x → /y\n{BLOCK_END}\n"
            ),
        )
        .unwrap();
        f.a.write_replica(&key("CLAUDE.md#a"), &instruction("A", "a body"))
            .unwrap();
        let items = f.a.scan().unwrap();
        let ids: Vec<_> = items.iter().map(|o| o.native_id.as_str()).collect();
        assert_eq!(ids, ["AGENTS.md#style", "AGENTS.md#claude:CLAUDE.md#a"]);
        assert!(items[0].replica_of.is_none());
        assert!(items[1].replica_of.is_some());
    }

    #[test]
    fn routing_block_goes_before_an_existing_region_and_leaves_it_alone() {
        let f = fx();
        f.a.write_replica(&key("CLAUDE.md#a"), &instruction("A", "a body"))
            .unwrap();
        let region_only = agents(&f);
        f.a.write_replica(&key("m"), &item("/w/a", "M", None, "m"))
            .unwrap();
        let both = agents(&f);
        let block = both.find(BLOCK_BEGIN).unwrap();
        let region = both.find("<!-- nativemem:instructions:begin -->").unwrap();
        assert!(block < region);
        assert!(both.ends_with(&region_only));
        let (id, _) =
            f.a.write_replica(&key("m"), &item("/w/a", "M", None, "m2"))
                .unwrap();
        f.a.remove(&id).unwrap();
        assert_eq!(agents(&f), region_only);
    }

    #[test]
    fn instruction_originals_are_editable_but_memory_originals_are_not() {
        let f = fx();
        fs::write(f.home.join("AGENTS.md"), "## Style\nold\n\n## Keep\nk\n").unwrap();
        f.a.write_original("AGENTS.md#style", &instruction("Style", "new"))
            .unwrap();
        assert_eq!(agents(&f), "## Style\nnew\n\n## Keep\nk\n");
        f.a.remove("AGENTS.md#style").unwrap();
        assert_eq!(agents(&f), "## Keep\nk\n");
        assert!(matches!(
            f.a.write_original("/w/app#fix-login", &item("/w/app", "n", None, "b")),
            Err(Error::Unsupported { .. })
        ));
        assert!(matches!(
            f.a.remove("/w/app#fix-login"),
            Err(Error::Unsupported { .. })
        ));
    }
}
