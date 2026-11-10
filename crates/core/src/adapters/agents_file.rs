//! Agents that only read a global `AGENTS.md` (OpenCode, Pi): instruction sections, and project
//! memory replicas stored below `<dir>/nativemem/memory/` and announced by a routing block in
//! that file. They never originate memory.

use std::path::PathBuf;

use crate::error::{Error, Result};
use crate::fsx::BackupSession;
use crate::model::{Adapter, AgentId, Item, ItemKey, Kind, Observed, Scope};

use super::memory_store::MemoryStore;
use super::sections::InstructionFile;

const INSTRUCTION_FILE: &str = "AGENTS.md";
const MEMORY_DIR: &str = "nativemem/memory";

pub struct AgentsFileAdapter {
    agent: AgentId,
    instructions: InstructionFile,
    memory: MemoryStore,
}

impl AgentsFileAdapter {
    /// `dir` is the agent's home directory; the file is `<dir>/AGENTS.md`.
    pub fn new(id: &str, dir: PathBuf, state_dir: PathBuf, backups: BackupSession) -> Self {
        let agent = AgentId::new(id);
        AgentsFileAdapter {
            memory: MemoryStore::new(
                agent.clone(),
                dir.clone(),
                state_dir.clone(),
                backups.clone(),
                dir.join(MEMORY_DIR),
                dir.clone(),
                dir.join(INSTRUCTION_FILE),
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

    fn unsupported(&self, operation: &str) -> Error {
        Error::unsupported(self.agent.as_str(), operation)
    }
}

impl Adapter for AgentsFileAdapter {
    fn id(&self) -> AgentId {
        self.agent.clone()
    }

    fn supports(&self, kind: Kind, scope: &Scope) -> bool {
        matches!(
            (kind, scope),
            (Kind::Memory, Scope::Project(_)) | (Kind::Instruction, Scope::Global)
        )
    }

    fn scan(&self) -> Result<Vec<Observed>> {
        let mut out = self.instructions.scan()?;
        out.extend(self.memory.scan()?);
        Ok(out)
    }

    fn write_replica(&self, key: &ItemKey, item: &Item) -> Result<(String, PathBuf)> {
        match item.kind {
            Kind::Instruction => self.instructions.write_replica(key, item),
            Kind::Memory => self.memory.write_replica(key, item),
            Kind::Session => Err(self.unsupported("write_replica session")),
        }
    }

    fn write_original(&self, native_id: &str, item: &Item) -> Result<()> {
        if !self.instructions.owns(native_id) {
            return Err(self.unsupported("write_original"));
        }
        self.instructions.write_original(native_id, item)
    }

    fn remove(&self, native_id: &str) -> Result<()> {
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
