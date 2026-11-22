//! The paginated Codex conversation index. Native app-server appends the model messages first.
//! This writer appends their visible projection at the existing rollout offsets.

use std::fs;
use std::path::Path;

use rusqlite::{
    params, Connection, OpenFlags, OptionalExtension, Transaction, TransactionBehavior,
};
use serde_json::{json, Value};

use super::codex_app_server::APPENDED_MESSAGE_ID;
use super::codex_probes::state_databases;
use super::codex_sessions;
use super::sqlite_guard::{self, Stop, BUSY_TIMEOUT};
use crate::error::{Error, Result};
use crate::fsx::{self, BackupSession};
use crate::model::AgentId;
use crate::session::{format_iso_ms, parse_iso_ms, uuid_v7, MAX_SOURCE_BYTES};

const DB: &str = "thread_history_1.sqlite";
pub(super) const LEGACY_TURN_ID: &str = "dc51a74e-c486-8";
const TURNS: &[&str] = &[
    "thread_id",
    "turn_id",
    "rollout_ordinal",
    "status",
    "error_json",
    "started_at",
    "completed_at",
    "duration_ms",
    "first_user_item_id",
    "final_agent_item_id",
    "rollout_byte_offset",
    "rollout_end_ordinal",
    "rollout_end_byte_offset",
    "root_turn_id",
];
const ITEMS: &[&str] = &[
    "thread_id",
    "turn_id",
    "item_id",
    "rollout_ordinal",
    "created_at_ms",
    "item_json",
    "item_type",
    "updated_at_ordinal",
    "started_at_ms",
    "completed_at_ms",
];
const CURSOR: &[&str] = &[
    "thread_id",
    "next_rollout_byte_offset",
    "next_rollout_ordinal",
];

struct Entry {
    id: String,
    user: bool,
    time: i64,
    offset: i64,
    end: i64,
    ordinal: i64,
    item: Value,
}

impl Entry {
    fn text(&self) -> &str {
        let value = if self.user {
            &self.item["content"][0]["text"]
        } else {
            &self.item["text"]
        };
        value.as_str().expect("validated conversation item")
    }
}

fn unsupported(reason: &str) -> Error {
    Error::unsupported("codex", reason)
}

/// The original, native response records anchor the projected messages in rollout order.
fn turns(raw: &str) -> Result<Vec<Vec<Entry>>> {
    let mut offset = 0;
    let mut turns: Vec<Vec<Entry>> = Vec::new();
    let mut previous = None;
    for (ordinal, line) in raw.split_inclusive('\n').enumerate() {
        let start = offset;
        offset += line.len() as i64;
        if !line.contains(APPENDED_MESSAGE_ID) {
            continue;
        }
        let Ok(record) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let message = &record["payload"];
        let Some(id) = message["id"]
            .as_str()
            .filter(|id| id.starts_with(APPENDED_MESSAGE_ID))
        else {
            continue;
        };
        if record["type"] != "response_item" || message["type"] != "message" {
            continue;
        }
        let user = message["role"] == "user";
        let text = message["content"]
            .as_array()
            .filter(|parts| parts.len() == 1)
            .and_then(|parts| parts[0]["text"].as_str())
            .ok_or_else(|| unsupported("unrecognised appended history message"))?;
        let time = record["timestamp"]
            .as_str()
            .and_then(parse_iso_ms)
            .ok_or_else(|| unsupported("appended history has no timestamp"))?;
        let item = if user {
            json!({
                "type": "userMessage", "id": id, "clientId": null,
                "content": [{ "type": "text", "text": text, "text_elements": [] }],
            })
        } else if message["role"] == "assistant" {
            json!({
                "type": "agentMessage", "id": id, "text": text, "moderation": null,
                "phase": null, "memoryCitation": null, "delivery": null, "questions": [],
            })
        } else {
            return Err(unsupported("unrecognised appended history role"));
        };
        if user
            || previous != ordinal.checked_sub(1)
            || turns.is_empty()
            || id.as_bytes().get(APPENDED_MESSAGE_ID.len()) == Some(&b'0')
        {
            turns.push(Vec::new());
        }
        turns
            .last_mut()
            .expect("there is a current turn")
            .push(Entry {
                id: id.to_string(),
                user,
                time,
                offset: start,
                end: offset,
                ordinal: ordinal as i64,
                item,
            });
        previous = Some(ordinal);
    }
    Ok(turns)
}

