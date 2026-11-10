//! Compatibility probes for Codex: the state database, rollouts and the import ledger. The
//! expected values are the formats `codex_sessions.rs`, `codex_rollout.rs` and
//! `codex_imports.rs` were written against.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};
use serde_json::Value;

use crate::compat::Check;
use crate::model::Kind;
use crate::probe::{json_lines, list, name_of, names, read_head};
use crate::session::sha_hex;

const SOURCES: &[&str] = &[
    "crates/core/src/adapters/codex_sessions.rs",
    "crates/core/src/adapters/codex_rollout.rs",
    "crates/core/src/adapters/codex_imports.rs",
    "crates/core/src/adapters/codex_probes.rs",
];

/// The number of the `state_<n>.sqlite` database the adapter was written against.
pub const SUPPORTED_STATE_DB: u32 = 5;
/// `name:type:notnull:default` of every column of that database's `threads` table.
pub const THREADS_COLUMNS: &[&str] = &[
    "id:TEXT:0:",
    "rollout_path:TEXT:1:",
    "created_at:INTEGER:1:",
    "updated_at:INTEGER:1:",
    "source:TEXT:1:",
    "model_provider:TEXT:1:",
    "cwd:TEXT:1:",
    "title:TEXT:1:",
    "sandbox_policy:TEXT:1:",
    "approval_mode:TEXT:1:",
    "tokens_used:INTEGER:1:0",
    "has_user_event:INTEGER:1:0",
    "archived:INTEGER:1:0",
    "archived_at:INTEGER:0:",
    "git_sha:TEXT:0:",
    "git_branch:TEXT:0:",
    "git_origin_url:TEXT:0:",
    "cli_version:TEXT:1:''",
    "first_user_message:TEXT:1:''",
    "agent_nickname:TEXT:0:",
    "agent_role:TEXT:0:",
    "memory_mode:TEXT:1:'enabled'",
    "model:TEXT:0:",
    "reasoning_effort:TEXT:0:",
    "agent_path:TEXT:0:",
    "created_at_ms:INTEGER:0:",
    "updated_at_ms:INTEGER:0:",
    "thread_source:TEXT:0:",
    "preview:TEXT:1:''",
    "recency_at:INTEGER:1:0",
    "recency_at_ms:INTEGER:1:0",
    "history_mode:TEXT:1:'legacy'",
    "name:TEXT:0:",
    "is_pinned:INTEGER:1:0",
    "thread_section_id:TEXT:0:",
    "section_position:INTEGER:0:",
    "section_entered_at_ms:INTEGER:0:",
    "project_id:TEXT:0:",
    "originator:TEXT:0:",
    "daybreak_enabled:BOOLEAN:0:",
    "creator_user_id:TEXT:0:",
    "creator_account_id:TEXT:0:",
    "attached_thread_parent_id:TEXT:0:",
];
/// Keys of the first rollout line's `payload` that the session reader needs.
pub const REQUIRED_META_KEYS: &[&str] = &["cwd", "id", "source", "timestamp"];
/// Keys of every `records[]` entry of the import ledger, which a written record carries too.
pub const LEDGER_RECORD_KEYS: &[&str] = &[
    "connector_names",
    "content_sha256",
    "imported_at",
    "imported_thread_id",
    "source_modified_at",
    "source_path",
    "title",
];
pub const ROLLOUT_LAYOUT: &str = "YYYY/MM/DD/rollout-YYYY-MM-DDTHH-MM-SS-<36 character id>.jsonl";

const LEDGER_FILE: &str = "external_agent_session_imports.json";
const ROLLOUT_SAMPLE: usize = 50;
const HEAD_BYTES: u64 = 256 << 10;
const MAX_DEPTH: usize = 4;

