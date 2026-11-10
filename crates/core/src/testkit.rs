//! Builders for fake agent homes. Callers pass a temp directory; nothing here touches real homes.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use crate::adapters::pi::encode_cwd;
use crate::session::{format_iso_ms, set_mtime};

/// `(role, text)` pairs; role is `user` or `assistant`.
pub type Turns<'a> = &'a [(&'a str, &'a str)];

/// Sets a file's modification time to `days` days ago.
pub fn age_days(path: &Path, days: i64) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    set_mtime(path, now - days * 86_400_000);
}

const BASE_MS: i64 = 1_794_304_800_000;

fn turn_ts(i: usize) -> String {
    format_iso_ms(BASE_MS + i as i64 * 1000)
}

fn jsonl(lines: &[Value]) -> String {
    lines.iter().map(|l| format!("{l}\n")).collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrontmatterVariant {
    Nested,
    Flat,
}

pub struct ClaudeHome {
    pub root: PathBuf,
}

impl ClaudeHome {
    pub fn new(dir: &Path) -> Self {
        ClaudeHome {
            root: dir.to_path_buf(),
        }
    }

    /// Creates `projects/<encoded>/` with a jsonl carrying `cwd` and an empty `memory/`.
    /// Returns the project directory name.
    pub fn add_project(&self, cwd: &Path) -> String {
        let name: String = cwd
            .to_string_lossy()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        let dir = self.root.join("projects").join(&name);
        fs::create_dir_all(dir.join("memory")).unwrap();
        let cwd_json = cwd
            .to_string_lossy()
            .replace('\\', "\\\\")
            .replace('"', "\\\"");
        fs::write(
            dir.join("session.jsonl"),
            format!("{{\"cwd\":\"{cwd_json}\"}}\n"),
        )
        .unwrap();
        name
    }

    /// Writes `projects/<encoded cwd>/<id>.jsonl` with the given turns. Returns the file path.
    pub fn add_session(&self, cwd: &Path, id: &str, title: Option<&str>, turns: Turns) -> PathBuf {
        let cwd_text = cwd.to_string_lossy();
        let dir = self.add_project(cwd);
        let mut lines = Vec::new();
        if let Some(t) = title {
            lines.push(json!({"type": "custom-title", "customTitle": t, "sessionId": id}));
        }
        for (i, (role, text)) in turns.iter().enumerate() {
            let content = if *role == "user" {
                json!(text)
            } else {
                json!([{"type": "text", "text": text}])
            };
            lines.push(json!({
                "type": role, "uuid": format!("u{i}"), "timestamp": turn_ts(i), "cwd": cwd_text,
                "sessionId": id, "message": {"role": role, "content": content},
            }));
        }
        let path = self
            .root
            .join("projects")
            .join(dir)
            .join(format!("{id}.jsonl"));
        fs::write(&path, jsonl(&lines)).unwrap();
        path
    }

    #[allow(clippy::too_many_arguments)]
    /// Writes `memory/<filename>` and appends its line to `MEMORY.md`. Returns the file path.
    pub fn add_memory(
        &self,
        project_dir: &str,
        filename: &str,
        variant: FrontmatterVariant,
        name: &str,
        description: &str,
        category: &str,
        body: &str,
    ) -> PathBuf {
        let mem = self.root.join("projects").join(project_dir).join("memory");
        fs::create_dir_all(&mem).unwrap();
        let fm = match variant {
            FrontmatterVariant::Nested => format!(
                "name: {name}\ndescription: {description}\nmetadata:\n  type: {category}\n  node_type: memory\n  originSessionId: s-1\n  modified: 2026-01-01\n"
            ),
            FrontmatterVariant::Flat => format!(
                "name: {name}\ndescription: {description}\ntype: {category}\noriginSessionId: s-1\n"
            ),
        };
        let path = mem.join(filename);
        fs::write(&path, format!("---\n{fm}---\n\n{body}\n")).unwrap();

        let index = mem.join("MEMORY.md");
        if !index.exists() {
            fs::write(&index, "# Memory Index\n\n").unwrap();
        }
        let mut f = fs::OpenOptions::new().append(true).open(&index).unwrap();
        writeln!(f, "- [{name}]({filename}) — {description}").unwrap();
        path
    }
}

pub struct CodexHome {
    pub root: PathBuf,
}

impl CodexHome {
    pub fn new(dir: &Path) -> Self {
        CodexHome {
            root: dir.to_path_buf(),
        }
    }

    pub fn write_agents_md(&self, text: &str) {
        fs::create_dir_all(&self.root).unwrap();
        fs::write(self.root.join("AGENTS.md"), text).unwrap();
    }

    pub fn write_memory_md(&self, text: &str) {
        let dir = self.root.join("memories");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("MEMORY.md"), text).unwrap();
    }

    /// Writes `sessions/2026/11/10/rollout-<ts>-<id>.jsonl` (`id` is a 36-character thread id).
    /// With `imported`, the file ends with Codex's import marker event.
    pub fn add_rollout(&self, id: &str, cwd: &Path, turns: Turns, imported: bool) -> PathBuf {
        let mut lines = vec![json!({
            "timestamp": turn_ts(0), "type": "session_meta",
            "payload": {"id": id, "cwd": cwd.to_string_lossy(), "timestamp": turn_ts(0), "source": "cli"},
        })];
        lines.push(json!({"timestamp": turn_ts(0), "type": "response_item", "payload": {
            "type": "message", "role": "user",
            "content": [{"type": "input_text", "text": "<environment_context>injected</environment_context>"}]}}));
        for (i, (role, text)) in turns.iter().enumerate() {
            let item = if *role == "user" {
                json!({"type": "UserMessage", "content": [{"type": "text", "text": text}]})
            } else {
                json!({"type": "AgentMessage", "content": [{"type": "Text", "text": text}]})
            };
            lines.push(json!({"timestamp": turn_ts(i + 1), "type": "event_msg",
                "payload": {"type": "item_completed", "item": item}}));
        }
        if imported {
            lines.push(json!({"timestamp": turn_ts(turns.len() + 1), "type": "event_msg", "payload": {
                "type": "item_completed",
                "item": {"type": "AgentMessage", "content": [{"type": "Text", "text": "<EXTERNAL SESSION IMPORTED>"}]}}}));
        }
        let dir = self.root.join("sessions/2026/11/10");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("rollout-2026-11-10T10-00-00-{id}.jsonl"));
        fs::write(&path, jsonl(&lines)).unwrap();
        path
    }

    /// Writes `state_<number>.sqlite` with the structure of Codex's `threads` table (tables it
    /// references, indexes and triggers, no rows), plus the `extra` column definitions
    /// (`name TYPE [NOT NULL DEFAULT x]`) added to it.
    pub fn write_state_db(&self, number: u32, extra: &[&str]) {
        fs::create_dir_all(&self.root).unwrap();
        let db = self.root.join(format!("state_{number}.sqlite"));
        let conn = rusqlite::Connection::open(db).unwrap();
        conn.execute_batch(include_str!("testkit_codex_state.sql"))
            .unwrap();
        for column in extra {
            conn.execute(&format!("ALTER TABLE threads ADD COLUMN {column}"), [])
                .unwrap();
        }
    }

    /// The ids in the `threads` table of `state_<number>.sqlite`, sorted.
    pub fn thread_ids(&self, number: u32) -> Vec<String> {
        let conn =
            rusqlite::Connection::open(self.root.join(format!("state_{number}.sqlite"))).unwrap();
        let mut stmt = conn.prepare("SELECT id FROM threads ORDER BY id").unwrap();
        let ids = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .flatten()
            .collect();
        ids
    }

    /// Inserts a row for a thread Codex itself created.
    pub fn insert_thread(&self, number: u32, id: &str, cwd: &Path) {
        let conn =
            rusqlite::Connection::open(self.root.join(format!("state_{number}.sqlite"))).unwrap();
        conn.execute(
            "INSERT INTO threads (id, rollout_path, created_at, updated_at, source, \
             model_provider, cwd, title, sandbox_policy, approval_mode, preview) \
             VALUES (?1, 'p', 1, 2, 'vscode', 'openai', ?2, 't', 'x', 'y', 'q')",
            rusqlite::params![id, cwd.to_string_lossy()],
        )
        .unwrap();
    }

    /// Writes `version.json` the way Codex's update check does.
    pub fn write_version(&self, version: &str) {
        fs::create_dir_all(&self.root).unwrap();
        fs::write(
            self.root.join("version.json"),
            json!({"latest_version": version}).to_string(),
        )
        .unwrap();
    }

    /// Writes Codex's importer ledger listing `thread_ids` as imported.
    pub fn write_import_ledger(&self, thread_ids: &[&str]) {
        let records: Vec<Value> = thread_ids
            .iter()
            .map(|id| {
                json!({"source_path": "/x/s.jsonl", "content_sha256": "00", "imported_thread_id": id,
                       "imported_at": 1, "source_modified_at": 2, "connector_names": [], "title": "t"})
            })
            .collect();
        fs::create_dir_all(&self.root).unwrap();
        fs::write(
            self.root.join("external_agent_session_imports.json"),
            json!({"records": records, "detected_connector_records": []}).to_string(),
        )
        .unwrap();
    }
}

