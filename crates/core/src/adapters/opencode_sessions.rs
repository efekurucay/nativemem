//! OpenCode chat sessions in its SQLite database, `<data home>/opencode.db`.
//!
//! Reading takes the root-level, non-archived rows of `session_v2` and their `session_message`
//! rows. Writing inserts one `session_v2` row, its messages and the `event_sequence` row per
//! replica (plus a `project` row when the directory has none), in one `BEGIN IMMEDIATE`
//! transaction. Rows OpenCode created are never changed or deleted; a refreshed replica rewrites
//! only its own rows, and rows it no longer needs become `idle` rows. Every row this tool writes
//! carries `MARKER_KEY` in its `data` JSON.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, ErrorCode, OpenFlags, OptionalExtension, TransactionBehavior};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::opencode_project::{self, Project};
use super::sqlite_guard::{self, Stop, BUSY_TIMEOUT};
use crate::error::{Error, Result};
use crate::fsx::BackupSession;
use crate::model::{AgentId, Item, ItemKey};
use crate::session::{
    clean_title, marker_text, parse_marker, result_content_text, title_from_turns,
    too_large_reason, Role, Session, SessionOptions, SessionScan, TurnBuilder, MAX_SOURCE_BYTES,
};

const AGENT: &str = "opencode";
pub const DATABASE: &str = "opencode.db";
/// Extra key inside the `data` JSON of every row this tool writes. OpenCode's own rows never
/// have it and its readers do not use it, so it survives a lost ledger and is never shown.
pub const MARKER_KEY: &str = "nativemem_origin";
/// Bump when the written rows change; unmodified replicas are then rewritten.
pub const SESSION_WRITER_REVISION: u32 = 1;
/// `version` of a replica when the installed OpenCode version cannot be found.
const OBSERVED_VERSION: &str = "2.2.6";
/// The agent every replica message names; OpenCode's default primary agent.
const AGENT_NAME: &str = "build";
/// `providerID` and `id` of the model replica messages name when no session names one.
const FALLBACK_MODEL: (&str, &str) = ("opencode", "big-pickle");
const ID_BITS: u64 = (1 << 48) - 1;
const ADJECTIVES: [&str; 16] = [
    "clever", "crisp", "curious", "eager", "jolly", "misty", "nimble", "proud", "quick", "quiet",
    "shiny", "silent", "sunny", "swift", "brave", "calm",
];
const NOUNS: [&str; 16] = [
    "cabin", "comet", "harbor", "meadow", "pixel", "river", "rocket", "signal", "sparrow", "tiger",
    "wizard", "garden", "canyon", "engine", "island", "planet",
];
const BASE62: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

pub struct OpencodeSessions {
    db: PathBuf,
    state_dir: PathBuf,
    backups: BackupSession,
    options: Option<SessionOptions>,
    busy_timeout: Duration,
    /// Set once the database was busy in this run, so the remaining writes do not wait again.
    busy: Cell<bool>,
}

