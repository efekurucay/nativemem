//! Codex rollouts: `sessions/YYYY/MM/DD/rollout-<ts>-<thread id>.jsonl`. Text turns come from the
//! event stream (it never carries the harness context Codex injects as user messages), tool calls
//! and results from the `response_item` stream. A rollout is a replica, never an original, when it
//! carries this tool's `nativemem_origin`, or Codex's import marker or import-ledger record; the
//! latter map to the Claude session they were imported from.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::codex_imports;
use super::codex_rollout::{ORIGIN_KEY, TITLE_KEY};
use crate::error::{Error, Result};
use crate::model::{AgentId, ItemKey};
use crate::session::{
    clean_title, parse_iso_ms, parse_marker, title_from_turns, too_large_reason, Role, Session,
    SessionOptions, SessionScan, Source, TurnBuilder,
};

/// The text Codex's importer appends as the last assistant event of an imported session.
pub const IMPORT_MARKER: &str = "<EXTERNAL SESSION IMPORTED>";
const MAX_DEPTH: usize = 4;

pub struct CodexSessions {
    home: PathBuf,
    options: SessionOptions,
}

/// What one rollout holds.
pub struct Parsed {
    pub session: Session,
    pub has_cwd: bool,
    /// Sub-agent thread, headless `exec` run, or a thread another agent started.
    pub automated: bool,
    /// The original this tool wrote the rollout from.
    pub origin: Option<ItemKey>,
}

impl CodexSessions {
    pub fn new(home: PathBuf, options: SessionOptions) -> Self {
        CodexSessions { home, options }
    }

    pub fn scan(&self, agent: &AgentId) -> Result<SessionScan> {
        let mut out = SessionScan::default();
        let root = self.home.join("sessions");
        let mut files = Vec::new();
        collect_rollouts(&root, 0, &mut files)?;
        files.sort();
        let imports = codex_imports::read(&self.home);
        for path in files {
            let native_id = path
                .strip_prefix(&root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            let mtime_ms = match self.options.window().check(&path)? {
                Source::Outside => continue,
                Source::TooLarge(bytes) => {
                    out.skipped.push((native_id, too_large_reason(bytes)));
                    continue;
                }
                Source::Read { mtime_ms } => mtime_ms,
            };
            let stem = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let thread_id = thread_id_of(&stem);
            let bytes = fs::read(&path).map_err(|e| Error::io(&path, e))?;
            let text = String::from_utf8_lossy(&bytes);
            let Some(parsed) = parse(&text, thread_id, mtime_ms) else {
                continue;
            };
            let import = imports.iter().find(|r| r.thread_id == thread_id);
            let replica_of = match parsed.origin.clone() {
                Some(origin) => Some(origin),
                None if import.is_some() || text.contains(IMPORT_MARKER) => {
                    match import.and_then(|r| claude_key(&r.source_path)) {
                        Some(key) => Some(key),
                        None => continue,
                    }
                }
                None => None,
            };
            if !parsed.session.has_user_turn() {
                continue;
            }
            if replica_of.is_none() && parsed.automated {
                out.automated += 1;
                if self.options.exclude_automated {
                    continue;
                }
            }
            if parsed.has_cwd {
                out.observed
                    .push(parsed.session.observed(agent, native_id, path, replica_of));
            } else {
                out.skipped
                    .push((native_id, "no cwd in session_meta".to_string()));
            }
        }
        Ok(out)
    }
}

/// The Claude session an import-ledger `source_path` names: `<project dir>/<file>`.
fn claude_key(source_path: &str) -> Option<ItemKey> {
    let p = Path::new(source_path);
    let file = p.file_name()?.to_string_lossy();
    let dir = p.parent()?.file_name()?.to_string_lossy();
    (file.ends_with(".jsonl") && !dir.is_empty()).then(|| ItemKey {
        origin: AgentId::new("claude"),
        native_id: format!("{dir}/{file}"),
    })
}

/// The UUID at the end of `rollout-<ts>-<uuid>`.
pub(super) fn thread_id_of(stem: &str) -> &str {
    stem.get(stem.len().saturating_sub(36)..).unwrap_or(stem)
}

pub(super) fn collect_rollouts(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) -> Result<()> {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(Error::io(dir, e)),
    };
    for entry in entries {
        let path = entry.map_err(|e| Error::io(dir, e))?.path();
        if path.is_dir() {
            if depth < MAX_DEPTH {
                collect_rollouts(&path, depth + 1, out)?;
            }
        } else if path.is_file() {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if name.starts_with("rollout-") && name.ends_with(".jsonl") {
                out.push(path);
            }
        }
    }
    Ok(())
}

