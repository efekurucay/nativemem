//! Writes a session as a Codex rollout, shaped like the output of Codex's own importer for an
//! imported Claude session: `session_meta`, one `external-import-turn-N` turn per user message with
//! event and response-item pairs, the `<EXTERNAL SESSION IMPORTED>` marker event once, a
//! `token_count` and the closing `task_complete`. The marker event is what makes the rollout a
//! replica; `nativemem_origin` and `nativemem_title` in `session_meta` let a re-read map it back
//! to its original. Only the rollout file is produced here; `codex_threads.rs` derives the
//! `threads` row from it.

use serde_json::{json, Value};

use crate::model::ItemKey;
use crate::session::{format_iso_ms, marker_text, uuid_v7, Role, Session};

use super::codex_sessions::IMPORT_MARKER;

/// `session_meta` values that Codex's `threads` row repeats.
pub const CLI_VERSION: &str = "0.167.0-alpha.3.2";
pub const SOURCE: &str = "vscode";
pub const MODEL_PROVIDER: &str = "openai";
pub const HISTORY_MODE: &str = "legacy";
const BASE_INSTRUCTIONS: &str = "You are Codex, a coding agent.";
/// `session_meta` keys this tool adds.
pub const ORIGIN_KEY: &str = "nativemem_origin";
pub const TITLE_KEY: &str = "nativemem_title";

/// The thread id of the replica of `key`: a UUIDv7 of the session's creation time.
pub fn thread_id(session: &Session, key: &ItemKey) -> String {
    uuid_v7(
        session.created_ms,
        &format!("codex-replica:{}", marker_text(key)),
    )
}

/// `YYYY/MM/DD/rollout-YYYY-MM-DDTHH-MM-SS-<thread id>.jsonl` below `sessions/` (UTC).
pub fn relative_path(session: &Session, thread_id: &str) -> String {
    let iso = format_iso_ms(session.created_ms);
    let (date, time) = iso.split_once('T').unwrap_or((&iso, ""));
    let stamp = format!("{date}T{}", time[..8].replace(':', "-"));
    format!(
        "{}/rollout-{stamp}-{thread_id}.jsonl",
        date.replace('-', "/")
    )
}

/// `originator` of `session_meta`: what the importer wrote for Claude sessions, the origin
/// agent's display name for everything else.
fn originator(origin: &str) -> String {
    match origin {
        "claude" => "Codex Desktop".to_string(),
        "pi" => "Pi".to_string(),
        "opencode" => "OpenCode".to_string(),
        "hermes" => "Hermes".to_string(),
        other => other.to_string(),
    }
}

fn message(role: &str, kind: &str, text: &str) -> Value {
    json!({ "type": "message", "role": role, "content": [{ "type": kind, "text": text }] })
}