struct Message {
    id: String,
    kind: &'static str,
    ts: i64,
    data: Value,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

fn is_busy(e: &rusqlite::Error) -> bool {
    matches!(
        e,
        rusqlite::Error::SqliteFailure(f, _)
            if matches!(f.code, ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked)
    )
}

fn failed(e: rusqlite::Error) -> Error {
    Error::Parse(format!("opencode database: {e}"))
}

/// 14 base62 characters derived from `seed`.
fn suffix(seed: &str) -> String {
    Sha256::digest(seed.as_bytes())
        .iter()
        .take(14)
        .map(|b| BASE62[*b as usize % 62] as char)
        .collect()
}

/// The 48-bit `(ms << 12) + counter` of OpenCode's ids.
fn id_clock(ms: i64, counter: usize) -> u64 {
    (((ms.max(0) as u64) << 12) + (counter as u64 & 0xfff)) & ID_BITS
}

/// OpenCode stores the complement for sessions, so newer sessions sort first.
fn session_id(created_ms: i64, key: &ItemKey) -> String {
    format!(
        "ses_{:012x}{}",
        !id_clock(created_ms, 0) & ID_BITS,
        suffix(&marker_text(key))
    )
}

fn message_id(session: &str, ts: i64, index: usize) -> String {
    format!(
        "msg_{:012x}{}",
        id_clock(ts, index),
        suffix(&format!("{session}:{index}"))
    )
}

fn slug(key: &ItemKey) -> String {
    let d = Sha256::digest(marker_text(key).as_bytes());
    format!(
        "{}-{}",
        ADJECTIVES[d[0] as usize % ADJECTIVES.len()],
        NOUNS[d[1] as usize % NOUNS.len()]
    )
}

/// The messages of a replica: one row per turn, an `idle` row after each assistant run like
/// OpenCode's own sessions have, and a final `idle` row at the session's last update.
fn messages(
    session: &Session,
    id: &str,
    marker: &str,
    model: &Value,
    updated_ms: i64,
) -> Vec<Message> {
    let mut rows: Vec<Message> = Vec::new();
    let mut clock = session.created_ms.min(updated_ms);
    let mut run_open = false;
    let push = |rows: &mut Vec<Message>, kind: &'static str, ts: i64, mut data: Value| {
        data[MARKER_KEY] = json!(marker);
        let id = message_id(id, ts, rows.len());
        rows.push(Message { id, kind, ts, data });
    };
    for turn in &session.turns {
        let ts = turn
            .ts_ms
            .clamp(session.created_ms.min(updated_ms), updated_ms)
            .max(clock);
        match turn.role {
            Role::User => {
                if run_open {
                    push(&mut rows, "idle", clock, idle(clock));
                    run_open = false;
                }
                push(
                    &mut rows,
                    "user",
                    ts,
                    json!({ "text": turn.text, "time": { "created": ts } }),
                );
            }
            Role::Assistant => {
                run_open = true;
                push(
                    &mut rows,
                    "assistant",
                    ts,
                    json!({
                        "agent": AGENT_NAME,
                        "model": model,
                        "content": [{ "type": "text", "text": turn.text }],
                        "finish": "stop",
                        "cost": 0,
                        "tokens": {
                            "input": 0, "output": 0, "reasoning": 0,
                            "cache": { "read": 0, "write": 0 }
                        },
                        "time": { "created": ts, "completed": ts },
                    }),
                );
            }
        }
        clock = ts;
    }
    push(
        &mut rows,
        "idle",
        updated_ms.max(clock),
        idle(updated_ms.max(clock)),
    );
    rows
}

fn idle(ts: i64) -> Value {
    json!({ "time": { "created": ts }, "outcome": "succeeded" })
}

