//! The `threads` row of Codex's state database for every replica rollout this tool wrote. Codex's
//! resume list is driven by that table, so a rollout without a row is never listed. A row mirrors
//! what Codex's own importer inserts for an imported Claude thread and is derived from the rollout
//! file itself, so the file and the row cannot disagree and a rollout that lost its row (a skipped
//! run, or a rollout written before this writer existed) gets it back on the next run.
//!
//! Rules: only threads whose rollout carries `nativemem_origin` get a row; a row is inserted or, for
//! a rollout rewritten in this run, refreshed; Codex's own rows are never touched and no row is
//! ever deleted. A row that exists keeps everything Codex or the user changed (archive flag, pin,
//! name, token counts): a refresh sets only the columns derived from the rollout.

use std::collections::BTreeSet;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, UNIX_EPOCH};

use rusqlite::backup::{Backup, StepResult};
use rusqlite::{params, Connection, ErrorCode, OpenFlags, OptionalExtension, TransactionBehavior};
use serde_json::Value;

use super::codex_probes::state_databases;
use super::codex_rollout::{CLI_VERSION, HISTORY_MODE, MODEL_PROVIDER, ORIGIN_KEY, SOURCE};
use super::codex_sessions::{collect_rollouts, parse, thread_id_of};
use crate::error::{Error, Result};
use crate::fsx::BackupSession;
use crate::session::Role;

/// How long a database that another process holds locked is waited for before the run skips it.
pub const BUSY_TIMEOUT: Duration = Duration::from_secs(5);
/// Database backups kept below `<state dir>/backups`; older ones are deleted.
pub const KEPT_BACKUPS: usize = 5;

/// What Codex's importer writes for an imported thread.
const SANDBOX_POLICY: &str = r#"{"type":"read-only"}"#;
const APPROVAL_MODE: &str = "on-request";

/// The columns of a row that come from the rollout; every other column keeps its default.
struct ThreadRow {
    id: String,
    rollout_path: String,
    cwd: String,
    title: String,
    first_user_message: String,
    /// First and last message time, in seconds.
    created_at: i64,
    updated_at: i64,
}

struct Candidate {
    row: ThreadRow,
    /// The rollout was written in this run, so its row is refreshed even when it exists.
    refresh: bool,
}

enum Stop {
    Busy,
    Failed(Error),
}

impl From<rusqlite::Error> for Stop {
    fn from(e: rusqlite::Error) -> Self {
        match &e {
            rusqlite::Error::SqliteFailure(f, _)
                if matches!(f.code, ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) =>
            {
                Stop::Busy
            }
            _ => Stop::Failed(Error::Parse(format!("codex state database: {e}"))),
        }
    }
}

impl From<Error> for Stop {
    fn from(e: Error) -> Self {
        Stop::Failed(e)
    }
}