fn validate_schema(conn: &Connection) -> std::result::Result<(), Stop> {
    for (name, expected) in [
        ("thread_turns", TURNS),
        ("thread_items", ITEMS),
        ("thread_history_projection_state", CURSOR),
    ] {
        let mut stmt =
            conn.prepare("SELECT name, type, pk FROM pragma_table_info(?1) ORDER BY cid")?;
        let fields = stmt
            .query_map([name], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, u32>(2)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let keys = match name {
            "thread_turns" => 2,
            "thread_items" => 3,
            _ => 1,
        };
        if fields.len() != expected.len()
            || fields
                .iter()
                .zip(expected)
                .enumerate()
                .any(|(i, ((column, kind, pk), wanted))| {
                    let text = matches!(
                        *wanted,
                        "thread_id"
                            | "turn_id"
                            | "item_id"
                            | "status"
                            | "error_json"
                            | "first_user_item_id"
                            | "final_agent_item_id"
                            | "root_turn_id"
                            | "item_json"
                            | "item_type"
                    );
                    column != wanted
                        || kind != if text { "TEXT" } else { "INTEGER" }
                        || *pk != if i < keys { i as u32 + 1 } else { 0 }
                })
        {
            return Err(unsupported("thread history index schema has changed").into());
        }
    }
    Ok(())
}

fn missing(conn: &Connection, thread: &str, entries: &[Entry]) -> std::result::Result<bool, Stop> {
    let first = &entries[0];
    let last = &entries[entries.len() - 1];
    let id = uuid_v7(first.time, &first.id);
    let found: i64 = conn.query_row(
        "SELECT count(*) FROM thread_turns WHERE thread_id = ?1 AND turn_id = ?2",
        params![thread, id],
        |row| row.get(0),
    )?;
    if found != 0 {
        let mut stmt = conn.prepare(
            "SELECT item_id, rollout_ordinal FROM thread_items WHERE thread_id = ?1 AND turn_id = ?2
             ORDER BY rollout_ordinal",
        )?;
        let items = stmt
            .query_map(params![thread, id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if items.len() != entries.len()
            || items
                .iter()
                .zip(entries)
                .any(|((item_id, ordinal), entry)| {
                    item_id != &entry.id || *ordinal != entry.ordinal
                })
        {
            return Err(unsupported("thread history has an incomplete projected turn").into());
        }
        return Ok(false);
    }
    let overlapped: i64 = conn.query_row(
        "SELECT count(*) FROM thread_turns WHERE thread_id = ?1 AND rollout_ordinal <= ?3
         AND coalesce(rollout_end_ordinal, rollout_ordinal) >= ?2",
        params![thread, first.ordinal, last.ordinal],
        |row| row.get(0),
    )?;
    if overlapped != 0 {
        return Err(unsupported("thread history already occupies these rollout offsets").into());
    }
    Ok(true)
}

fn insert_turn(
    tx: &Transaction<'_>,
    thread: &str,
    entries: &[Entry],
) -> std::result::Result<(), Stop> {
    let first = &entries[0];
    let last = &entries[entries.len() - 1];
    let id = uuid_v7(first.time, &first.id);
    let first_user = entries
        .iter()
        .find(|entry| entry.user)
        .map(|entry| &entry.id);
    let final_agent = entries
        .iter()
        .rev()
        .find(|entry| !entry.user)
        .map(|entry| &entry.id);
    tx.execute(
        "INSERT INTO thread_turns (thread_id,turn_id,rollout_ordinal,status,started_at,completed_at,
         duration_ms,first_user_item_id,final_agent_item_id,rollout_byte_offset,rollout_end_ordinal,
         rollout_end_byte_offset,root_turn_id) VALUES (?1,?2,?3,'completed',?4,?5,?6,?7,?8,?9,?10,?11,?2)",
        params![thread,id,first.ordinal,first.time.div_euclid(1000),last.time.div_euclid(1000),
            last.time.saturating_sub(first.time),first_user,final_agent,first.offset,last.ordinal,last.end],
    )?;
    for entry in entries {
        tx.execute(
            "INSERT INTO thread_items (thread_id,turn_id,item_id,rollout_ordinal,created_at_ms,item_json,
             item_type,updated_at_ordinal,started_at_ms,completed_at_ms)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?4,?5,?5)",
            params![thread,id,entry.id,entry.ordinal,entry.time,entry.item.to_string(),
                entry.item["type"].as_str()],
        )?;
    }
    Ok(())
}

