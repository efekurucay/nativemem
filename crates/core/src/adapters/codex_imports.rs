//! Codex's importer ledger, `<codex_home>/external_agent_session_imports.json`: which source
//! sessions Codex already imported as which thread. Reading tells the session reader which
//! rollouts are imports; appending a record keeps Codex's importer from importing a session this
//! tool already wrote.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use crate::error::{Error, Result};
use crate::fsx::{self, BackupSession};
use crate::model::AgentId;
use crate::session::sha_hex;

const FILE: &str = "external_agent_session_imports.json";

/// One import record, reduced to what the sync needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub source_path: String,
    pub thread_id: String,
}

fn path(home: &Path) -> PathBuf {
    home.join(FILE)
}

fn load(home: &Path) -> Option<Value> {
    serde_json::from_str(&fs::read_to_string(path(home)).ok()?).ok()
}

/// Every record of the ledger; an absent or unreadable ledger has none.
pub fn read(home: &Path) -> Vec<Record> {
    let Some(doc) = load(home) else {
        return Vec::new();
    };
    doc.get("records")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|r| {
            Some(Record {
                source_path: r.get("source_path")?.as_str()?.to_string(),
                thread_id: r.get("imported_thread_id")?.as_str()?.to_string(),
            })
        })
        .collect()
}

/// The record Codex's importer writes for `source` (a Claude transcript) imported as
/// `thread_id`; `None` when the source file cannot be read.
pub fn record_for(source: &Path, thread_id: &str, title: &str) -> Option<Value> {
    let bytes = fs::read(source).ok()?;
    let modified_ns = fs::metadata(source)
        .ok()?
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_nanos() as u64;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    Some(json!({
        "source_path": source.to_string_lossy(),
        "content_sha256": sha_hex(&bytes),
        "imported_thread_id": thread_id,
        "imported_at": now,
        "source_modified_at": modified_ns,
        "connector_names": [],
        "title": title,
    }))
}

/// Adds `record`, replacing a record of the same thread. Every other key and record stays as
/// parsed; the file is created when absent, backed up before it changes, and left untouched when
/// the record is already there. An unparsable ledger is not overwritten.
pub fn upsert(home: &Path, state_dir: &Path, backups: &BackupSession, record: Value) -> Result<()> {
    let file = path(home);
    let mut doc = match fs::read_to_string(&file) {
        Ok(text) => serde_json::from_str::<Value>(&text)
            .map_err(|e| Error::Parse(format!("{}: {e}", file.display())))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            json!({ "records": [], "detected_connector_records": [] })
        }
        Err(e) => return Err(Error::io(&file, e)),
    };
    let thread = record.get("imported_thread_id").cloned();
    let records = doc
        .as_object_mut()
        .ok_or_else(|| Error::Parse(format!("{}: not an object", file.display())))?
        .entry("records")
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .ok_or_else(|| Error::Parse(format!("{}: records is not an array", file.display())))?;
    if records.contains(&record) {
        return Ok(());
    }
    records.retain(|r| r.get("imported_thread_id") != thread.as_ref());
    records.push(record);
    if file.exists() {
        fsx::backup(backups, state_dir, &AgentId::new("codex"), home, &file)?;
    }
    fsx::atomic_write(&file, doc.to_string().as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(thread: &str, sha: &str) -> Value {
        json!({"source_path": "/c/p/a.jsonl", "content_sha256": sha, "imported_thread_id": thread,
               "imported_at": 1, "source_modified_at": 2, "connector_names": [], "title": "t"})
    }

    #[test]
    fn creates_appends_replaces_and_preserves_other_keys() {
        let t = tempfile::tempdir().unwrap();
        let (home, state) = (t.path().join("codex"), t.path().join("state"));
        fs::create_dir_all(&home).unwrap();
        let b = BackupSession::with_stamp("T");
        assert!(read(&home).is_empty());
        upsert(&home, &state, &b, rec("t1", "a")).unwrap();
        assert!(!state.join("backups").exists(), "creating needs no backup");
        assert_eq!(
            read(&home),
            [Record {
                source_path: "/c/p/a.jsonl".into(),
                thread_id: "t1".into()
            }]
        );

        let mut doc: Value =
            serde_json::from_str(&fs::read_to_string(path(&home)).unwrap()).unwrap();
        doc["extra"] = json!({"keep": [1, 2]});
        doc["records"]
            .as_array_mut()
            .unwrap()
            .push(json!({"imported_thread_id": "other", "source_path": "/x", "odd": true}));
        fs::write(path(&home), doc.to_string()).unwrap();

        upsert(&home, &state, &b, rec("t1", "a")).unwrap();
        assert!(
            !state.join("backups").exists(),
            "an identical record changes nothing"
        );
        upsert(&home, &state, &b, rec("t1", "b")).unwrap();
        assert!(state
            .join("backups/T/codex/external_agent_session_imports.json")
            .is_file());
        let after: Value = serde_json::from_str(&fs::read_to_string(path(&home)).unwrap()).unwrap();
        assert_eq!(after["extra"], json!({"keep": [1, 2]}));
        let records = after["records"].as_array().unwrap();
        assert_eq!(records.len(), 2);
        assert!(records.iter().any(|r| r["odd"] == true));
        assert!(records.iter().any(|r| r["content_sha256"] == "b"));
    }

    #[test]
    fn unparsable_ledger_is_not_overwritten() {
        let t = tempfile::tempdir().unwrap();
        fs::write(path(t.path()), "{broken").unwrap();
        let r = upsert(
            t.path(),
            &t.path().join("s"),
            &BackupSession::with_stamp("T"),
            rec("t", "a"),
        );
        assert!(r.is_err());
        assert_eq!(fs::read_to_string(path(t.path())).unwrap(), "{broken");
    }

    #[test]
    fn record_hashes_the_raw_source_bytes() {
        let t = tempfile::tempdir().unwrap();
        let src = t.path().join("a.jsonl");
        fs::write(&src, "raw\n").unwrap();
        let r = record_for(&src, "tid", "T").unwrap();
        assert_eq!(r["content_sha256"], sha_hex(b"raw\n"));
        assert_eq!(r["imported_thread_id"], "tid");
        assert!(record_for(&t.path().join("missing"), "x", "T").is_none());
    }
}
