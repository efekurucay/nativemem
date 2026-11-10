//! Pi: global `AGENTS.md` instruction sections and project memory (through the generic
//! agents-file adapter) and chat sessions, `sessions/--<cwd>--/<ISO ts>_<session id>.jsonl`.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::compat::Check;
use crate::error::{Error, Result};
use crate::fsx::{self, BackupSession};
use crate::model::{Adapter, AgentId, Item, ItemKey, Kind, Observed, Scope};
use crate::session::{
    clean_title, derived_uuid, format_iso_ms, is_session_id, join_user_blocks, marker_text,
    parse_iso_ms, parse_marker, result_content_text, set_mtime, title_from_turns, too_large_reason,
    Role, Session, SessionOptions, SessionReport, SessionScan, Source, TurnBuilder,
};

use super::agents_file::AgentsFileAdapter;
use super::pi_probes;
use super::skills::{skill_name, SkillReport};

const AGENT: &str = "pi";
/// `customType` of the entry that marks a session as a replica. Pi keeps custom entries out of
/// the model context.
pub const MARKER_TYPE: &str = "nativemem-origin";
/// Bump when the written session format changes; unmodified replicas are then rewritten.
const SESSION_WRITER_REVISION: u32 = 2;
/// Assistant messages of a replica carry this `api`. Pi restores the model a session recorded from
/// the last `model_change` or the last assistant message that is not virtual
/// (`core/virtual-models.js` `getBranchSelection`), and warns when that model does not exist. A
/// virtual message never names a restorable model, so the replica opens with Pi's own default.
const REPLICA_API: &str = "pi-virtual";

pub struct PiAdapter {
    home: PathBuf,
    state_dir: PathBuf,
    backups: BackupSession,
    instructions: AgentsFileAdapter,
    options: Option<SessionOptions>,
    search_path: Option<std::ffi::OsString>,
    blocked: Vec<Kind>,
}

/// What one session file holds.
pub struct Parsed {
    pub session: Session,
    pub replica_of: Option<ItemKey>,
}

/// Pi's directory name for a working directory: one leading separator dropped, `/`, `\` and `:`
/// replaced by `-`, wrapped in `--`.
pub fn encode_cwd(cwd: &str) -> String {
    let rest = cwd.strip_prefix(['/', '\\']).unwrap_or(cwd);
    let enc: String = rest
        .chars()
        .map(|c| {
            if matches!(c, '/' | '\\' | ':') {
                '-'
            } else {
                c
            }
        })
        .collect();
    format!("--{enc}--")
}