fn sync_legacy(
    home: &Path,
    state_dir: &Path,
    backups: &BackupSession,
    path: &Path,
    raw: &str,
    turns: &[Vec<Entry>],
) -> Result<()> {
    let mut last_native = 0;
    let mut projected = std::collections::BTreeSet::new();
    for (ordinal, line) in raw.lines().enumerate() {
        if !line.contains("\"task_started\"") && !line.contains("\"task_complete\"") {
            continue;
        }
        let Ok(record) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if record["type"] != "event_msg" {
            continue;
        }
        let event = &record["payload"];
        let Some(id) = event["turn_id"].as_str() else {
            continue;
        };
        if id.starts_with(LEGACY_TURN_ID) && event["type"] == "task_complete" {
            projected.insert(id.to_string());
        } else if !id.starts_with(LEGACY_TURN_ID) && event["type"] == "task_started" {
            last_native = ordinal as i64;
        }
    }
    let mut suffix = String::new();
    for entries in turns {
        let first = &entries[0];
        let last = &entries[entries.len() - 1];
        let uuid = uuid_v7(first.time, &first.id);
        let id = format!("{LEGACY_TURN_ID}{}", &uuid[15..]);
        if projected.contains(&id) {
            continue;
        }
        if !raw.ends_with('\n') || !codex_sessions::is_idle(raw) || last_native > first.ordinal {
            return Err(unsupported(
                "legacy thread changed before its transcript could be projected",
            ));
        }
        let mut push = |time, event: Value| {
            suffix.push_str(
                &json!({
                    "timestamp": format_iso_ms(time), "type": "event_msg", "payload": event,
                })
                .to_string(),
            );
            suffix.push('\n');
        };
        let started = first.time.div_euclid(1000);
        push(
            first.time,
            json!({
                "type": "task_started", "turn_id": id, "started_at": started,
                "model_context_window": null, "collaboration_mode_kind": "default",
            }),
        );
        for entry in entries {
            let data = if entry.user {
                json!({
                    "type": "user_message", "message": entry.text(),
                    "local_images": [], "local_audio": [], "text_elements": [],
                })
            } else {
                json!({
                    "type": "agent_message", "message": entry.text(),
                    "phase": null, "memory_citation": null,
                })
            };
            push(entry.time, data);
        }
        push(
            last.time,
            json!({
                "type": "task_complete", "turn_id": id, "last_agent_message": null,
                "started_at": started, "completed_at": last.time.div_euclid(1000),
            }),
        );
    }
    if !suffix.is_empty() {
        if (raw.len() + suffix.len()) as u64 > MAX_SOURCE_BYTES {
            return Err(unsupported(
                "legacy transcript projection exceeds the rollout size limit",
            ));
        }
        fsx::backup(backups, state_dir, &AgentId::new("codex"), home, path)?;
        if !fsx::append_checked(path, raw.as_bytes(), suffix.as_bytes())? {
            return Err(unsupported(
                "legacy thread changed during transcript projection",
            ));
        }
    }
    Ok(())
}

pub(super) fn sync(
    home: &Path,
    state_dir: &Path,
    backups: &BackupSession,
    path: &Path,
    thread: &str,
    raw: &str,
) -> Result<()> {
    match sync_index(home, state_dir, backups, path, thread, raw) {
        Ok(()) => Ok(()),
        Err(Stop::Busy) => Err(unsupported("thread history index is busy; sync will retry")),
        Err(Stop::Failed(e)) => Err(e),
    }
}