/// Text of an output value: a string, or an array of parts that carry `text`.
fn output_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|p| p.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn item_text(item: &Value) -> String {
    item.get("content")
        .and_then(Value::as_array)
        .map(|parts| {
            parts
                .iter()
                .filter_map(|p| p.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n\n")
        })
        .unwrap_or_default()
}

/// Parses a rollout. `None` for rollouts without `session_meta`.
pub fn parse(text: &str, thread_id: &str, mtime_ms: i64) -> Option<Parsed> {
    let mut meta: Option<Value> = None;
    let mut builder = TurnBuilder::default();
    let mut last_ts = mtime_ms;
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if let Some(ms) = v
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(parse_iso_ms)
        {
            last_ts = ms;
        }
        let Some(payload) = v.get("payload") else {
            continue;
        };
        let kind = payload.get("type").and_then(Value::as_str);
        let mut text_turn = |role: Role, text: String| {
            let dup = builder
                .turns
                .last()
                .is_some_and(|t| t.role == role && t.text == text);
            if !dup && text != IMPORT_MARKER {
                builder.text(role, text, last_ts);
            }
        };
        match (v.get("type").and_then(Value::as_str), kind) {
            (Some("session_meta"), _) => meta = meta.or_else(|| Some(payload.clone())),
            (Some("event_msg"), Some("user_message")) => {
                let msg = payload.get("message").and_then(Value::as_str);
                text_turn(Role::User, msg.unwrap_or_default().to_string());
            }
            (Some("event_msg"), Some("agent_message")) => {
                let msg = payload.get("message").and_then(Value::as_str);
                text_turn(Role::Assistant, msg.unwrap_or_default().to_string());
            }
            (Some("event_msg"), Some("item_completed")) => {
                let item = payload.get("item").unwrap_or(&Value::Null);
                match item.get("type").and_then(Value::as_str) {
                    Some("UserMessage") => text_turn(Role::User, item_text(item)),
                    Some("AgentMessage") => text_turn(Role::Assistant, item_text(item)),
                    _ => {}
                }
            }
            (Some("response_item"), Some("function_call")) => {
                let name = payload
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("tool");
                let raw = payload
                    .get("arguments")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let input = serde_json::from_str::<Value>(raw)
                    .unwrap_or_else(|_| Value::String(raw.to_string()));
                let id = payload.get("call_id").and_then(Value::as_str);
                builder.call(id, name, &input, last_ts);
            }
            (Some("response_item"), Some("custom_tool_call")) => {
                let name = payload
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("tool");
                let input = payload.get("input").cloned().unwrap_or(Value::Null);
                let id = payload.get("call_id").and_then(Value::as_str);
                builder.call(id, name, &input, last_ts);
            }
            (Some("response_item"), Some("function_call_output" | "custom_tool_call_output")) => {
                let out = output_text(payload.get("output").unwrap_or(&Value::Null));
                let id = payload.get("call_id").and_then(Value::as_str);
                builder.result(id, &out, false, last_ts);
            }
            _ => {}
        }
    }
    let turns = builder.turns;
    let meta = meta?;
    let text_of = |key: &str| meta.get(key).and_then(Value::as_str);
    let automated = meta.get("source").is_some_and(|s| !s.is_string())
        || meta.get("parent_thread_id").is_some_and(|p| !p.is_null())
        || text_of("source") == Some("exec")
        || text_of("thread_source") == Some("subagent")
        || matches!(text_of("originator"), Some("codex_exec" | "Claude Code"));
    let cwd = meta
        .get("cwd")
        .and_then(Value::as_str)
        .filter(|c| !c.is_empty());
    let id = meta
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or(thread_id)
        .to_string();
    let created = meta
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(parse_iso_ms)
        .or_else(|| turns.first().map(|t| t.ts_ms))
        .unwrap_or(mtime_ms);
    Some(Parsed {
        has_cwd: cwd.is_some(),
        automated,
        origin: text_of(ORIGIN_KEY).and_then(parse_marker),
        session: Session {
            id,
            cwd: PathBuf::from(cwd.unwrap_or_default()),
            title: text_of(TITLE_KEY).map_or_else(|| title_from_turns(&turns), clean_title),
            created_ms: created,
            updated_ms: mtime_ms,
            turns,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn line(ts: &str, kind: &str, payload: Value) -> String {
        format!(
            "{}\n",
            json!({"timestamp": ts, "type": kind, "payload": payload})
        )
    }

    fn meta(extra: Value) -> String {
        let mut p = json!({"id": "tid", "cwd": "/w/app", "timestamp": "2026-11-10T09:00:00.000Z", "source": "cli"});
        for (k, v) in extra.as_object().unwrap() {
            p[k] = v.clone();
        }
        line("2026-11-10T09:00:00.000Z", "session_meta", p)
    }

    #[test]
    fn reads_events_and_tool_items_but_not_injected_context() {
        let mut t = meta(json!({}));
        t += &line(
            "2026-11-10T09:00:01.000Z",
            "response_item",
            json!({"type": "message", "role": "user", "content": [{"type": "input_text", "text": "<environment_context>x"}]}),
        );
        t += &line(
            "2026-11-10T09:00:02.000Z",
            "event_msg",
            json!({"type": "item_completed", "item": {"type": "UserMessage", "content": [{"type": "text", "text": "fix the bug"}]}}),
        );
        t += &line(
            "2026-11-10T09:00:03.000Z",
            "event_msg",
            json!({"type": "item_completed", "item": {"type": "AgentMessage", "content": [{"type": "Text", "text": "looking"}]}}),
        );
        t += &line(
            "2026-11-10T09:00:04.000Z",
            "response_item",
            json!({"type": "function_call", "name": "shell", "arguments": "{\"command\":\"ls\"}", "call_id": "c1"}),
        );
        t += &line(
            "2026-11-10T09:00:05.000Z",
            "response_item",
            json!({"type": "function_call_output", "call_id": "c1", "output": "a b"}),
        );
        t += &line(
            "2026-11-10T09:00:06.000Z",
            "response_item",
            json!({"type": "custom_tool_call", "name": "apply_patch", "input": "*** patch", "call_id": "c2"}),
        );
        t += &line(
            "2026-11-10T09:00:07.000Z",
            "response_item",
            json!({"type": "custom_tool_call_output", "call_id": "c2", "output": [{"type": "input_text", "text": "ok"}]}),
        );
        t += &line(
            "2026-11-10T09:00:08.000Z",
            "event_msg",
            json!({"type": "agent_message", "message": "done"}),
        );
        t += &line(
            "2026-11-10T09:00:09.000Z",
            "event_msg",
            json!({"type": "token_count"}),
        );
        let p = parse(&t, "fallback", 5).unwrap();
        assert!(p.has_cwd);
        let s = p.session;
        assert_eq!(s.id, "tid");
        assert_eq!(s.title, "fix the bug");
        let texts: Vec<&str> = s.turns.iter().map(|t| t.text.as_str()).collect();
        assert_eq!(texts[0], "fix the bug");
        assert_eq!(texts[1], "looking");
        assert_eq!(
            texts[2],
            "Tool: shell\n```\nls\n```\nResult:\n```\na b\n```"
        );
        assert_eq!(
            texts[3],
            "Tool: apply_patch\n```\n*** patch\n```\nResult:\n```\nok\n```"
        );
        assert_eq!(texts[4], "done");
        assert_eq!(s.turns.len(), 5);
        assert_eq!(s.turns[0].role, Role::User);
        assert_eq!(
            s.turns[0].ts_ms,
            parse_iso_ms("2026-11-10T09:00:02.000Z").unwrap()
        );
    }

    #[test]
    fn js_style_call_and_json_wrapper_are_unwrapped() {
        let mut t = meta(json!({}));
        t += &line(
            "2026-11-10T09:00:02.000Z",
            "event_msg",
            json!({"type": "user_message", "message": "show it"}),
        );
        t += &line(
            "2026-11-10T09:00:03.000Z",
            "response_item",
            json!({"type": "custom_tool_call", "name": "exec", "call_id": "c1",
            "input": "text(await tools.exec_command({cmd:\"cat \\\"a b\\\"\\nls\",max_output_tokens:5000}));"}),
        );
        t += &line(
            "2026-11-10T09:00:04.000Z",
            "response_item",
            json!({"type": "custom_tool_call_output", "call_id": "c1",
            "output": "{\"chunk_id\":\"1\",\"wall_time_seconds\":0.1,\"exit_code\":3,\"original_token_count\":9,\"output\":\"missing\\n\"}"}),
        );
        t += &line(
            "2026-11-10T09:00:05.000Z",
            "response_item",
            json!({"type": "custom_tool_call", "name": "exec", "call_id": "c2", "input": "x"}),
        );
        t += &line(
            "2026-11-10T09:00:06.000Z",
            "response_item",
            json!({"type": "custom_tool_call_output", "call_id": "c2",
            "output": [{"type": "input_text", "text": "z".repeat(1600)}]}),
        );
        let s = parse(&t, "x", 0).unwrap().session;
        assert_eq!(s.turns.len(), 3);
        assert_eq!(
            s.turns[1].text,
            "Tool: exec\n```\ncat \"a b\"\nls\n```\nResult (exit code 3):\n```\nmissing\n```"
        );
        assert!(s.turns[2].text.ends_with("… (100 more characters)"));
        assert!(!s
            .turns
            .iter()
            .any(|t| t.text.contains("chunk_id") || t.text.contains("external_agent")));
    }

    #[test]
    fn legacy_events_work_and_automated_runs_are_flagged() {
        let mut t = meta(json!({}));
        t += &line(
            "2026-11-10T09:00:02.000Z",
            "event_msg",
            json!({"type": "user_message", "message": "hello"}),
        );
        t += &line(
            "2026-11-10T09:00:03.000Z",
            "event_msg",
            json!({"type": "agent_message", "message": "hi"}),
        );
        assert_eq!(parse(&t, "x", 0).unwrap().session.turns.len(), 2);
        assert!(!parse(&t, "x", 0).unwrap().automated);
        let automated = |extra: Value| parse(&meta(extra), "x", 0).unwrap().automated;
        assert!(automated(json!({"source": {"subagent": {}}})));
        assert!(automated(json!({"parent_thread_id": "p"})));
        assert!(automated(json!({"source": "exec"})));
        assert!(automated(json!({"thread_source": "subagent"})));
        assert!(automated(json!({"originator": "codex_exec"})));
        assert!(automated(json!({"originator": "Claude Code"})));
        assert!(!automated(
            json!({"originator": "codex-tui", "thread_source": "user"})
        ));
        assert!(!automated(
            json!({"source": "vscode", "originator": "Codex Desktop"})
        ));
        assert!(parse("", "x", 0).is_none());
    }

    #[test]
    fn thread_id_is_the_trailing_uuid() {
        assert_eq!(
            thread_id_of("rollout-2026-11-10T09-00-00-0198a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b"),
            "0198a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b"
        );
    }
}
