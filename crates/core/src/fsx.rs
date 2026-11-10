use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::{Error, Result};
use crate::model::AgentId;

/// Write via a temp file in the same directory, fsync, rename. Creates parent dirs.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy())
        .unwrap_or_default();
    let tmp = dir.join(format!(".{file_name}.nativemem-tmp-{}", std::process::id()));
    let write = || -> std::io::Result<()> {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        fs::rename(&tmp, path)
    };
    write().map_err(|e| {
        let _ = fs::remove_file(&tmp);
        Error::io(path, e)
    })
}

/// One UTC timestamp shared by every backup made in a process run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupSession {
    stamp: String,
}

impl BackupSession {
    pub fn new() -> Self {
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Self::with_stamp(format_utc(secs))
    }

    pub fn with_stamp(stamp: impl Into<String>) -> Self {
        BackupSession {
            stamp: stamp.into(),
        }
    }

    pub fn stamp(&self) -> &str {
        &self.stamp
    }
}

impl Default for BackupSession {
    fn default() -> Self {
        Self::new()
    }
}

/// `YYYYMMDDTHHMMSSZ`, filesystem-safe on every platform.
fn format_utc(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Civil-from-days (Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}{m:02}{d:02}T{:02}{:02}{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// Copy `path` to `<state_dir>/backups/<stamp>/<agent>/<path relative to root>`.
pub fn backup(
    session: &BackupSession,
    state_dir: &Path,
    agent: &AgentId,
    root: &Path,
    path: &Path,
) -> Result<PathBuf> {
    let rel = path.strip_prefix(root).map_err(|_| {
        Error::io(
            path,
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("not under root {}", root.display()),
            ),
        )
    })?;
    let dest = state_dir
        .join("backups")
        .join(session.stamp())
        .join(agent.as_str())
        .join(rel);
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
    }
    fs::copy(path, &dest).map_err(|e| Error::io(path, e))?;
    Ok(dest)
}

/// Back up `path`, then delete it. Returns the backup location.
pub fn remove_with_backup(
    session: &BackupSession,
    state_dir: &Path,
    agent: &AgentId,
    root: &Path,
    path: &Path,
) -> Result<PathBuf> {
    let dest = backup(session, state_dir, agent, root, path)?;
    fs::remove_file(path).map_err(|e| Error::io(path, e))?;
    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_creates_parents_and_replaces() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("a/b/f.txt");
        atomic_write(&p, b"one").unwrap();
        atomic_write(&p, b"two").unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"two");
        let leftovers: Vec<_> = fs::read_dir(p.parent().unwrap()).unwrap().collect();
        assert_eq!(leftovers.len(), 1);
    }

    #[test]
    fn backup_layout() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("home");
        let state = t.path().join("state");
        let file = root.join("projects/p/memory/x.md");
        atomic_write(&file, b"hi").unwrap();
        let s = BackupSession::with_stamp("20260101T000000Z");
        let agent = AgentId::new("claude");
        let dest = backup(&s, &state, &agent, &root, &file).unwrap();
        assert_eq!(
            dest,
            state.join("backups/20260101T000000Z/claude/projects/p/memory/x.md")
        );
        assert_eq!(fs::read(&dest).unwrap(), b"hi");
        assert!(file.exists());
    }

    #[test]
    fn remove_with_backup_deletes_original() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("home");
        let file = root.join("x.md");
        atomic_write(&file, b"hi").unwrap();
        let s = BackupSession::new();
        let dest = remove_with_backup(
            &s,
            &t.path().join("state"),
            &AgentId::new("codex"),
            &root,
            &file,
        )
        .unwrap();
        assert!(!file.exists());
        assert_eq!(fs::read(dest).unwrap(), b"hi");
    }

    #[test]
    fn backup_outside_root_fails() {
        let t = tempfile::tempdir().unwrap();
        let f = t.path().join("x");
        atomic_write(&f, b"1").unwrap();
        let r = backup(
            &BackupSession::new(),
            t.path(),
            &AgentId::new("a"),
            &t.path().join("other"),
            &f,
        );
        assert!(r.is_err());
    }

    #[test]
    fn utc_format() {
        assert_eq!(format_utc(0), "19700101T000000Z");
        assert_eq!(format_utc(1_709_210_096), "20240229T123456Z");
    }
}