/// First and last message time of a rollout, from its turn events.
fn message_times(text: &str) -> (Option<i64>, Option<i64>) {
    let (mut first, mut last) = (None::<i64>, None::<i64>);
    for line in text.lines() {
        let started = line.contains("\"task_started\"");
        if !started && !line.contains("\"task_complete\"") {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let payload = v.get("payload").unwrap_or(&Value::Null);
        if started {
            if let Some(s) = payload.get("started_at").and_then(Value::as_i64) {
                first = Some(first.map_or(s, |f| f.min(s)));
            }
        } else if let Some(c) = payload.get("completed_at").and_then(Value::as_i64) {
            last = Some(last.map_or(c, |l| l.max(c)));
        }
    }
    (first, last)
}

/// The row for the replica rollout at `path`; `None` when it is not a readable replica.
fn thread_row(path: &Path) -> Option<ThreadRow> {
    let text = fs::read_to_string(path).ok()?;
    let mtime_ms = fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_millis() as i64;
    let stem = path.file_stem()?.to_string_lossy();
    let parsed = parse(&text, thread_id_of(&stem), mtime_ms)?;
    parsed.origin.as_ref()?;
    if !parsed.has_cwd {
        return None;
    }
    let session = parsed.session;
    let first_user_message = session
        .turns
        .iter()
        .find(|t| t.role == Role::User)?
        .text
        .clone();
    let (started, completed) = message_times(&text);
    let created_at = started.unwrap_or(session.created_ms / 1000);
    Some(ThreadRow {
        id: session.id,
        rollout_path: path.to_string_lossy().into_owned(),
        cwd: session.cwd.to_string_lossy().into_owned(),
        title: session.title,
        first_user_message,
        created_at,
        updated_at: completed.unwrap_or(mtime_ms / 1000).max(created_at),
    })
}

fn carries_origin_marker(path: &Path) -> bool {
    let Ok(file) = fs::File::open(path) else {
        return false;
    };
    let mut first = String::new();
    BufReader::new(file).read_line(&mut first).is_ok() && first.contains(ORIGIN_KEY)
}

/// Every replica rollout this tool wrote: `written` in this run plus the marked ones on disk.
fn candidates(home: &Path, written: &[PathBuf]) -> Vec<Candidate> {
    let mut files = Vec::new();
    let _ = collect_rollouts(&home.join("sessions"), 0, &mut files);
    let marked: BTreeSet<PathBuf> = files
        .into_iter()
        .filter(|p| carries_origin_marker(p))
        .collect();
    let refreshed: BTreeSet<&PathBuf> = written.iter().collect();
    marked
        .iter()
        .chain(refreshed.iter().copied())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter_map(|p| {
            Some(Candidate {
                row: thread_row(p)?,
                refresh: refreshed.contains(p),
            })
        })
        .collect()
}

/// `refresh` replaces the derived columns of an existing row. Without it an existing row is only
/// advanced when the rollout is newer than the row, which a resume in Codex never makes true.
fn upsert_sql(refresh: bool) -> String {
    let guard = if refresh {
        ""
    } else {
        " WHERE threads.updated_at < excluded.updated_at"
    };
    format!(
        "INSERT INTO threads (id, rollout_path, created_at, updated_at, source, model_provider, \
         cwd, title, sandbox_policy, approval_mode, tokens_used, has_user_event, archived, \
         cli_version, first_user_message, created_at_ms, updated_at_ms, preview, recency_at, \
         recency_at_ms, history_mode, name) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 0, 0, 0, ?11, ?12, ?3 * 1000, \
         ?4 * 1000, ?12, ?4, ?4 * 1000, ?13, ?8) \
         ON CONFLICT(id) DO UPDATE SET rollout_path = excluded.rollout_path, \
         cwd = excluded.cwd, title = excluded.title, \
         first_user_message = excluded.first_user_message, preview = excluded.preview, \
         created_at = excluded.created_at, created_at_ms = excluded.created_at_ms, \
         updated_at = excluded.updated_at, updated_at_ms = excluded.updated_at_ms, \
         recency_at = excluded.recency_at, recency_at_ms = excluded.recency_at_ms, \
         cli_version = excluded.cli_version, history_mode = excluded.history_mode{guard}"
    )
}

fn db_backups(state_dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(stamps) = fs::read_dir(state_dir.join("backups")) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for stamp in stamps.flatten() {
        let dir = stamp.path().join("codex");
        for file in fs::read_dir(&dir).into_iter().flatten().flatten() {
            let name = file.file_name().to_string_lossy().into_owned();
            if name.starts_with("state_") && name.ends_with(".sqlite") {
                found.push((
                    stamp.file_name().to_string_lossy().into_owned(),
                    file.path(),
                ));
            }
        }
    }
    found.sort();
    found
}

/// Deletes all but the newest `KEPT_BACKUPS` database backup copies, and nothing else.
fn prune_backups(state_dir: &Path) {
    let found = db_backups(state_dir);
    for (_, file) in found.iter().take(found.len().saturating_sub(KEPT_BACKUPS)) {
        let _ = fs::remove_file(file);
        if let Some(dir) = file.parent() {
            let _ = fs::remove_dir(dir);
            if let Some(stamp) = dir.parent() {
                let _ = fs::remove_dir(stamp);
            }
        }
    }
}

/// A consistent copy of the database through SQLite's online backup, once per run.
fn backup_database(
    source: &Connection,
    db: &Path,
    state_dir: &Path,
    session: &BackupSession,
    busy: Duration,
) -> std::result::Result<(), Stop> {
    let name = db.file_name().map(PathBuf::from).unwrap_or_default();
    let dir = state_dir
        .join("backups")
        .join(session.stamp())
        .join("codex");
    let dest = dir.join(name);
    if dest.exists() {
        return Ok(());
    }
    fs::create_dir_all(&dir).map_err(|e| Error::io(&dir, e))?;
    let copy = || -> std::result::Result<(), Stop> {
        let mut target = Connection::open(&dest)?;
        let backup = Backup::new(source, &mut target)?;
        let deadline = Instant::now() + busy;
        loop {
            match backup.step(256)? {
                StepResult::Done => return Ok(()),
                StepResult::More => {}
                _ if Instant::now() >= deadline => return Err(Stop::Busy),
                _ => std::thread::sleep(Duration::from_millis(25)),
            }
        }
    };
    if let Err(e) = copy() {
        let _ = fs::remove_file(&dest);
        return Err(e);
    }
    prune_backups(state_dir);
    Ok(())
}

/// Reads the row back and checks it against the rollout.
fn verify(conn: &Connection, row: &ThreadRow) -> std::result::Result<(), Stop> {
    let stored = conn
        .query_row(
            "SELECT rollout_path, cwd, title, created_at, updated_at, first_user_message, preview \
             FROM threads WHERE id = ?1",
            [&row.id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, i64>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?,
                ))
            },
        )
        .optional()?;
    let ok = stored.is_some_and(|s| {
        s.0 == row.rollout_path
            && s.1 == row.cwd
            && s.2 == row.title
            && s.3 == row.created_at
            && s.4 >= row.updated_at
            && s.5 == row.first_user_message
            && s.6 == row.first_user_message
    });
    if ok {
        Ok(())
    } else {
        Err(Stop::Failed(Error::Parse(format!(
            "thread row {} does not match its rollout after the write",
            row.id
        ))))
    }
}

