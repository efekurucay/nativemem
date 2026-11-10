//! Agents that only read a global `AGENTS.md` (OpenCode, Pi): instruction sections, and project
//! memory replicas stored below `<dir>/nativemem/memory/` and announced by a routing block in
//! that file. They never originate memory.

use std::path::PathBuf;

use crate::compat::Check;
use crate::error::{Error, Result};
use crate::fsx::BackupSession;
use crate::model::{Adapter, AgentId, Item, ItemKey, Kind, Observed, Scope};
use crate::session::{is_session_id, SessionOptions, SessionReport, SessionScan};

use super::memory_store::MemoryStore;
use super::opencode_probes;
use super::opencode_sessions::{OpencodeSessions, SESSION_WRITER_REVISION};
use super::sections::InstructionFile;
use super::skills::{skill_name, SkillReport, SkillStore};

const INSTRUCTION_FILE: &str = "AGENTS.md";
const MEMORY_DIR: &str = "nativemem/memory";

pub struct AgentsFileAdapter {
    agent: AgentId,
    home: PathBuf,
    search_path: Option<std::ffi::OsString>,
    state_dir: PathBuf,
    backups: BackupSession,
    blocked: Vec<Kind>,
    instructions: InstructionFile,
    memory: MemoryStore,
    skills: SkillStore,
    sessions: Option<OpencodeSessions>,
    session_writes: bool,
}

impl AgentsFileAdapter {
    /// `dir` is the agent's home directory; the file is `<dir>/AGENTS.md`.
    pub fn new(id: &str, dir: PathBuf, state_dir: PathBuf, backups: BackupSession) -> Self {
        let agent = AgentId::new(id);
        AgentsFileAdapter {
            home: dir.clone(),
            search_path: crate::probe::process_path(),
            blocked: Vec::new(),
            state_dir: state_dir.clone(),
            backups: backups.clone(),
            sessions: None,
            session_writes: false,
            memory: MemoryStore::new(
                agent.clone(),
                dir.clone(),
                state_dir.clone(),
                backups.clone(),
                dir.join(MEMORY_DIR),
                dir.clone(),
                dir.join(INSTRUCTION_FILE),
            ),
            skills: SkillStore::new(
                agent.clone(),
                dir.clone(),
                state_dir.clone(),
                backups.clone(),
            ),
            instructions: InstructionFile::new(
                agent.clone(),
                dir,
                state_dir,
                backups,
                INSTRUCTION_FILE,
            ),
            agent,
        }
    }

    /// Directories the agent also reads skills from; copies there are never written.
    pub fn with_visible_roots(mut self, roots: Vec<PathBuf>) -> Self {
        self.skills = self.skills.with_visible_roots(roots);
        self
    }

    /// The probe of the skills directory layout.
    pub fn skills_check(&self) -> Check {
        self.skills.layout_check()
    }

    /// Kinds this adapter neither reads nor writes while its compatibility checks fail.
    pub fn with_blocked_kinds(mut self, kinds: &[Kind]) -> Self {
        self.blocked = kinds.to_vec();
        self
    }

    /// OpenCode's session database. `options` is `None` while session sync is off, so nothing is
    /// read; replicas are written only while session writes are enabled.
    pub fn with_opencode_sessions(mut self, db: PathBuf, options: Option<SessionOptions>) -> Self {
        self.sessions = Some(OpencodeSessions::new(
            db,
            self.state_dir.clone(),
            self.backups.clone(),
            options,
        ));
        self
    }

    /// How long a locked session database is waited for before a run skips it.
    pub fn with_session_busy_timeout(mut self, wait: std::time::Duration) -> Self {
        self.sessions = self.sessions.map(|s| s.with_busy_timeout(wait));
        self
    }

    /// Whether session replicas are written into OpenCode; off unless the caller opts in.
    pub fn with_session_writes(mut self, enabled: bool) -> Self {
        self.session_writes = enabled;
        self
    }

    pub fn session_writes_enabled(&self) -> bool {
        self.session_writes
    }

    /// Whether the agent has a session database to read from.
    pub fn has_session_store(&self) -> bool {
        self.sessions.is_some()
    }

    fn scan_sessions(&self) -> Result<SessionScan> {
        match &self.sessions {
            Some(s) if !self.blocked.contains(&Kind::Session) => s.scan(&self.agent),
            _ => Ok(SessionScan::default()),
        }
    }

    /// Sessions that cannot be synced and how many are automated.
    pub fn session_report(&self) -> Result<SessionReport> {
        Ok(self.scan_sessions()?.report())
    }