/// `rollout-*.jsonl` files below `dir`, newest date path first, at most `limit`.
fn newest_rollouts(dir: &Path, depth: usize, limit: usize, out: &mut Vec<PathBuf>) {
    if out.len() >= limit {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    paths.sort_by(|a, b| b.cmp(a));
    for p in paths {
        if out.len() >= limit {
            return;
        }
        if p.is_dir() {
            if depth < MAX_DEPTH {
                newest_rollouts(&p, depth + 1, limit, out);
            }
        } else if p.is_file() {
            let name = name_of(&p);
            if name.starts_with("rollout-") && name.ends_with(".jsonl") {
                out.push(p);
            }
        }
    }
}

fn rollouts(home: &Path, limit: usize) -> Vec<PathBuf> {
    let mut out = Vec::new();
    newest_rollouts(&home.join("sessions"), 0, limit, &mut out);
    out
}

fn first_line(path: &Path) -> Option<Value> {
    json_lines(read_head(path, HEAD_BYTES)?.lines().next()?)
        .into_iter()
        .next()
}

pub fn version(home: &Path) -> Option<String> {
    let from_file = std::fs::read_to_string(home.join("version.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.get("latest_version")?.as_str().map(str::to_string));
    from_file.or_else(|| {
        let newest = rollouts(home, 1).into_iter().next()?;
        first_line(&newest)?
            .get("payload")?
            .get("cli_version")?
            .as_str()
            .map(str::to_string)
    })
}

/// `state_<n>.sqlite` files of `home` as `(n, path)`, highest first.
fn state_databases(home: &Path) -> Vec<(u32, PathBuf)> {
    let mut found: Vec<(u32, PathBuf)> = std::fs::read_dir(home)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let n = name.strip_prefix("state_")?.strip_suffix(".sqlite")?;
            Some((n.parse::<u32>().ok()?, e.path()))
        })
        .collect();
    found.sort_by(|a, b| b.0.cmp(&a.0));
    found
}

fn state_db_number(home: &Path) -> Check {
    let highest = state_databases(home).first().map(|(n, _)| *n);
    let observed = highest.map_or("no state database".to_string(), |n| n.to_string());
    Check::new(
        "codex.state_db",
        &[Kind::Session],
        "highest state_<n>.sqlite number",
        SOURCES,
    )
    .found(
        SUPPORTED_STATE_DB.to_string(),
        observed,
        highest.is_none_or(|n| n == SUPPORTED_STATE_DB),
    )
}

/// Percent-encodes `path` for a SQLite `file:` URI.
fn file_uri(path: &Path) -> String {
    let mut out = String::from("file:");
    for b in path.to_string_lossy().bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'/' | b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out.push_str("?immutable=1");
    out
}

/// Sorted `name:type:notnull:default` lines of the `threads` table, read without ever writing.
fn threads_columns(db: &Path) -> Option<Vec<String>> {
    let conn = Connection::open_with_flags(
        file_uri(db),
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )
    .ok()?;
    let mut stmt = conn.prepare("PRAGMA table_info(threads)").ok()?;
    let mut cols: Vec<String> = stmt
        .query_map([], |r| {
            Ok(format!(
                "{}:{}:{}:{}",
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, Option<String>>(4)?.unwrap_or_default()
            ))
        })
        .ok()?
        .flatten()
        .collect();
    cols.sort();
    Some(cols)
}

fn signature(cols: &[String]) -> String {
    format!(
        "{} columns, sha256 {}",
        cols.len(),
        &sha_hex(cols.join("\n").as_bytes())[..12]
    )
}

fn threads_signature(home: &Path) -> Check {
    let mut expected: Vec<String> = THREADS_COLUMNS.iter().map(|c| c.to_string()).collect();
    expected.sort();
    let check = Check::new(
        "codex.threads_columns",
        &[Kind::Session],
        "threads table column signature (name, type, not null, default)",
        SOURCES,
    );
    let Some((_, db)) = state_databases(home).into_iter().next() else {
        return check.found(signature(&expected), "no state database", true);
    };
    let Some(cols) = threads_columns(&db).filter(|c| !c.is_empty()) else {
        return check.found(signature(&expected), "no readable threads table", false);
    };
    let diff = |a: &[String], b: &[String]| -> BTreeSet<String> {
        a.iter()
            .filter(|c| !b.contains(c))
            .map(|c| c.split(':').next().unwrap_or("").to_string())
            .collect()
    };
    let (added, removed) = (diff(&cols, &expected), diff(&expected, &cols));
    let ok = cols == expected;
    let observed = if ok {
        signature(&cols)
    } else {
        format!(
            "{}; added: {}; removed or changed: {}",
            signature(&cols),
            list(&added),
            list(&removed)
        )
    };
    check.found(signature(&expected), observed, ok)
}

fn session_meta_keys(home: &Path) -> Check {
    let check = Check::new(
        "codex.session_meta_keys",
        &[Kind::Session],
        "session_meta payload keys of the newest rollout",
        SOURCES,
    );
    let expected = format!(
        "first line is session_meta with keys {}",
        names(REQUIRED_META_KEYS)
    );
    let Some(newest) = rollouts(home, 1).into_iter().next() else {
        return check.found(expected, "no rollouts", true);
    };
    let line = first_line(&newest);
    let is_meta = line
        .as_ref()
        .and_then(|v| v.get("type")?.as_str())
        .is_some_and(|t| t == "session_meta");
    let payload = line.as_ref().and_then(|v| v.get("payload"));
    let missing: BTreeSet<String> = REQUIRED_META_KEYS
        .iter()
        .filter(|k| payload.is_none_or(|p| p.get(**k).is_none()))
        .map(|k| k.to_string())
        .collect();
    let observed = if is_meta {
        format!("missing keys: {}", list(&missing))
    } else {
        "first line is not session_meta".to_string()
    };
    check.found(expected, observed, is_meta && missing.is_empty())
}

fn ledger_keys(home: &Path) -> Check {
    let check = Check::new(
        "codex.import_ledger_keys",
        &[Kind::Session],
        "import ledger record keys",
        SOURCES,
    );
    let expected = format!("records[] keys: {}", names(LEDGER_RECORD_KEYS));
    let Some(text) = std::fs::read_to_string(home.join(LEDGER_FILE)).ok() else {
        return check.found(expected, "no import ledger", true);
    };
    let records = serde_json::from_str::<Value>(&text)
        .ok()
        .and_then(|v| v.get("records").cloned())
        .and_then(|r| r.as_array().cloned());
    let Some(records) = records else {
        return check.found(expected, "records is not an array", false);
    };
    let known: BTreeSet<&str> = LEDGER_RECORD_KEYS.iter().copied().collect();
    let (mut added, mut removed) = (BTreeSet::new(), BTreeSet::new());
    for r in &records {
        let keys: BTreeSet<&str> = r
            .as_object()
            .map(|o| o.keys().map(String::as_str).collect())
            .unwrap_or_default();
        added.extend(keys.difference(&known).map(|k| k.to_string()));
        removed.extend(known.difference(&keys).map(|k| k.to_string()));
    }
    let ok = added.is_empty() && removed.is_empty();
    check.found(
        expected,
        format!(
            "added keys: {}; missing keys: {}",
            list(&added),
            list(&removed)
        ),
        ok,
    )
}

fn digits(s: &str, n: usize) -> bool {
    s.len() == n && s.bytes().all(|b| b.is_ascii_digit())
}

/// Whether `rel` (below `sessions/`) is `YYYY/MM/DD/rollout-YYYY-MM-DDTHH-MM-SS-<id>.jsonl`.
fn is_rollout_layout(rel: &str) -> bool {
    let parts: Vec<&str> = rel.split('/').collect();
    let [y, m, d, file] = parts[..] else {
        return false;
    };
    let Some(stem) = file
        .strip_prefix("rollout-")
        .and_then(|f| f.strip_suffix(".jsonl"))
    else {
        return false;
    };
    // `YYYY-MM-DDTHH-MM-SS` is 19 characters, then `-` and the 36 character thread id.
    let (Some(stamp), Some(id)) = (stem.get(..19), stem.get(19..)) else {
        return false;
    };
    let t: Vec<&str> = stamp.split(['-', 'T']).collect();
    digits(y, 4)
        && digits(m, 2)
        && digits(d, 2)
        && t.len() == 6
        && [4, 2, 2, 2, 2, 2]
            .iter()
            .zip(&t)
            .all(|(n, p)| digits(p, *n))
        && id.len() == 37
        && id.starts_with('-')
}

fn rollout_layout(home: &Path) -> Check {
    let root = home.join("sessions");
    let odd = rollouts(home, ROLLOUT_SAMPLE)
        .iter()
        .filter(|p| {
            let rel = p
                .strip_prefix(&root)
                .unwrap_or(p)
                .to_string_lossy()
                .replace('\\', "/");
            !is_rollout_layout(&rel)
        })
        .count();
    Check::new(
        "codex.rollout_layout",
        &[Kind::Session],
        "rollout path layout below sessions/",
        SOURCES,
    )
    .found(
        format!("sessions/{ROLLOUT_LAYOUT}"),
        format!("{odd} sampled rollouts differ"),
        odd == 0,
    )
}

pub fn probes(home: &Path) -> Vec<Check> {
    vec![
        state_db_number(home),
        threads_signature(home),
        session_meta_keys(home),
        ledger_keys(home),
        rollout_layout(home),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_accepts_codex_paths_and_rejects_others() {
        let id = "01a1c682-fd13-7263-8871-4ca47590d7d1";
        assert!(is_rollout_layout(&format!(
            "2026/11/10/rollout-2026-11-10T20-30-36-{id}.jsonl"
        )));
        assert!(!is_rollout_layout(&format!(
            "2026/11/rollout-2026-11-10T20-30-36-{id}.jsonl"
        )));
        assert!(!is_rollout_layout(&format!(
            "2026/11/10/rollout-20261110T203036-{id}.jsonl"
        )));
        assert!(!is_rollout_layout(
            "2026/11/10/rollout-2026-11-10T20-30-36-short.jsonl"
        ));
    }
}
