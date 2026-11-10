//! What every write into an agent-owned SQLite database shares: a bounded wait for locks, the
//! busy-versus-failed distinction, and one consistent copy per run made with SQLite's online
//! backup API before the first write, of which only the newest `KEPT_BACKUPS` are kept.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rusqlite::backup::{Backup, StepResult};
use rusqlite::{Connection, ErrorCode};

use crate::error::Error;
use crate::fsx::BackupSession;

/// How long a database that another process holds locked is waited for before the run skips it.
pub const BUSY_TIMEOUT: Duration = Duration::from_secs(5);
/// Database backups kept below `<state dir>/backups` per agent; older ones are deleted.
pub const KEPT_BACKUPS: usize = 5;

pub enum Stop {
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
            _ => Stop::Failed(Error::Parse(format!("sqlite database: {e}"))),
        }
    }
}

impl From<Error> for Stop {
    fn from(e: Error) -> Self {
        Stop::Failed(e)
    }
}

/// `(stamp, path)` of every backup copy of `agent_dir` whose file name satisfies `is_copy`,
/// oldest first.
fn db_backups(
    state_dir: &Path,
    agent_dir: &str,
    is_copy: &dyn Fn(&str) -> bool,
) -> Vec<(String, PathBuf)> {
    let Ok(stamps) = fs::read_dir(state_dir.join("backups")) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for stamp in stamps.flatten() {
        let dir = stamp.path().join(agent_dir);
        for file in fs::read_dir(&dir).into_iter().flatten().flatten() {
            let name = file.file_name().to_string_lossy().into_owned();
            if is_copy(&name) {
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
fn prune_backups(state_dir: &Path, agent_dir: &str, is_copy: &dyn Fn(&str) -> bool) {
    let found = db_backups(state_dir, agent_dir, is_copy);
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

/// A consistent copy of `db` through SQLite's online backup to
/// `<state dir>/backups/<stamp>/<agent_dir>/<file name>`, once per run.
pub fn backup_database(
    source: &Connection,
    db: &Path,
    state_dir: &Path,
    session: &BackupSession,
    agent_dir: &str,
    is_copy: &dyn Fn(&str) -> bool,
    busy: Duration,
) -> std::result::Result<(), Stop> {
    let name = db.file_name().map(PathBuf::from).unwrap_or_default();
    let dir = state_dir
        .join("backups")
        .join(session.stamp())
        .join(agent_dir);
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
    prune_backups(state_dir, agent_dir, is_copy);
    Ok(())
}