    /// Where the version probe looks for the agent's executable [default: `PATH`].
    pub fn with_search_path(mut self, path: std::ffi::OsString) -> Self {
        self.search_path = Some(path);
        self
    }

    fn unsupported(&self, operation: &str) -> Error {
        Error::unsupported(self.agent.as_str(), operation)
    }
}

impl Adapter for AgentsFileAdapter {
    fn id(&self) -> AgentId {
        self.agent.clone()
    }

    fn version(&self) -> Option<String> {
        match self.agent.as_str() {
            "opencode" => opencode_probes::version(self.search_path.as_deref()),
            _ => None,
        }
    }

    fn probes(&self) -> Vec<Check> {
        let mut checks = match self.agent.as_str() {
            "opencode" => opencode_probes::probes(&self.home),
            _ => Vec::new(),
        };
        if let Some(s) = &self.sessions {
            checks.extend(opencode_probes::session_probes(
                s.database(),
                self.version().as_deref(),
            ));
        }
        checks.push(self.skills_check());
        checks
    }

    fn supports(&self, kind: Kind, scope: &Scope) -> bool {
        !self.blocked.contains(&kind)
            && match (kind, scope) {
                (Kind::Memory, Scope::Project(_))
                | (Kind::Instruction | Kind::Skill, Scope::Global) => true,
                (Kind::Session, Scope::Project(_)) => {
                    self.session_writes && self.sessions.is_some()
                }
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

    fn scan(&self) -> Result<Vec<Observed>> {
        let mut out = Vec::new();
        if !self.blocked.contains(&Kind::Instruction) {
            out.extend(self.instructions.scan()?);
        }
        if !self.blocked.contains(&Kind::Memory) {
            out.extend(self.memory.scan()?);
        }
        if !self.blocked.contains(&Kind::Skill) {
            out.extend(self.skills.scan()?);
        }
        out.extend(self.scan_sessions()?.observed);
        Ok(out)
    }

    fn visible_copy(&self, native_id: &str) -> Option<String> {
        self.skills.visible_copy(native_id)
    }

    fn skill_report(&self) -> Result<SkillReport> {
        self.skills.report()
    }

    fn write_replica(&self, key: &ItemKey, item: &Item) -> Result<(String, PathBuf)> {
        match item.kind {
            Kind::Instruction => self.instructions.write_replica(key, item),
            Kind::Memory => self.memory.write_replica(key, item),
            Kind::Skill => self.skills.write(item),
            Kind::Session => match &self.sessions {
                Some(sessions) => sessions.write(key, item, self.version().as_deref()),
                None => Err(self.unsupported("write_replica session")),
            },
        }
    }

    fn write_original(&self, native_id: &str, item: &Item) -> Result<()> {
        if is_session_id(native_id) {
            return Err(self.unsupported("write_original session"));
        }
        if skill_name(native_id).is_some() {
            return self.skills.write(item).map(|_| ());
        }
        if !self.instructions.owns(native_id) {
            return Err(self.unsupported("write_original"));
        }
        self.instructions.write_original(native_id, item)
    }

    fn remove(&self, native_id: &str) -> Result<()> {
        if is_session_id(native_id) {
            return Err(self.unsupported("remove session"));
        }
        if let Some(name) = skill_name(native_id) {
            return self.skills.remove(name);
        }
        if self.instructions.owns(native_id) {
            return self.instructions.remove(native_id);
        }
        self.memory.remove(native_id)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::slug::slugify;

    struct Fx {
        _t: tempfile::TempDir,
        dir: PathBuf,
        state: PathBuf,
        a: AgentsFileAdapter,
    }

    fn fx() -> Fx {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("pi");
        let state = t.path().join("state");
        fs::create_dir_all(&dir).unwrap();
        let a = AgentsFileAdapter::new(
            "pi",
            dir.clone(),
            state.clone(),
            BackupSession::with_stamp("T"),
        );
        Fx {
            _t: t,
            dir,
            state,
            a,
        }
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

    fn key(id: &str) -> ItemKey {
        ItemKey {
            origin: AgentId::new("claude"),
            native_id: id.into(),
        }
    }

    fn text(f: &Fx) -> String {
        fs::read_to_string(f.dir.join("AGENTS.md")).unwrap()
    }

    #[test]
    fn supports_global_instructions_and_project_memory() {
        let f = fx();
        assert!(f.a.supports(Kind::Instruction, &Scope::Global));
        assert!(f.a.supports(Kind::Memory, &Scope::Project("/w".into())));
        assert!(!f
            .a
            .supports(Kind::Instruction, &Scope::Project("/w".into())));
        assert!(!f.a.supports(Kind::Memory, &Scope::Global));
        assert!(!f.a.supports(Kind::Session, &Scope::Project("/w".into())));
    }

    #[test]
    fn missing_file_has_no_items_and_first_replica_creates_it() {
        let f = fx();
        assert!(f.a.scan().unwrap().is_empty());
        let (id, path) =
            f.a.write_replica(&key("CLAUDE.md#a"), &instruction("A", "a body"))
                .unwrap();
        assert_eq!(id, "AGENTS.md#claude:CLAUDE.md#a");
        assert_eq!(path, f.dir.join("AGENTS.md"));
        assert!(text(&f).starts_with("<!-- nativemem:instructions:begin -->\n"));
        assert!(!f.state.join("backups").exists());
        let found = f.a.scan().unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].replica_of, Some(key("CLAUDE.md#a")));
    }

    #[test]
    fn originals_stay_outside_the_region_and_are_editable_and_removable() {
        let f = fx();
        fs::write(
            f.dir.join("AGENTS.md"),
            "# Mine\n\n## Style\nold\n\n## Keep\nk\n",
        )
        .unwrap();
        f.a.write_replica(&key("CLAUDE.md#a"), &instruction("A", "a body"))
            .unwrap();
        let ids: Vec<_> =
            f.a.scan()
                .unwrap()
                .into_iter()
                .map(|o| o.native_id)
                .collect();
        assert_eq!(
            ids,
            [
                "AGENTS.md#style",
                "AGENTS.md#keep",
                "AGENTS.md#claude:CLAUDE.md#a"
            ]
        );
        f.a.write_original("AGENTS.md#style", &instruction("Style", "new"))
            .unwrap();
        assert!(text(&f).starts_with("# Mine\n\n## Style\nnew\n\n## Keep\nk\n"));
        f.a.remove("AGENTS.md#style").unwrap();
        f.a.remove("AGENTS.md#claude:CLAUDE.md#a").unwrap();
        assert_eq!(text(&f), "# Mine\n\n## Keep\nk\n");
        assert!(f.state.join("backups/T/pi/AGENTS.md").is_file());
    }

    fn memory(cwd: &str, name: &str) -> Item {
        Item {
            kind: Kind::Memory,
            scope: Scope::Project(cwd.into()),
            name: name.into(),
            description: format!("about {name}"),
            category: Some("user".into()),
            body: "b".into(),
            source_dir: None,
        }
    }

    #[test]
    fn memory_replicas_live_below_the_base_with_a_routing_block() {
        let f = fx();
        fs::write(f.dir.join("AGENTS.md"), "# Mine\n\n## Style\nkeep\n").unwrap();
        let (id, path) =
            f.a.write_replica(&key("m"), &memory("/w/app", "Alpha"))
                .unwrap();
        let (project, file) = id.split_once('/').unwrap();
        assert_eq!(file, "alpha.md");
        let dir = f.dir.join("nativemem/memory").join(project);
        assert_eq!(path, dir.join("alpha.md"));
        assert!(dir.join("MEMORY.md").is_file() && dir.join("scope.json").is_file());
        let agents = text(&f);
        assert!(agents.starts_with("# Mine\n\n## Style\nkeep\n\n<!-- nativemem:begin -->\n"));
        assert!(agents.contains(&format!("- /w/app → {}\n", dir.join("MEMORY.md").display())));

        let found = f.a.scan().unwrap();
        assert_eq!(found.len(), 2);
        assert_eq!(found[1].replica_of, Some(key("m")));
        assert_eq!(found[1].item.kind, Kind::Memory);

        f.a.remove(&id).unwrap();
        assert_eq!(text(&f), "# Mine\n\n## Style\nkeep\n");
        assert!(!f.dir.join("nativemem").exists());
        assert!(f.state.join("backups/T/pi/nativemem/memory").is_dir());
    }

    #[test]
    fn memory_originals_and_sessions_are_unsupported() {
        let f = fx();
        let m = memory("/w", "n");
        assert!(matches!(
            f.a.write_original("/w#x", &m),
            Err(Error::Unsupported { .. })
        ));
        assert!(matches!(f.a.remove("/w#x"), Err(Error::Unsupported { .. })));
        let session = Item {
            kind: Kind::Session,
            ..m
        };
        assert!(matches!(
            f.a.write_replica(&key("k"), &session),
            Err(Error::Unsupported { .. })
        ));
        assert!(!f.dir.join("AGENTS.md").exists());
    }
}