pub struct PiHome {
    pub root: PathBuf,
}

impl PiHome {
    pub fn new(dir: &Path) -> Self {
        PiHome {
            root: dir.to_path_buf(),
        }
    }

    /// Writes `sessions/--<cwd>--/<ts>_<id>.jsonl` with a linear entry chain.
    pub fn add_session(&self, cwd: &Path, id: &str, title: Option<&str>, turns: Turns) -> PathBuf {
        let mut lines = vec![json!({
            "type": "session", "version": 3, "id": id, "timestamp": turn_ts(0),
            "cwd": cwd.to_string_lossy(),
        })];
        let mut parent = Value::Null;
        let mut push = |lines: &mut Vec<Value>, mut e: Value| {
            let eid = format!("e{}", lines.len());
            e["id"] = json!(eid);
            e["parentId"] = parent.clone();
            parent = json!(eid);
            lines.push(e);
        };
        for (i, (role, text)) in turns.iter().enumerate() {
            let message = if *role == "user" {
                json!({"role": "user", "content": [{"type": "text", "text": text}], "timestamp": BASE_MS + i as i64 * 1000})
            } else {
                json!({"role": "assistant", "content": [{"type": "text", "text": text}], "timestamp": BASE_MS + i as i64 * 1000})
            };
            push(
                &mut lines,
                json!({"type": "message", "timestamp": turn_ts(i), "message": message}),
            );
        }
        if let Some(t) = title {
            push(
                &mut lines,
                json!({"type": "session_info", "timestamp": turn_ts(0), "name": t}),
            );
        }
        let dir = self
            .root
            .join("sessions")
            .join(encode_cwd(&cwd.to_string_lossy()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("2026-11-10T10-00-00-000Z_{id}.jsonl"));
        fs::write(&path, jsonl(&lines)).unwrap();
        path
    }
}

/// OpenCode's data home with `opencode.db`, built from the real structure of the tables the
/// session adapter uses.
pub struct OpencodeData {
    pub root: PathBuf,
}

impl OpencodeData {
    pub fn new(dir: &Path) -> Self {
        OpencodeData {
            root: dir.to_path_buf(),
        }
    }

