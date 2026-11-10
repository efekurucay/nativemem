//! Claude Code transcripts: `projects/<dir>/<session id>.jsonl`. Reading takes user and assistant
//! lines in file order (thinking and meta lines dropped, tool activity merged into text turns);
//! writing produces the smallest file Claude Code lists and resumes.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::error::{Error, Result};
use crate::model::{AgentId, ItemKey};
use crate::session::{
    clean_title, derived_uuid, format_iso_ms, join_user_blocks, marker_text, parse_iso_ms,
    parse_marker, result_content_text, title_from_turns, too_large_reason, truncate_chars, Role,
    Session, SessionOptions, SessionScan, Source, TurnBuilder,
};

/// Extra top-level key on every line a replica is written with. Claude Code reads lines by
/// their known keys, so an unknown key is ignored; it survives the replica being continued
/// because Claude only appends.
pub const MARKER_KEY: &str = "nativemem_origin";
const CLAUDE_VERSION: &str = "2.1.322";
const LAST_PROMPT_LIMIT: usize = 200;

pub struct ClaudeSessions {
    home: PathBuf,
    options: SessionOptions,
}

/// What one transcript file holds.
pub struct Parsed {
    pub session: Option<Session>,
    pub has_cwd: bool,
    pub replica_of: Option<ItemKey>,
    /// The first line's `entrypoint` is an SDK one (`sdk-cli`, `sdk-ts`, ...): a headless run.
    pub automated: bool,
}

impl ClaudeSessions {
    pub fn new(home: PathBuf, options: SessionOptions) -> Self {
        ClaudeSessions { home, options }
    }