/// The rollout text; `now_ms` stamps every line, increasing by a millisecond per line.
pub fn render(session: &Session, key: &ItemKey, thread: &str, now_ms: i64) -> String {
    let mut clock = now_ms;
    let mut lines: Vec<Value> = Vec::new();
    let mut push = |kind: &str, payload: Value| {
        clock += 1;
        lines.push(json!({ "timestamp": format_iso_ms(clock), "type": kind, "payload": payload }));
    };
    push(
        "session_meta",
        json!({
            "session_id": thread,
            "id": thread,
            "timestamp": format_iso_ms(now_ms),
            "cwd": session.cwd.to_string_lossy(),
            "originator": originator(key.origin.as_str()),
            "cli_version": CLI_VERSION,
            "source": SOURCE,
            "model_provider": MODEL_PROVIDER,
            "base_instructions": {
                "text": BASE_INSTRUCTIONS,
                "provenance": { "type": "model", "model": "gpt-5" },
            },
            "history_mode": HISTORY_MODE,
            "multi_agent_version": "v1",
            "context_window": { "window_id": uuid_v7(session.created_ms, &format!("window:{}", marker_text(key))) },
            ORIGIN_KEY: marker_text(key),
            TITLE_KEY: session.title,
        }),
    );

    let secs = |ms: i64| ms.div_euclid(1000);
    let mut turn_no = 0;
    let mut started = 0;
    let mut chars = 0usize;
    for turn in &session.turns {
        if turn.role == Role::User || turn_no == 0 {
            if turn_no > 0 {
                push(
                    "event_msg",
                    json!({ "type": "task_complete", "turn_id": format!("external-import-turn-{turn_no}"),
                            "last_agent_message": null, "started_at": started }),
                );
            }
            turn_no += 1;
            started = secs(turn.ts_ms);
            push(
                "event_msg",
                json!({ "type": "task_started", "turn_id": format!("external-import-turn-{turn_no}"),
                        "started_at": started, "model_context_window": null,
                        "collaboration_mode_kind": "default" }),
            );
        }
        chars += turn.text.len();
        match turn.role {
            Role::User => {
                push(
                    "event_msg",
                    json!({ "type": "user_message", "message": turn.text, "local_images": [],
                            "local_audio": [], "text_elements": [] }),
                );
                push("response_item", message("user", "input_text", &turn.text));
            }
            Role::Assistant => {
                push(
                    "event_msg",
                    json!({ "type": "agent_message", "message": turn.text, "phase": null,
                            "memory_citation": null }),
                );
                push(
                    "response_item",
                    message("assistant", "output_text", &turn.text),
                );
            }
        }
    }
    push(
        "event_msg",
        json!({ "type": "agent_message", "message": IMPORT_MARKER, "phase": null, "memory_citation": null }),
    );
    let usage = json!({ "input_tokens": 0, "cached_input_tokens": 0, "cache_write_input_tokens": 0,
        "output_tokens": 0, "reasoning_output_tokens": 0, "total_tokens": chars.div_ceil(4) });
    push(
        "event_msg",
        json!({ "type": "token_count",
                "info": { "total_token_usage": usage, "last_token_usage": usage, "model_context_window": null },
                "rate_limits": null }),
    );
    push(
        "event_msg",
        json!({ "type": "task_complete", "turn_id": format!("external-import-turn-{turn_no}"),
                "last_agent_message": null, "started_at": started,
                "completed_at": secs(session.turns.last().map_or(now_ms, |t| t.ts_ms)) }),
    );
    lines.iter().map(|l| format!("{l}\n")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::AgentId;
    use crate::session::{parse_iso_ms, Turn};
    use std::path::PathBuf;

    fn session() -> Session {
        let t0 = parse_iso_ms("2026-11-10T10:00:00.000Z").unwrap();
        Session {
            id: "o".into(),
            cwd: PathBuf::from("/w/app"),
            title: "Fix login".into(),
            created_ms: t0,
            updated_ms: t0 + 9_000,
            turns: vec![
                Turn::new(Role::Assistant, "preamble", t0),
                Turn::new(Role::User, "q1", t0 + 1_000),
                Turn::new(Role::Assistant, "a1", t0 + 2_000),
                Turn::new(Role::User, "q2", t0 + 3_000),
            ],
        }
    }

    fn key(origin: &str) -> ItemKey {
        ItemKey {
            origin: AgentId::new(origin),
            native_id: "d/f.jsonl".into(),
        }
    }

    #[test]
    fn layout_follows_the_importer_output() {
        let s = session();
        let k = key("claude");
        let id = thread_id(&s, &k);
        let text = render(&s, &k, &id, 5_000_000_000_000);
        let lines: Vec<Value> = text
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        let kinds: Vec<String> = lines
            .iter()
            .map(|l| {
                format!(
                    "{}:{}",
                    l["type"].as_str().unwrap(),
                    l["payload"]["type"].as_str().unwrap_or("-")
                )
            })
            .collect();
        let expected = [
            "session_meta:-",
            "event_msg:task_started",
            "event_msg:agent_message",
            "response_item:message",
            "event_msg:task_complete",
            "event_msg:task_started",
            "event_msg:user_message",
            "response_item:message",
            "event_msg:agent_message",
            "response_item:message",
            "event_msg:task_complete",
            "event_msg:task_started",
            "event_msg:user_message",
            "response_item:message",
            "event_msg:agent_message",
            "event_msg:token_count",
            "event_msg:task_complete",
        ];
        assert_eq!(kinds, expected);
        let meta = &lines[0]["payload"];
        assert_eq!(meta["id"], id.as_str());
        assert_eq!(meta["originator"], "Codex Desktop");
        assert_eq!(meta["cli_version"], "0.167.0-alpha.3.2");
        assert_eq!(meta["history_mode"], "legacy");
        assert_eq!(lines[1]["payload"]["turn_id"], "external-import-turn-1");
        assert_eq!(lines[16]["payload"]["turn_id"], "external-import-turn-3");
        assert_eq!(lines[16]["payload"]["completed_at"], 1_794_304_803);
        assert_eq!(lines[4]["payload"].get("completed_at"), None);
        let stamps: Vec<&str> = lines
            .iter()
            .map(|l| l["timestamp"].as_str().unwrap())
            .collect();
        assert!(stamps.windows(2).all(|w| w[0] < w[1]), "monotonic");
        assert_eq!(text.matches(IMPORT_MARKER).count(), 1);
        let pi = render(&s, &key("pi"), &id, 1);
        assert!(pi.lines().next().unwrap().contains("\"originator\":\"Pi\""));
    }

    #[test]
    fn path_and_id_derive_from_the_creation_time() {
        let s = session();
        let id = thread_id(&s, &key("claude"));
        assert_eq!(id, thread_id(&s, &key("claude")));
        assert_ne!(id, thread_id(&s, &key("pi")));
        assert_eq!(id.as_bytes()[14], b'7');
        assert_eq!(
            relative_path(&s, &id),
            format!("2026/11/10/rollout-2026-11-10T10-00-00-{id}.jsonl")
        );
        let mut later = session();
        later.created_ms += 1;
        assert!(id < thread_id(&later, &key("claude")));
    }
}