    pub fn db(&self) -> PathBuf {
        self.root.join("opencode.db")
    }

    pub fn connect(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(self.db()).unwrap()
    }

    /// Creates `opencode.db` (structure only, no rows) plus the `extra` column definitions added
    /// to `session_v2`.
    pub fn write_db(&self, extra: &[&str]) {
        fs::create_dir_all(&self.root).unwrap();
        let conn = self.connect();
        conn.execute_batch(include_str!("testkit_opencode_schema.sql"))
            .unwrap();
        for column in extra {
            conn.execute(&format!("ALTER TABLE session_v2 ADD COLUMN {column}"), [])
                .unwrap();
        }
    }

    /// A session OpenCode itself created: its `project` row (the id of a directory without a
    /// version-control marker) if missing, the `session_v2` row, one message row per turn, and the
    /// `event_sequence` row. Times are `BASE_MS`-based, `updated_ms` is the last update.
    pub fn add_session(
        &self,
        id: &str,
        cwd: &Path,
        title: Option<&str>,
        turns: Turns,
        updated_ms: i64,
    ) {
        let conn = self.connect();
        let project = crate::adapters::opencode_project::sha1_hex(
            format!("directory:{}", cwd.display()).as_bytes(),
        );
        conn.execute(
            "INSERT OR IGNORE INTO project (id, worktree, sandboxes, time_created, time_updated) \
             VALUES (?1, ?2, '[]', 1, 1)",
            rusqlite::params![project, cwd.to_string_lossy()],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_v2 (id, project_id, slug, directory, path, title, version, \
             time_created, time_updated) VALUES (?1, ?2, 'quick-fox', ?3, '', ?4, '2.2.6', ?5, ?6)",
            rusqlite::params![
                id,
                project,
                cwd.to_string_lossy(),
                title,
                BASE_MS,
                updated_ms
            ],
        )
        .unwrap();
        for (i, (role, text)) in turns.iter().enumerate() {
            let ts = BASE_MS + i as i64 * 1000;
            let (kind, data) = if *role == "user" {
                ("user", json!({"text": text, "time": {"created": ts}}))
            } else {
                (
                    "assistant",
                    json!({"agent": "build", "model": {"providerID": "p", "id": "m"},
                           "content": [{"type": "reasoning", "text": "hidden", "time": {"created": ts}},
                                       {"type": "text", "text": text}],
                           "finish": "stop", "time": {"created": ts, "completed": ts}}),
                )
            };
            self.add_message(id, 4 + i as i64, kind, ts, &data);
        }
        conn.execute(
            "INSERT INTO event_sequence (aggregate_id, seq) VALUES (?1, ?2)",
            rusqlite::params![id, 3 + turns.len() as i64],
        )
        .unwrap();
    }