fn write_rows(
    db: &Path,
    state_dir: &Path,
    session: &BackupSession,
    candidates: &[Candidate],
    busy: Duration,
) -> std::result::Result<(), Stop> {
    let reader = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    reader.busy_timeout(busy)?;
    let mut work: Vec<&Candidate> = Vec::new();
    for c in candidates {
        let stored: Option<i64> = reader
            .query_row(
                "SELECT updated_at FROM threads WHERE id = ?1",
                [&c.row.id],
                |r| r.get(0),
            )
            .optional()?;
        if c.refresh || stored.is_none_or(|u| u < c.row.updated_at) {
            work.push(c);
        }
    }
    drop(reader);
    if work.is_empty() {
        return Ok(());
    }

    let mut conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    conn.busy_timeout(busy)?;
    backup_database(&conn, db, state_dir, session, busy)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    {
        let mut refresh = tx.prepare(&upsert_sql(true))?;
        let mut advance = tx.prepare(&upsert_sql(false))?;
        for c in &work {
            let r = &c.row;
            let stmt = if c.refresh {
                &mut refresh
            } else {
                &mut advance
            };
            stmt.execute(params![
                r.id,
                r.rollout_path,
                r.created_at,
                r.updated_at,
                SOURCE,
                MODEL_PROVIDER,
                r.cwd,
                r.title,
                SANDBOX_POLICY,
                APPROVAL_MODE,
                CLI_VERSION,
                r.first_user_message,
                HISTORY_MODE,
            ])?;
        }
    }
    tx.commit()?;
    for c in &work {
        verify(&conn, &c.row)?;
    }
    Ok(())
}

/// Brings the `threads` rows of this tool's replica rollouts in line with the files: rows missing
/// are inserted, rows of rollouts in `written` are refreshed. `Ok(Some(reason))` means the database
/// was busy and the run left it alone; the next run does the same work. A home without a state
/// database has nothing to write to.
pub fn sync_rows(
    home: &Path,
    state_dir: &Path,
    session: &BackupSession,
    written: &[PathBuf],
    busy: Duration,
) -> Result<Option<String>> {
    let Some((_, db)) = state_databases(home).into_iter().next() else {
        return Ok(None);
    };
    let candidates = candidates(home, written);
    if candidates.is_empty() {
        return Ok(None);
    }
    match write_rows(&db, state_dir, session, &candidates, busy) {
        Ok(()) => Ok(None),
        Err(Stop::Busy) => Ok(Some(
            "state database is busy, thread rows follow on the next run".to_string(),
        )),
        Err(Stop::Failed(e)) => Err(e),
    }
}