fn add_message(b: &mut TurnBuilder, msg: &Value, fallback_ts: i64) {
    let ts = msg
        .get("timestamp")
        .and_then(Value::as_i64)
        .unwrap_or(fallback_ts);
    let content = msg.get("content").unwrap_or(&Value::Null);
    match msg.get("role").and_then(Value::as_str) {
        Some("user") => match content {
            Value::String(s) => b.text(Role::User, s.as_str(), ts),
            Value::Array(blocks) => b.text(Role::User, join_user_blocks(blocks), ts),
            _ => {}
        },
        Some("assistant") => match content {
            Value::String(s) => b.text(Role::Assistant, s.as_str(), ts),
            Value::Array(blocks) => {
                for block in blocks {
                    match block.get("type").and_then(Value::as_str) {
                        Some("text") => {
                            let text = block.get("text").and_then(Value::as_str).unwrap_or("");
                            b.text(Role::Assistant, text, ts);
                        }
                        Some("toolCall") => {
                            let name = block.get("name").and_then(Value::as_str).unwrap_or("tool");
                            let id = block.get("id").and_then(Value::as_str);
                            b.call(id, name, block.get("arguments").unwrap_or(&Value::Null), ts);
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        },
        Some("toolResult") => {
            let is_error = msg.get("isError").and_then(Value::as_bool) == Some(true);
            let id = msg.get("toolCallId").and_then(Value::as_str);
            b.result(id, &result_content_text(content), is_error, ts);
        }
        Some("bashExecution") => {
            if msg.get("excludeFromContext").and_then(Value::as_bool) == Some(true) {
                return;
            }
            let command = msg.get("command").and_then(Value::as_str).unwrap_or("");
            let output = msg.get("output").and_then(Value::as_str).unwrap_or("");
            let code = msg
                .get("exitCode")
                .and_then(Value::as_i64)
                .filter(|c| *c != 0);
            let name = "bash";
            b.call(None, name, &serde_json::json!({ "command": command }), ts);
            let shown = match code {
                Some(c) => serde_json::json!({ "output": output, "exit_code": c }).to_string(),
                None => output.to_string(),
            };
            b.result(None, &shown, false, ts);
        }
        _ => {}
    }
}

/// Parses a session file. `None` when the first line is not a session header or the header has
/// no working directory. The conversation is the path from the last entry to the root.
pub fn parse(text: &str, mtime_ms: i64) -> Option<Parsed> {
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let header: Value = serde_json::from_str(lines.next()?).ok()?;
    if header.get("type").and_then(Value::as_str) != Some("session") {
        return None;
    }
    let id = header.get("id")?.as_str()?.to_string();
    let cwd = header
        .get("cwd")
        .and_then(Value::as_str)
        .filter(|c| !c.is_empty())?;
    let header_ms = header
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(parse_iso_ms);

    let entries: Vec<Value> = lines
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|e| e.get("id").and_then(Value::as_str).is_some())
        .collect();
    let by_id: std::collections::HashMap<&str, usize> = entries
        .iter()
        .enumerate()
        .filter_map(|(i, e)| Some((e.get("id")?.as_str()?, i)))
        .collect();
    let mut path = Vec::new();
    let mut seen = HashSet::new();
    let mut cursor = entries.len().checked_sub(1);
    while let Some(i) = cursor {
        if !seen.insert(i) {
            break;
        }
        path.push(i);
        cursor = entries[i]
            .get("parentId")
            .and_then(Value::as_str)
            .and_then(|p| by_id.get(p).copied());
    }
    path.reverse();

    let mut name = None;
    let mut origin = None;
    for e in &entries {
        match e.get("type").and_then(Value::as_str) {
            Some("session_info") => {
                name = e
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|n| !n.is_empty())
                    .map(str::to_string);
            }
            Some("custom") if e.get("customType").and_then(Value::as_str) == Some(MARKER_TYPE) => {
                origin = origin.or_else(|| {
                    e.get("data")?
                        .get("origin")?
                        .as_str()
                        .and_then(parse_marker)
                });
            }
            _ => {}
        }
    }

    let mut builder = TurnBuilder::default();
    let mut last_ts = header_ms.unwrap_or(mtime_ms);
    for i in path {
        let e = &entries[i];
        if e.get("type").and_then(Value::as_str) != Some("message") {
            continue;
        }
        if let Some(ms) = e
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(parse_iso_ms)
        {
            last_ts = ms;
        }
        if let Some(m) = e.get("message") {
            add_message(&mut builder, m, last_ts);
        }
    }
    let turns = builder.turns;
    let title = name.map_or_else(|| title_from_turns(&turns), |n| clean_title(&n));
    Some(Parsed {
        session: Session {
            id,
            cwd: PathBuf::from(cwd),
            title,
            created_ms: header_ms
                .or_else(|| turns.first().map(|t| t.ts_ms))
                .unwrap_or(mtime_ms),
            updated_ms: mtime_ms,
            turns,
        },
        replica_of: origin,
    })
}

/// File text for a replica of `session` carrying the id `uuid`.
pub fn render(session: &Session, key: &ItemKey, uuid: &str) -> String {
    let marker = marker_text(key);
    let mut used: HashSet<String> = HashSet::new();
    let mut next_id = |seed: String| {
        let full = derived_uuid(&seed);
        let mut id = full[..8].to_string();
        if !used.insert(id.clone()) {
            id = full.clone();
            used.insert(id.clone());
        }
        id
    };
    let created = format_iso_ms(session.created_ms);
    let mut lines = vec![json!({
        "type": "session",
        "version": 3,
        "id": uuid,
        "timestamp": created,
        "cwd": session.cwd.to_string_lossy(),
    })];
    let marker_id = next_id(format!("{marker}#marker"));
    lines.push(json!({
        "type": "custom", "id": marker_id, "parentId": null, "timestamp": created,
        "customType": MARKER_TYPE, "data": { "origin": marker },
    }));
    let mut parent = marker_id;
    let title_id = next_id(format!("{marker}#title"));
    lines.push(json!({
        "type": "session_info", "id": title_id, "parentId": parent, "timestamp": created,
        "name": session.title,
    }));
    parent = title_id;
    for (i, turn) in session.turns.iter().enumerate() {
        let id = next_id(format!("{marker}#{i}"));
        let message = match turn.role {
            Role::User => json!({
                "role": "user",
                "content": [{ "type": "text", "text": turn.text }],
                "timestamp": turn.ts_ms,
            }),
            Role::Assistant => json!({
                "role": "assistant",
                "content": [{ "type": "text", "text": turn.text }],
                "api": REPLICA_API,
                "provider": "nativemem",
                "model": "replica",
                "usage": {
                    "input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "totalTokens": 0,
                    "cost": { "input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "total": 0 },
                },
                "stopReason": "stop",
                "timestamp": turn.ts_ms,
            }),
        };
        lines.push(json!({
            "type": "message", "id": id, "parentId": parent,
            "timestamp": format_iso_ms(turn.ts_ms), "message": message,
        }));
        parent = id;
    }
    let mut out = String::new();
    for l in lines {
        out.push_str(&l.to_string());
        out.push('\n');
    }
    out
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn sorted_entries(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut v: Vec<PathBuf> = fs::read_dir(dir)
        .map_err(|e| Error::io(dir, e))?
        .map(|e| e.map(|e| e.path()).map_err(|e| Error::io(dir, e)))
        .collect::<Result<_>>()?;
    v.sort();
    Ok(v)
}

impl PiAdapter {
    pub fn new(home: PathBuf, state_dir: PathBuf, backups: BackupSession) -> Self {
        PiAdapter {
            instructions: AgentsFileAdapter::new(
                AGENT,
                home.clone(),
                state_dir.clone(),
                backups.clone(),
            ),
            home,
            state_dir,
            backups,
            options: None,
            search_path: crate::probe::process_path(),
            blocked: Vec::new(),
        }
    }

    /// Kinds this adapter neither reads nor writes while its compatibility checks fail.
    pub fn with_blocked_kinds(mut self, kinds: &[Kind]) -> Self {
        self.blocked = kinds.to_vec();
        self
    }

    /// Directories Pi also reads skills from; copies there are never written.
    pub fn with_visible_roots(mut self, roots: Vec<PathBuf>) -> Self {
        self.instructions = self.instructions.with_visible_roots(roots);
        self
    }

    /// Where the version probe looks for the `pi` executable [default: `PATH`].
    pub fn with_search_path(mut self, path: std::ffi::OsString) -> Self {
        self.search_path = Some(path);
        self
    }

    /// Takes part in session sync; without this call sessions stay out. Pi's session files carry
    /// no marker for headless runs, so every session counts as interactive.
    pub fn with_sessions(mut self, options: SessionOptions) -> Self {
        self.options = Some(options);
        self
    }

    fn sessions_dir(&self) -> PathBuf {
        self.home.join("sessions")
    }

    fn scan_sessions(&self) -> Result<SessionScan> {
        let mut out = SessionScan::default();
        if self.blocked.contains(&Kind::Session) {
            return Ok(out);
        }
        let Some(window) = self.options.map(|o| o.window()) else {
            return Ok(out);
        };
        let root = self.sessions_dir();
        if !root.is_dir() {
            return Ok(out);
        }
        for dir in sorted_entries(&root)? {
            if !dir.is_dir() {
                continue;
            }
            for path in sorted_entries(&dir)? {
                if !path.is_file() || path.extension().is_none_or(|e| e != "jsonl") {
                    continue;
                }
                let native_id = format!("{}/{}", file_name(&dir), file_name(&path));
                let mtime_ms = match window.check(&path)? {
                    Source::Outside => continue,
                    Source::TooLarge(bytes) => {
                        out.skipped.push((native_id, too_large_reason(bytes)));
                        continue;
                    }
                    Source::Read { mtime_ms } => mtime_ms,
                };
                let bytes = fs::read(&path).map_err(|e| Error::io(&path, e))?;
                let Some(parsed) = parse(&String::from_utf8_lossy(&bytes), mtime_ms) else {
                    continue;
                };
                if parsed.session.has_user_turn() {
                    out.observed.push(parsed.session.observed(
                        &self.id(),
                        native_id,
                        path,
                        parsed.replica_of,
                    ));
                }
            }
        }
        Ok(out)
    }

    /// Sessions that cannot be synced; Pi has no automated ones.
    pub fn session_report(&self) -> Result<SessionReport> {
        Ok(self.scan_sessions()?.report())
    }

    fn write_session_replica(&self, key: &ItemKey, item: &Item) -> Result<(String, PathBuf)> {
        let session = Session::from_body(&item.body)
            .ok_or_else(|| Error::Parse("session body is not a session".into()))?;
        let uuid = derived_uuid(&format!("pi-replica:{}", marker_text(key)));
        let dir = self
            .sessions_dir()
            .join(encode_cwd(&session.cwd.to_string_lossy()));
        let suffix = format!("_{uuid}.jsonl");
        let existing = match fs::read_dir(&dir) {
            Ok(entries) => entries
                .flatten()
                .map(|e| e.path())
                .find(|p| file_name(p).ends_with(&suffix)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(Error::io(&dir, e)),
        };
        let path = existing.unwrap_or_else(|| {
            let ts = format_iso_ms(session.created_ms).replace([':', '.'], "-");
            dir.join(format!("{ts}{suffix}"))
        });
        let text = render(&session, key, &uuid);
        let current = fs::read_to_string(&path).ok();
        if current.as_deref() != Some(text.as_str()) {
            if current.is_some() {
                fsx::backup(
                    &self.backups,
                    &self.state_dir,
                    &self.id(),
                    &self.home,
                    &path,
                )?;
            }
            fsx::atomic_write(&path, text.as_bytes())?;
        }
        set_mtime(&path, session.updated_ms);
        Ok((format!("{}/{}", file_name(&dir), file_name(&path)), path))
    }
}

impl Adapter for PiAdapter {
    fn id(&self) -> AgentId {
        AgentId::new(AGENT)
    }

    fn version(&self) -> Option<String> {
        pi_probes::version(self.search_path.as_deref())
    }

    fn probes(&self) -> Vec<Check> {
        let mut checks = pi_probes::probes(&self.home);
        checks.push(self.instructions.skills_check());
        checks
    }

    fn supports(&self, kind: Kind, scope: &Scope) -> bool {
        !self.blocked.contains(&kind)
            && ((kind == Kind::Session && matches!(scope, Scope::Project(_)))
                || self.instructions.supports(kind, scope))
    }

    fn writer_revision(&self, kind: Kind) -> u32 {
        if kind == Kind::Session {
            SESSION_WRITER_REVISION
        } else {
            0
        }
    }

    fn scan(&self) -> Result<Vec<Observed>> {
        let mut out = self.instructions.scan()?;
        out.retain(|o| !self.blocked.contains(&o.item.kind));
        out.extend(self.scan_sessions()?.observed);
        Ok(out)
    }

    fn visible_copy(&self, native_id: &str) -> Option<String> {
        self.instructions.visible_copy(native_id)
    }

    fn skill_report(&self) -> Result<SkillReport> {
        self.instructions.skill_report()
    }

    fn write_replica(&self, key: &ItemKey, item: &Item) -> Result<(String, PathBuf)> {
        if item.kind == Kind::Session {
            return self.write_session_replica(key, item);
        }
        self.instructions.write_replica(key, item)
    }

    fn create_original(&self, item: &Item) -> Result<(String, PathBuf)> {
        self.instructions.create_original(item)
    }

    fn write_original(&self, native_id: &str, item: &Item) -> Result<()> {
        if skill_name(native_id).is_some() {
            return self.instructions.write_original(native_id, item);
        }
        if is_session_id(native_id) {
            return Err(Error::unsupported(AGENT, "write_original session"));
        }
        self.instructions.write_original(native_id, item)
    }

    fn remove(&self, native_id: &str) -> Result<()> {
        if skill_name(native_id).is_some() {
            return self.instructions.remove(native_id);
        }
        if is_session_id(native_id) {
            return Err(Error::unsupported(AGENT, "remove session"));
        }
        self.instructions.remove(native_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::Turn;

    fn entry(v: Value) -> String {
        format!("{v}\n")
    }

    fn header() -> String {
        entry(
            json!({"type": "session", "version": 3, "id": "pid", "timestamp": "2026-11-10T10:00:00.000Z", "cwd": "/w/app"}),
        )
    }

    fn msg(id: &str, parent: Value, message: Value) -> String {
        entry(
            json!({"type": "message", "id": id, "parentId": parent, "timestamp": "2026-11-10T10:00:01.000Z", "message": message}),
        )
    }

    #[test]
    fn encodes_cwd_like_pi() {
        assert_eq!(
            encode_cwd("/Users/x/work/Foo.github.io"),
            "--Users-x-work-Foo.github.io--"
        );
        assert_eq!(encode_cwd("/Users/x"), "--Users-x--");
        assert_eq!(encode_cwd("C:\\w\\app"), "--C--w-app--");
    }

    #[test]
    fn reads_the_active_branch_and_flattens_tools() {
        let mut t = header();
        t += &msg(
            "a1",
            Value::Null,
            json!({"role": "user", "content": [{"type": "text", "text": "q1"}], "timestamp": 1000}),
        );
        t += &msg(
            "a2",
            json!("a1"),
            json!({"role": "assistant", "content": [
            {"type": "thinking", "thinking": "x"},
            {"type": "text", "text": "answer"},
            {"type": "toolCall", "id": "c", "name": "read", "arguments": {"path": "/a"}}], "timestamp": 2000}),
        );
        t += &msg(
            "a3",
            json!("a2"),
            json!({"role": "toolResult", "toolCallId": "c", "toolName": "read", "content": [{"type": "text", "text": "data"}], "isError": true, "timestamp": 3000}),
        );
        t += &msg(
            "b1",
            json!("a1"),
            json!({"role": "assistant", "content": [{"type": "text", "text": "abandoned branch"}], "timestamp": 2500}),
        );
        t += &msg(
            "a4",
            json!("a3"),
            json!({"role": "user", "content": "q2", "timestamp": 4000}),
        );
        t += &entry(
            json!({"type": "session_info", "id": "n1", "parentId": "a4", "timestamp": "2026-11-10T10:00:02.000Z", "name": " Named "}),
        );
        t += &entry(
            json!({"type": "model_change", "id": "m1", "parentId": "n1", "timestamp": "2026-11-10T10:00:02.000Z", "provider": "p", "modelId": "m"}),
        );
        let p = parse(&t, 7).unwrap();
        let s = p.session;
        assert!(p.replica_of.is_none());
        assert_eq!((s.id.as_str(), s.title.as_str()), ("pid", "Named"));
        assert_eq!(s.cwd, PathBuf::from("/w/app"));
        let texts: Vec<&str> = s.turns.iter().map(|t| t.text.as_str()).collect();
        assert_eq!(texts[0], "q1");
        assert_eq!(texts[1], "answer");
        assert_eq!(
            texts[2],
            "Tool: read\n```\n{\"path\":\"/a\"}\n```\nResult (error):\n```\ndata\n```"
        );
        assert_eq!(texts[3], "q2");
        assert_eq!(texts.len(), 4);
        assert_eq!(s.turns[0].ts_ms, 1000);
    }

    #[test]
    fn bash_executions_and_consecutive_tool_calls_render_without_empty_turns() {
        let mut t = header();
        t += &msg(
            "a1",
            Value::Null,
            json!({"role": "user", "content": "go", "timestamp": 1}),
        );
        t += &msg(
            "a2",
            json!("a1"),
            json!({"role": "assistant", "content": [
            {"type": "toolCall", "id": "x", "name": "ls", "arguments": {"command": "ls"}},
            {"type": "toolCall", "id": "y", "name": "pwd", "arguments": {"command": "pwd"}}], "timestamp": 2}),
        );
        t += &msg(
            "a3",
            json!("a2"),
            json!({"role": "toolResult", "toolCallId": "y", "content": [{"type": "text", "text": "/w"}], "timestamp": 3}),
        );
        t += &msg(
            "a4",
            json!("a3"),
            json!({"role": "toolResult", "toolCallId": "x", "content": [{"type": "text", "text": "f"}], "timestamp": 4}),
        );
        t += &msg(
            "a5",
            json!("a4"),
            json!({"role": "bashExecution", "command": "false", "output": "", "exitCode": 1, "timestamp": 5}),
        );
        let s = parse(&t, 0).unwrap().session;
        let texts: Vec<&str> = s.turns.iter().map(|t| t.text.as_str()).collect();
        assert_eq!(texts[1], "Tool: ls\n```\nls\n```\nResult:\n```\nf\n```");
        assert_eq!(texts[2], "Tool: pwd\n```\npwd\n```\nResult:\n```\n/w\n```");
        assert_eq!(
            texts[3],
            "Tool: bash\n```\nfalse\n```\nResult (exit code 1): (no output)"
        );
        assert_eq!(texts.len(), 4);
        assert!(texts.iter().all(|t| !t.is_empty()));
    }

    #[test]
    fn title_falls_back_to_first_user_text_and_rejects_non_sessions() {
        let mut t = header();
        t += &msg(
            "a1",
            Value::Null,
            json!({"role": "user", "content": "plain string prompt", "timestamp": 1}),
        );
        assert_eq!(parse(&t, 0).unwrap().session.title, "plain string prompt");
        assert!(parse("{\"type\":\"message\"}\n", 0).is_none());
        assert!(parse("", 0).is_none());
    }

    #[test]
    fn rendered_replica_parses_back_with_marker() {
        let session = Session {
            id: "orig".into(),
            cwd: "/w/app".into(),
            title: "Fix login".into(),
            created_ms: parse_iso_ms("2026-11-10T10:00:00.000Z").unwrap(),
            updated_ms: 5,
            turns: vec![
                Turn::new(Role::User, "q", 1_000),
                Turn::new(Role::Assistant, "a", 2_000),
                Turn::new(Role::Assistant, "b", 3_000),
            ],
        };
        let key = ItemKey {
            origin: AgentId::new("claude"),
            native_id: "-w-app/o.jsonl".into(),
        };
        let text = render(&session, &key, "new-id");
        let p = parse(&text, 0).unwrap();
        assert_eq!(p.replica_of, Some(key));
        assert_eq!(p.session.hash(), session.hash());
        assert_eq!(p.session.id, "new-id");
        let ids: Vec<String> = text
            .lines()
            .skip(1)
            .map(|l| {
                serde_json::from_str::<Value>(l).unwrap()["id"]
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect();
        assert_eq!(ids.iter().collect::<HashSet<_>>().len(), ids.len());
    }

    #[test]
    fn replica_names_no_restorable_model_and_has_no_setup_entries() {
        let session = Session {
            id: "o".into(),
            cwd: "/w/app".into(),
            title: "t".into(),
            created_ms: 0,
            updated_ms: 0,
            turns: vec![
                Turn::new(Role::User, "q", 0),
                Turn::new(Role::Assistant, "a", 1),
            ],
        };
        let key = ItemKey {
            origin: AgentId::new("claude"),
            native_id: "d/o.jsonl".into(),
        };
        let text = render(&session, &key, "u");
        assert!(!text.contains("model_change") && !text.contains("thinking_level_change"));
        for l in text.lines() {
            let v: Value = serde_json::from_str(l).unwrap();
            if v["message"]["role"] == "assistant" {
                assert_eq!(v["message"]["api"], "pi-virtual");
            }
        }
    }
}