/// The turns and replica marker of one session's `(type, data, time_created)` rows.
fn parse(rows: &[(String, String, i64)]) -> (Vec<crate::session::Turn>, Option<ItemKey>) {
    let mut b = TurnBuilder::default();
    let mut origin = None;
    for (i, (kind, data, row_ts)) in rows.iter().enumerate() {
        let Ok(v) = serde_json::from_str::<Value>(data) else {
            continue;
        };
        if i == 0 {
            origin = v
                .get(MARKER_KEY)
                .and_then(Value::as_str)
                .and_then(parse_marker);
        }
        let ts = v
            .pointer("/time/created")
            .and_then(Value::as_i64)
            .unwrap_or(*row_ts);
        match kind.as_str() {
            "user" => b.text(
                Role::User,
                v.get("text").and_then(Value::as_str).unwrap_or(""),
                ts,
            ),
            "assistant" => {
                for block in v
                    .get("content")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    let at = block
                        .pointer("/time/created")
                        .and_then(Value::as_i64)
                        .unwrap_or(ts);
                    match block.get("type").and_then(Value::as_str) {
                        Some("text") => b.text(
                            Role::Assistant,
                            block.get("text").and_then(Value::as_str).unwrap_or(""),
                            at,
                        ),
                        Some("tool") => tool(&mut b, block, at),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    (b.turns, origin)
}

fn tool(b: &mut TurnBuilder, block: &Value, ts: i64) {
    let id = block.get("id").and_then(Value::as_str);
    let name = block.get("name").and_then(Value::as_str).unwrap_or("tool");
    let state = block.get("state").unwrap_or(&Value::Null);
    b.call(id, name, state.get("input").unwrap_or(&Value::Null), ts);
    match state.get("status").and_then(Value::as_str) {
        Some("completed") => b.result(
            id,
            &result_content_text(state.get("content").unwrap_or(&Value::Null)),
            false,
            ts,
        ),
        Some("error") => b.result(
            id,
            state.get("error").and_then(Value::as_str).unwrap_or(""),
            true,
            ts,
        ),
        _ => {}
    }
}

impl OpencodeSessions {
    /// `options` is `None` while session sync is off: no transcript is read then.
    pub fn new(
        db: PathBuf,
        state_dir: PathBuf,
        backups: BackupSession,
        options: Option<SessionOptions>,
    ) -> Self {
        OpencodeSessions {
            db,
            state_dir,
            backups,
            options,
            busy_timeout: BUSY_TIMEOUT,
            busy: Cell::new(false),
        }
    }

    /// How long a locked database is waited for before the run skips it.
    pub fn with_busy_timeout(mut self, busy_timeout: Duration) -> Self {
        self.busy_timeout = busy_timeout;
        self
    }

    /// Whether session sync is on.
    pub fn is_enabled(&self) -> bool {
        self.options.is_some()
    }

    pub fn database(&self) -> &Path {
        &self.db
    }

    fn reader(&self) -> std::result::Result<Connection, rusqlite::Error> {
        let conn = Connection::open_with_flags(&self.db, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        conn.busy_timeout(self.busy_timeout)?;
        Ok(conn)
    }

    /// Project paths for skill discovery; never reads message rows or opens the database writable.
    pub fn project_dirs(&self) -> std::result::Result<Vec<PathBuf>, rusqlite::Error> {
        let conn = self.reader()?;
        conn.busy_timeout(Duration::ZERO)?;
        let mut statement =
            conn.prepare("SELECT DISTINCT directory FROM session_v2 WHERE time_archived IS NULL")?;
        let projects = statement
            .query_map([], |row| row.get::<_, String>(0).map(PathBuf::from))?
            .collect();
        projects
    }

    pub fn scan(&self, agent: &AgentId) -> Result<SessionScan> {
        let mut out = SessionScan::default();
        let Some(options) = self.options else {
            return Ok(out);
        };
        match self.read(agent, options, &mut out) {
            Ok(()) => Ok(out),
            Err(e) if is_busy(&e) => {
                out.skipped
                    .push((DATABASE.to_string(), "database is busy".to_string()));
                Ok(out)
            }
            Err(e) => Err(failed(e)),
        }
    }

    fn read(
        &self,
        agent: &AgentId,
        options: SessionOptions,
        out: &mut SessionScan,
    ) -> std::result::Result<(), rusqlite::Error> {
        let conn = self.reader()?;
        out.automated = conn.query_row(
            "SELECT count(*) FROM session_v2 WHERE parent_id IS NOT NULL AND time_archived IS NULL",
            [],
            |r| r.get::<_, i64>(0),
        )? as usize;
        let cutoff = options.days.map_or(i64::MIN, |days| {
            now_ms().saturating_sub(i64::from(days) * 86_400_000)
        });
        let mut sessions = conn.prepare(
            "SELECT id, directory, title, time_created, time_updated FROM session_v2 \
             WHERE parent_id IS NULL AND time_archived IS NULL AND time_updated >= ?1 ORDER BY id",
        )?;
        let found = sessions
            .query_map([cutoff], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, i64>(4)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut size = conn.prepare(
            "SELECT coalesce(sum(length(data)), 0) FROM session_message WHERE session_id = ?1",
        )?;
        let mut content = conn.prepare(
            "SELECT type, data, time_created FROM session_message WHERE session_id = ?1 ORDER BY seq",
        )?;
        for (id, directory, title, created_ms, updated_ms) in found {
            let native_id = format!("{id}.jsonl");
            let bytes: i64 = size.query_row([&id], |r| r.get(0))?;
            if bytes as u64 > MAX_SOURCE_BYTES {
                out.skipped
                    .push((native_id, too_large_reason(bytes as u64)));
                continue;
            }
            let rows = content
                .query_map([&id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            let (turns, origin) = parse(&rows);
            let title = match title.as_deref().filter(|t| !t.trim().is_empty()) {
                Some(t) => clean_title(t),
                None => title_from_turns(&turns),
            };
            let session = Session {
                id,
                cwd: PathBuf::from(directory),
                title,
                created_ms,
                updated_ms,
                turns,
            };
            if session.has_user_turn() && !session.cwd.as_os_str().is_empty() {
                out.observed
                    .push(session.observed(agent, native_id, self.db.clone(), origin));
            }
        }
        Ok(())
    }

    /// Creates or refreshes the replica of `key`: `(native id, database path)`. A busy database
    /// and a session this tool cannot place are reported as `Error::Unsupported`, which the
    /// engine turns into a note and retries on the next run.
    pub fn write(
        &self,
        key: &ItemKey,
        item: &Item,
        version: Option<&str>,
    ) -> Result<(String, PathBuf)> {
        let session = Session::from_body(&item.body)
            .ok_or_else(|| Error::Parse("session body is not a session".into()))?;
        if self.busy.get() {
            return Err(busy());
        }
        let project = opencode_project::resolve(&session.cwd).ok_or_else(|| {
            Error::unsupported(
                AGENT,
                "a session whose project id cannot be derived (Mercurial, or a git repository without origin or commits)",
            )
        })?;
        match self.write_rows(key, &session, &project, version.unwrap_or(OBSERVED_VERSION)) {
            Ok(id) => Ok((format!("{id}.jsonl"), self.db.clone())),
            Err(Stop::Busy) => {
                self.busy.set(true);
                Err(busy())
            }
            Err(Stop::Failed(e)) => Err(e),
        }
    }

    fn write_rows(
        &self,
        key: &ItemKey,
        session: &Session,
        project: &Project,
        version: &str,
    ) -> std::result::Result<String, Stop> {
        let mut conn = Connection::open_with_flags(&self.db, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        conn.busy_timeout(self.busy_timeout)?;
        conn.pragma_update(None, "foreign_keys", true)?;
        sqlite_guard::backup_database(
            &conn,
            &self.db,
            &self.state_dir,
            &self.backups,
            AGENT,
            &|name| name == DATABASE,
            self.busy_timeout,
        )?;
        let marker = marker_text(key);
        let updated_ms = session.updated_ms.max(session.created_ms);
        let created_ms = session.created_ms.min(updated_ms);
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing: Option<String> = tx
            .query_row(
                "SELECT id FROM session_v2 WHERE substr(id, 17) = ?1",
                [suffix(&marker)],
                |r| r.get(0),
            )
            .optional()?;
        let id = existing
            .clone()
            .unwrap_or_else(|| session_id(created_ms, key));
        if existing.is_some() {
            let foreign: i64 = tx.query_row(
                "SELECT count(*) FROM session_message WHERE session_id = ?1 \
                 AND coalesce(json_extract(data, ?2), '') <> ?3",
                params![id, format!("$.{MARKER_KEY}"), marker],
                |r| r.get(0),
            )?;
            if foreign > 0 {
                return Err(Stop::Failed(Error::unsupported(
                    AGENT,
                    "a replica that has rows this tool did not write",
                )));
            }
        }
        let model = replica_model(&tx)?;
        let rows = messages(session, &id, &marker, &model, updated_ms);
        let last_seq = rows.len() as i64 - 1;
        let now = now_ms();
        tx.execute(
            "INSERT OR IGNORE INTO project \
             (id, worktree, vcs, sandboxes, time_created, time_updated, time_active) \
             VALUES (?1, ?2, ?3, '[]', ?4, ?4, ?4)",
            params![
                project.id,
                project.worktree.to_string_lossy(),
                project.vcs,
                now
            ],
        )?;
        if existing.is_some() {
            tx.execute(
                "UPDATE session_v2 SET directory = ?2, path = ?3, title = ?4, time_created = ?5, \
                 time_updated = ?6, time_idle = ?6, time_busy = ?5, idle_outcome = 'succeeded' \
                 WHERE id = ?1",
                params![
                    id,
                    session.cwd.to_string_lossy(),
                    project.path,
                    session.title,
                    created_ms,
                    updated_ms
                ],
            )?;
        } else {
            tx.execute(
                "INSERT INTO session_v2 \
                 (id, project_id, slug, directory, path, title, version, agent, time_created, \
                  time_updated, time_idle, time_busy, idle_outcome) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10, ?9, 'succeeded')",
                params![
                    id,
                    project.id,
                    slug(key),
                    session.cwd.to_string_lossy(),
                    project.path,
                    session.title,
                    version,
                    AGENT_NAME,
                    created_ms,
                    updated_ms
                ],
            )?;
        }
        for (seq, m) in rows.iter().enumerate() {
            tx.execute(
                "INSERT INTO session_message (id, session_id, type, seq, time_created, time_updated, data) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6) \
                 ON CONFLICT(session_id, seq) DO UPDATE SET id = excluded.id, type = excluded.type, \
                 time_created = excluded.time_created, time_updated = excluded.time_updated, \
                 data = excluded.data",
                params![m.id, id, m.kind, seq as i64, m.ts, m.data.to_string()],
            )?;
        }
        let mut spare = idle(updated_ms);
        spare[MARKER_KEY] = json!(marker);
        tx.execute(
            "UPDATE session_message SET type = 'idle', data = ?3 WHERE session_id = ?1 AND seq > ?2",
            params![id, last_seq, spare.to_string()],
        )?;
        tx.execute(
            "INSERT INTO event_sequence (aggregate_id, seq) VALUES (?1, ?2) \
             ON CONFLICT(aggregate_id) DO UPDATE SET seq = max(seq, excluded.seq)",
            params![id, last_seq],
        )?;
        tx.commit()?;
        verify(&conn, &id, &marker, session, project, rows.len())?;
        Ok(id)
    }
}

fn busy() -> Error {
    Error::unsupported(
        AGENT,
        "the database is busy; this session follows on the next run",
    )
}

/// The model of the most recently updated session that names one: a model the user's own
/// OpenCode resolves. Replica messages must name some model; a replica's session row names none.
fn replica_model(conn: &Connection) -> std::result::Result<Value, rusqlite::Error> {
    let text: Option<String> = conn
        .query_row(
            "SELECT model FROM session_v2 WHERE model IS NOT NULL ORDER BY time_updated DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .optional()?;
    let named = text
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| {
            Some((
                v.get("providerID")?.as_str()?.to_string(),
                v.get("id")?.as_str()?.to_string(),
            ))
        })
        .unwrap_or((FALLBACK_MODEL.0.into(), FALLBACK_MODEL.1.into()));
    Ok(json!({ "providerID": named.0, "id": named.1 }))
}

/// Reads the rows back and checks them against what was written.
fn verify(
    conn: &Connection,
    id: &str,
    marker: &str,
    session: &Session,
    project: &Project,
    count: usize,
) -> std::result::Result<(), Stop> {
    let row: Option<(String, String, String, i64)> = conn
        .query_row(
            "SELECT directory, path, title, time_updated FROM session_v2 WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    let marked: i64 = conn.query_row(
        "SELECT count(*) FROM session_message WHERE session_id = ?1 AND seq < ?2 \
         AND json_extract(data, ?3) = ?4",
        params![id, count as i64, format!("$.{MARKER_KEY}"), marker],
        |r| r.get(0),
    )?;
    let seq: Option<i64> = conn
        .query_row(
            "SELECT seq FROM event_sequence WHERE aggregate_id = ?1",
            [id],
            |r| r.get(0),
        )
        .optional()?;
    let project_row: i64 = conn.query_row(
        "SELECT count(*) FROM project WHERE id = ?1",
        [&project.id],
        |r| r.get(0),
    )?;
    let ok = row.is_some_and(|r| {
        r.0 == session.cwd.to_string_lossy()
            && r.1 == project.path
            && r.2 == session.title
            && r.3 == session.updated_ms.max(session.created_ms)
    }) && marked == count as i64
        && seq.is_some_and(|s| s >= count as i64 - 1)
        && project_row == 1;
    if ok {
        Ok(())
    } else {
        Err(Stop::Failed(Error::Parse(format!(
            "session {id} does not match its source after the write"
        ))))
    }
}