    /// One `session_message` row with the given `data`.
    pub fn add_message(&self, session: &str, seq: i64, kind: &str, ts: i64, data: &Value) {
        self.connect()
            .execute(
                "INSERT INTO session_message (id, session_id, type, seq, time_created, time_updated, data) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6)",
                rusqlite::params![
                    format!("msg_native{session}{seq}"),
                    session,
                    kind,
                    seq,
                    ts,
                    data.to_string()
                ],
            )
            .unwrap();
    }

    /// The ids of every row of `session_v2`, sorted.
    pub fn session_ids(&self) -> Vec<String> {
        let conn = self.connect();
        let mut stmt = conn
            .prepare("SELECT id FROM session_v2 ORDER BY id")
            .unwrap();
        let ids = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .flatten()
            .collect();
        ids
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_home_layout() {
        let t = tempfile::tempdir().unwrap();
        let h = ClaudeHome::new(t.path());
        let dir = h.add_project(Path::new("/work/my proj"));
        assert_eq!(dir, "-work-my-proj");
        let jsonl =
            fs::read_to_string(t.path().join("projects/-work-my-proj/session.jsonl")).unwrap();
        assert_eq!(jsonl, "{\"cwd\":\"/work/my proj\"}\n");
        let f = h.add_memory(
            &dir,
            "a.md",
            FrontmatterVariant::Flat,
            "a",
            "desc",
            "user",
            "body",
        );
        assert!(fs::read_to_string(f).unwrap().contains("type: user"));
        h.add_memory(
            &dir,
            "b.md",
            FrontmatterVariant::Nested,
            "b",
            "d2",
            "feedback",
            "x",
        );
        let idx =
            fs::read_to_string(t.path().join("projects/-work-my-proj/memory/MEMORY.md")).unwrap();
        assert_eq!(
            idx,
            "# Memory Index\n\n- [a](a.md) — desc\n- [b](b.md) — d2\n"
        );
    }

    #[test]
    fn codex_home_files() {
        let t = tempfile::tempdir().unwrap();
        let h = CodexHome::new(t.path());
        h.write_agents_md("hi");
        h.write_memory_md("m");
        assert_eq!(
            fs::read_to_string(t.path().join("AGENTS.md")).unwrap(),
            "hi"
        );
        assert_eq!(
            fs::read_to_string(t.path().join("memories/MEMORY.md")).unwrap(),
            "m"
        );
    }
}