    pub fn scan(&self, agent: &AgentId) -> Result<SessionScan> {
        let mut out = SessionScan::default();
        let root = self.home.join("projects");
        if !root.is_dir() {
            return Ok(out);
        }
        for dir in sorted_entries(&root)? {
            if !dir.is_dir() {
                continue;
            }
            let dir_name = file_name(&dir);
            for path in sorted_entries(&dir)? {
                if !path.is_file() || path.extension().is_none_or(|e| e != "jsonl") {
                    continue;
                }
                let native_id = format!("{dir_name}/{}", file_name(&path));
                let mtime_ms = match self.options.window().check(&path)? {
                    Source::Outside => continue,
                    Source::TooLarge(bytes) => {
                        out.skipped.push((native_id, too_large_reason(bytes)));
                        continue;
                    }
                    Source::Read { mtime_ms } => mtime_ms,
                };
                let bytes = fs::read(&path).map_err(|e| Error::io(&path, e))?;
                let stem = path
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let parsed = parse(&String::from_utf8_lossy(&bytes), &stem, mtime_ms);
                if parsed.session.is_some() && parsed.automated {
                    out.automated += 1;
                    if self.options.exclude_automated {
                        continue;
                    }
                }
                match parsed.session {
                    None => {}
                    Some(_) if !parsed.has_cwd => out
                        .skipped
                        .push((native_id, "no cwd in any line".to_string())),
                    Some(session) => {
                        out.observed.push(session.observed(
                            agent,
                            native_id,
                            path,
                            parsed.replica_of,
                        ));
                    }
                }
            }
        }
        Ok(out)
    }
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

/// A tool-result-only user line adds the results to their calls; any other array becomes one
/// user turn.
fn add_user(b: &mut TurnBuilder, content: &Value, ts: i64) {
    match content {
        Value::String(s) => b.text(Role::User, s.as_str(), ts),
        Value::Array(blocks) => {
            let is_result =
                |b: &Value| b.get("type").and_then(Value::as_str) == Some("tool_result");
            if !blocks.is_empty() && blocks.iter().all(is_result) {
                for block in blocks {
                    let text = result_content_text(block.get("content").unwrap_or(&Value::Null));
                    let is_error = block.get("is_error").and_then(Value::as_bool) == Some(true);
                    let id = block.get("tool_use_id").and_then(Value::as_str);
                    b.result(id, &text, is_error, ts);
                }
            } else {
                b.text(Role::User, join_user_blocks(blocks), ts);
            }
        }
        _ => {}
    }
}

fn add_assistant(b: &mut TurnBuilder, content: &Value, ts: i64) {
    match content {
        Value::String(s) => b.text(Role::Assistant, s.as_str(), ts),
        Value::Array(blocks) => {
            for block in blocks {
                match block.get("type").and_then(Value::as_str) {
                    Some("text") => {
                        let text = block.get("text").and_then(Value::as_str).unwrap_or("");
                        b.text(Role::Assistant, text, ts);
                    }
                    Some("tool_use") => {
                        let name = block.get("name").and_then(Value::as_str).unwrap_or("tool");
                        let id = block.get("id").and_then(Value::as_str);
                        b.call(id, name, block.get("input").unwrap_or(&Value::Null), ts);
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

/// Parses a transcript in file order, as Codex's importer does. `stem` is the session id.
pub fn parse(text: &str, stem: &str, mtime_ms: i64) -> Parsed {
    let (mut custom, mut ai, mut last_prompt) = (None, None, None);
    let (mut cwd, mut origin) = (None, None);
    let mut entrypoint: Option<String> = None;
    let mut builder = TurnBuilder::default();
    let mut last_ts = None;
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let str_at = |key: &str| v.get(key).and_then(Value::as_str).map(str::to_string);
        if cwd.is_none() {
            cwd = str_at("cwd").filter(|c| !c.is_empty());
        }
        if origin.is_none() {
            origin = str_at(MARKER_KEY).as_deref().and_then(parse_marker);
        }
        let flag = |key: &str| v.get(key).and_then(Value::as_bool) == Some(true);
        match v.get("type").and_then(Value::as_str) {
            Some("custom-title") => custom = str_at("customTitle").or(custom),
            Some("ai-title") => ai = str_at("aiTitle").or(ai),
            Some("last-prompt") => last_prompt = str_at("lastPrompt").or(last_prompt),
            Some(kind @ ("user" | "assistant")) => {
                if entrypoint.is_none() {
                    entrypoint = str_at("entrypoint");
                }
                if flag("isMeta") || flag("isSidechain") {
                    continue;
                }
                let ts = str_at("timestamp")
                    .as_deref()
                    .and_then(parse_iso_ms)
                    .or(last_ts)
                    .unwrap_or(mtime_ms);
                last_ts = Some(ts);
                let content = v
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .unwrap_or(&Value::Null);
                if kind == "user" {
                    add_user(&mut builder, content, ts);
                } else {
                    add_assistant(&mut builder, content, ts);
                }
            }
            _ => {}
        }
    }
    let turns = builder.turns;
    let has_cwd = cwd.is_some();
    let title = [custom, ai, last_prompt]
        .into_iter()
        .flatten()
        .find(|t| !t.trim().is_empty())
        .map_or_else(|| title_from_turns(&turns), |t| clean_title(&t));
    let session = Session {
        id: stem.to_string(),
        cwd: PathBuf::from(cwd.unwrap_or_default()),
        title,
        created_ms: turns.first().map_or(mtime_ms, |t| t.ts_ms),
        updated_ms: mtime_ms,
        turns,
    };
    Parsed {
        session: session.has_user_turn().then_some(session),
        has_cwd,
        replica_of: origin,
        automated: entrypoint.is_some_and(|e| e.starts_with("sdk")),
    }
}

/// Transcript text for a replica of `session`; `uuid` is the new session id.
pub fn render(session: &Session, key: &ItemKey, uuid: &str) -> String {
    let marker = marker_text(key);
    let mut lines = vec![json!({
        "type": "custom-title",
        "customTitle": session.title,
        "sessionId": uuid,
        MARKER_KEY: marker,
    })];
    let cwd = session.cwd.to_string_lossy();
    let mut parent: Option<String> = None;
    for (i, turn) in session.turns.iter().enumerate() {
        let id = derived_uuid(&format!("{marker}#{i}"));
        let message = match turn.role {
            Role::User => json!({ "role": "user", "content": turn.text }),
            Role::Assistant => json!({
                "id": format!("msg_{}", id.replace('-', "")),
                "type": "message",
                "role": "assistant",
                "model": "<synthetic>",
                "content": [{ "type": "text", "text": turn.text }],
                "stop_reason": "end_turn",
                "stop_sequence": null,
                "usage": { "input_tokens": 0, "output_tokens": 0 },
            }),
        };
        let mut line = json!({
            "parentUuid": parent,
            "isSidechain": false,
            "type": turn.role.as_str(),
            "message": message,
            "uuid": id,
            "timestamp": format_iso_ms(turn.ts_ms),
            "userType": "external",
            "entrypoint": "cli",
            "cwd": cwd,
            "sessionId": uuid,
            "version": CLAUDE_VERSION,
            "gitBranch": "HEAD",
            MARKER_KEY: marker,
        });
        if turn.role == Role::Assistant {
            line["requestId"] = json!(format!("req_{}", id.replace('-', "")));
        }
        lines.push(line);
        parent = Some(id);
    }
    if let (Some(leaf), Some(prompt)) = (
        parent,
        session.turns.iter().rev().find(|t| t.role == Role::User),
    ) {
        lines.push(json!({
            "type": "last-prompt",
            "lastPrompt": truncate_chars(&prompt.text, LAST_PROMPT_LIMIT),
            "leafUuid": leaf,
            "sessionId": uuid,
        }));
    }
    let mut out = String::new();
    for l in lines {
        out.push_str(&l.to_string());
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::Turn;

    fn line(v: Value) -> String {
        format!("{v}\n")
    }

    fn user(text: Value, ts: &str) -> String {
        line(json!({"type": "user", "timestamp": ts, "cwd": "/w/app",
            "message": {"role": "user", "content": text}}))
    }

    fn assistant(blocks: Value, ts: &str) -> String {
        line(
            json!({"type": "assistant", "timestamp": ts, "cwd": "/w/app",
            "message": {"role": "assistant", "content": blocks}}),
        )
    }

    #[test]
    fn reads_turns_like_the_importer() {
        let mut t = String::new();
        t += &line(json!({"type": "ai-title", "aiTitle": "Generated"}));
        t += &line(
            json!({"type": "user", "isMeta": true, "message": {"role": "user", "content": "meta"}}),
        );
        t += &user(json!("question"), "2026-11-10T10:00:00.000Z");
        t += &assistant(
            json!([{"type": "thinking", "thinking": "hmm", "signature": "s"}]),
            "2026-11-10T10:00:01.000Z",
        );
        t += &assistant(
            json!([{"type": "text", "text": "answer"}]),
            "2026-11-10T10:00:02.000Z",
        );
        t += &assistant(
            json!([{"type": "tool_use", "id": "t1", "name": "Bash", "input": {"command": "ls", "description": "list"}}]),
            "2026-11-10T10:00:03.000Z",
        );
        t += &user(
            json!([{"type": "tool_result", "tool_use_id": "t1", "content": "a\nb"}]),
            "2026-11-10T10:00:04.000Z",
        );
        t += &user(
            json!([{"type": "text", "text": "more"}, {"type": "image", "source": {}}]),
            "2026-11-10T10:00:05.000Z",
        );
        t += &line(
            json!({"type": "assistant", "isSidechain": true, "message": {"role": "assistant", "content": [{"type": "text", "text": "side"}]}}),
        );
        let p = parse(&t, "sid", 99);
        let s = p.session.unwrap();
        assert!(p.has_cwd && p.replica_of.is_none());
        assert_eq!(s.id, "sid");
        assert_eq!(s.cwd, PathBuf::from("/w/app"));
        assert_eq!(s.title, "Generated");
        let roles: Vec<_> = s.turns.iter().map(|t| t.role).collect();
        assert_eq!(
            roles,
            [Role::User, Role::Assistant, Role::Assistant, Role::User]
        );
        assert_eq!(s.turns[0].text, "question");
        assert_eq!(s.turns[1].text, "answer");
        assert_eq!(
            s.turns[2].text,
            "Tool: Bash\n```\nls\n```\nResult:\n```\na\nb\n```"
        );
        assert_eq!(
            s.turns[3].text,
            "more\n\n[external unsupported block: image]"
        );
        assert_eq!(
            s.created_ms,
            parse_iso_ms("2026-11-10T10:00:00.000Z").unwrap()
        );
        assert_eq!(s.updated_ms, 99);
    }

    #[test]
    fn title_priority_and_fallbacks() {
        let base = user(json!("first question"), "2026-11-10T10:00:00.000Z");
        let title_of = |extra: &str| {
            parse(&format!("{extra}{base}"), "s", 0)
                .session
                .unwrap()
                .title
        };
        let ai = line(json!({"type": "ai-title", "aiTitle": "AI"}));
        let custom = line(json!({"type": "custom-title", "customTitle": "Mine"}));
        let last = line(json!({"type": "last-prompt", "lastPrompt": "last words"}));
        assert_eq!(title_of(&format!("{ai}{custom}{last}")), "Mine");
        assert_eq!(title_of(&format!("{ai}{last}")), "AI");
        assert_eq!(title_of(&last), "last words");
        assert_eq!(title_of(""), "first question");
    }

    #[test]
    fn needs_a_user_turn_and_reports_missing_cwd() {
        let only_assistant = assistant(
            json!([{"type": "text", "text": "x"}]),
            "2026-11-10T10:00:00.000Z",
        );
        assert!(parse(&only_assistant, "s", 0).session.is_none());
        let no_cwd = line(json!({"type": "user", "message": {"role": "user", "content": "q"}}));
        let p = parse(&no_cwd, "s", 0);
        assert!(p.session.is_some() && !p.has_cwd);
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
                Turn::new(
                    Role::User,
                    "q\nwith lines",
                    parse_iso_ms("2026-11-10T10:00:00.000Z").unwrap(),
                ),
                Turn::new(
                    Role::Assistant,
                    "a1",
                    parse_iso_ms("2026-11-10T10:00:01.000Z").unwrap(),
                ),
                Turn::new(
                    Role::Assistant,
                    "a2",
                    parse_iso_ms("2026-11-10T10:00:02.000Z").unwrap(),
                ),
                Turn::new(
                    Role::User,
                    "q2",
                    parse_iso_ms("2026-11-10T10:00:03.000Z").unwrap(),
                ),
            ],
        };
        let key = ItemKey {
            origin: AgentId::new("pi"),
            native_id: "--w-app--/x.jsonl".into(),
        };
        let text = render(&session, &key, "new-id");
        let p = parse(&text, "new-id", 0);
        let back = p.session.unwrap();
        assert_eq!(p.replica_of, Some(key));
        assert_eq!(back.hash(), session.hash());
        assert_eq!(back.created_ms, session.created_ms);
        assert_eq!(
            back.turns.last().unwrap().ts_ms,
            session.turns.last().unwrap().ts_ms
        );
        let lines: Vec<Value> = text
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines[0]["type"], "custom-title");
        assert_eq!(lines[1]["parentUuid"], Value::Null);
        assert_eq!(lines[2]["parentUuid"], lines[1]["uuid"]);
        assert_eq!(lines.last().unwrap()["type"], "last-prompt");
        assert_eq!(lines.last().unwrap()["leafUuid"], lines[4]["uuid"]);
    }
}