fn sync_index(
    home: &Path,
    state_dir: &Path,
    backups: &BackupSession,
    path: &Path,
    thread: &str,
    raw: &str,
) -> std::result::Result<(), Stop> {
    let turns = turns(raw)?;
    if turns.is_empty() {
        return Ok(());
    }
    let Some((_, state_path)) = state_databases(home).into_iter().next() else {
        return Err(unsupported("Codex state database is missing").into());
    };
    let state = Connection::open_with_flags(state_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    state.busy_timeout(BUSY_TIMEOUT)?;
    let mode = state
        .query_row(
            "SELECT history_mode FROM threads WHERE id = ?1 AND archived = 0",
            [thread],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten();
    match mode.as_deref() {
        Some("paginated") => {}
        Some("legacy") => return Ok(sync_legacy(home, state_dir, backups, path, raw, &turns)?),
        _ => return Err(unsupported("unknown Codex thread history mode").into()),
    }
    if fs::read_dir(home)
        .map_err(|e| Error::io(home, e))?
        .flatten()
        .any(|entry| {
            let name = entry.file_name();
            name.to_str()
                .and_then(|name| name.strip_prefix("thread_history_"))
                .and_then(|name| name.strip_suffix(".sqlite"))
                .and_then(|version| version.parse::<u32>().ok())
                .is_some_and(|version| version > 1)
        })
    {
        return Err(unsupported("Codex thread history database version has changed").into());
    }
    let db = home.join(DB);
    if !fs::symlink_metadata(&db).is_ok_and(|meta| meta.is_file()) {
        return Err(unsupported("Codex thread history database is missing").into());
    }
    let mut conn = Connection::open_with_flags(&db, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    conn.busy_timeout(BUSY_TIMEOUT)?;
    validate_schema(&conn)?;
    let mut needs_projection = false;
    for entries in &turns {
        needs_projection |= missing(&conn, thread, entries)?;
    }
    if !needs_projection {
        return Ok(());
    }
    sqlite_guard::backup_database(
        &conn,
        &db,
        state_dir,
        backups,
        "codex",
        &|name| name == DB,
        BUSY_TIMEOUT,
    )?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let cursor = tx.query_row(
        "SELECT next_rollout_byte_offset,next_rollout_ordinal FROM thread_history_projection_state WHERE thread_id = ?1",
        [thread], |row| Ok((row.get::<_, i64>(0)?,row.get::<_, i64>(1)?)),
    ).optional()?;
    if cursor != Some((raw.len() as i64, raw.lines().count() as i64))
        || !codex_sessions::is_idle(raw)
        || !fs::read(path).is_ok_and(|bytes| bytes == raw.as_bytes())
    {
        return Err(
            unsupported("thread history changed before projection; sync will retry").into(),
        );
    }
    for entries in &turns {
        if missing(&tx, thread, entries)? {
            insert_turn(&tx, thread, entries)?;
        }
    }
    if !fs::read(path).is_ok_and(|bytes| bytes == raw.as_bytes()) {
        return Err(
            unsupported("thread history changed during projection; sync will retry").into(),
        );
    }
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const THREAD: &str = "session";
    const USER: &str = "msg_1d961050-4d1b-8012-8123-123456789abc";
    const ASSISTANT: &str = "msg_1d961050-4d1b-8113-8123-123456789abc";

    fn rollout() -> String {
        [
            ("session_meta", json!({"id": THREAD, "cwd": "/tmp/project"})),
            (
                "event_msg",
                json!({"type":"user_message", "message":"first"}),
            ),
            (
                "event_msg",
                json!({"type":"task_complete", "turn_id":"original"}),
            ),
            (
                "response_item",
                json!({"type":"message","role":"user","id":USER,
                "content":[{"type":"input_text","text":"new question"}]}),
            ),
            (
                "response_item",
                json!({"type":"message","role":"assistant","id":ASSISTANT,
                "content":[{"type":"output_text","text":"new answer"}]}),
            ),
        ]
        .map(|(kind, payload)| {
            format!(
                "{}\n",
                json!({
                    "type":kind, "timestamp":"2026-11-22T12:00:00.000Z", "payload":payload,
                })
            )
        })
        .concat()
    }

    fn create_state(path: &Path, mode: &str) {
        let state = Connection::open(path.join("state_5.sqlite")).unwrap();
        state
            .execute_batch(
                "CREATE TABLE threads (id TEXT PRIMARY KEY, history_mode TEXT, archived INTEGER);",
            )
            .unwrap();
        state
            .execute(
                "INSERT INTO threads VALUES (?1,?2,0)",
                params![THREAD, mode],
            )
            .unwrap();
    }

    fn create_history(path: &Path, raw: &str) -> Connection {
        let db = Connection::open(path.join(DB)).unwrap();
        db.execute_batch("
            CREATE TABLE thread_turns (thread_id TEXT, turn_id TEXT, rollout_ordinal INTEGER,
            status TEXT,error_json TEXT,started_at INTEGER,completed_at INTEGER,duration_ms INTEGER,
            first_user_item_id TEXT,final_agent_item_id TEXT,rollout_byte_offset INTEGER,
            rollout_end_ordinal INTEGER,rollout_end_byte_offset INTEGER,root_turn_id TEXT,
            PRIMARY KEY(thread_id,turn_id));
            CREATE TABLE thread_items (thread_id TEXT,turn_id TEXT,item_id TEXT,rollout_ordinal INTEGER,
            created_at_ms INTEGER,item_json TEXT,item_type TEXT,updated_at_ordinal INTEGER,
            started_at_ms INTEGER,completed_at_ms INTEGER,PRIMARY KEY(thread_id,turn_id,item_id));
            CREATE TABLE thread_history_projection_state (thread_id TEXT PRIMARY KEY,
            next_rollout_byte_offset INTEGER,next_rollout_ordinal INTEGER);
            INSERT INTO thread_turns(thread_id,turn_id,rollout_ordinal,status,rollout_end_ordinal)
            VALUES('session','original',1,'completed',2);
        ").unwrap();
        db.execute(
            "INSERT INTO thread_history_projection_state VALUES (?1,?2,?3)",
            params![THREAD, raw.len() as i64, raw.lines().count() as i64],
        )
        .unwrap();
        db
    }

    fn count(db: &Connection) -> i64 {
        db.query_row("SELECT count(*) FROM thread_turns", [], |row| row.get(0))
            .unwrap()
    }

    #[test]
    fn paginated_projection_retries_a_stale_cursor_and_inserts_only_once() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let state = home.join("sync");
        let raw = rollout();
        let path = home.join("session.jsonl");
        fs::write(&path, &raw).unwrap();
        create_state(home, "paginated");
        let db = create_history(home, &raw);
        db.execute(
            "UPDATE thread_history_projection_state SET next_rollout_ordinal=2",
            [],
        )
        .unwrap();
        let stamp = BackupSession::with_stamp("run");
        assert!(matches!(
            sync(home, &state, &stamp, &path, THREAD, &raw),
            Err(Error::Unsupported { .. })
        ));
        assert_eq!(count(&db), 1);
        db.execute(
            "UPDATE thread_history_projection_state SET next_rollout_ordinal=?1",
            [raw.lines().count()],
        )
        .unwrap();
        sync(home, &state, &stamp, &path, THREAD, &raw).unwrap();
        let backup = Connection::open(state.join("backups/run/codex").join(DB)).unwrap();
        assert_eq!(count(&backup), 1);
        assert_eq!(count(&db), 2);
        assert_eq!(fs::read_to_string(&path).unwrap(), raw);
        let visible: Vec<String> = db
            .prepare("SELECT item_id FROM thread_items ORDER BY rollout_ordinal")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap();
        assert_eq!(visible, [USER, ASSISTANT]);
        sync(
            home,
            &state,
            &BackupSession::with_stamp("retry"),
            &path,
            THREAD,
            &raw,
        )
        .unwrap();
        assert_eq!(count(&db), 2);
        assert!(!state.join("backups/retry").exists());
    }

    #[test]
    fn changed_history_schema_refuses_to_write() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path();
        let raw = rollout();
        let path = root.join("session.jsonl");
        fs::write(&path, &raw).unwrap();
        create_state(root, "paginated");
        let db = create_history(root, &raw);
        db.execute("ALTER TABLE thread_items ADD COLUMN unknown TEXT", [])
            .unwrap();
        let state = root.join("sync");
        assert!(matches!(
            sync(
                root,
                &state,
                &BackupSession::with_stamp("run"),
                &path,
                THREAD,
                &raw
            ),
            Err(Error::Unsupported { .. })
        ));
        assert_eq!(count(&db), 1);
        assert!(!state.exists());
    }

    #[test]
    fn legacy_events_are_visible_once_and_keep_the_model_history_unchanged() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path();
        let state = root.join("sync");
        let raw = rollout();
        let expected = codex_sessions::parse(&raw, THREAD, 0).unwrap().session;
        let path = root.join("session.jsonl");
        fs::write(&path, &raw).unwrap();
        create_state(root, "legacy");
        let stamp = BackupSession::with_stamp("run");
        sync(root, &state, &stamp, &path, THREAD, &raw).unwrap();
        let full = fs::read_to_string(&path).unwrap();
        assert!(full.starts_with(&raw));
        assert!(full[raw.len()..].contains("new answer"));
        assert_eq!(
            codex_sessions::parse(&full, THREAD, 0).unwrap().session,
            expected
        );
        sync(root, &state, &stamp, &path, THREAD, &full).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), full);
    }
}
