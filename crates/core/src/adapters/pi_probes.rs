//! Compatibility probes for Pi: session files `sessions/--<cwd>--/<timestamp>_<id>.jsonl` and the
//! installed package. The expected values are the formats `pi.rs` was written against.

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::compat::Check;
use crate::model::Kind;
use crate::probe::{
    json_lines, list, name_of, names, newest_files, read_head, read_tail, resolve_executable,
};

use super::pi::{encode_cwd, MARKER_TYPE};

const SOURCES: &[&str] = &[
    "crates/core/src/adapters/pi.rs",
    "crates/core/src/adapters/pi_probes.rs",
];

/// `type` of the first line of a session file and its `version`.
pub const HEADER_TYPE: &str = "session";
pub const HEADER_VERSION: i64 = 3;
/// Keys of the header the reader needs.
pub const REQUIRED_HEADER_KEYS: &[&str] = &["cwd", "id", "timestamp"];
/// Entry `type`s Pi writes after the header.
pub const KNOWN_ENTRY_TYPES: &[&str] = &[
    "branch_summary",
    "compaction",
    "custom",
    "custom_message",
    "label",
    "message",
    "model_change",
    "session_info",
    "thinking_level_change",
    "usage",
];

const TAIL_BYTES: u64 = 1 << 20;
const HEAD_BYTES: u64 = 64 << 10;
const SESSION_SAMPLE: usize = 3;
const DIR_SAMPLE: usize = 8;

fn is_session_file(p: &Path) -> bool {
    p.extension().is_some_and(|e| e == "jsonl")
}

fn newest_sessions(home: &Path, limit: usize) -> Vec<PathBuf> {
    newest_files(&home.join("sessions"), 1, &is_session_file, limit)
}

fn header(path: &Path) -> Option<Value> {
    json_lines(read_head(path, HEAD_BYTES)?.lines().next()?)
        .into_iter()
        .next()
}

/// The version of the installed package that holds the `pi` executable.
pub fn version(search_path: Option<&OsStr>) -> Option<String> {
    let exe = resolve_executable("pi", search_path)?;
    exe.ancestors().skip(1).find_map(|dir| {
        let doc: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("package.json")).ok()?).ok()?;
        doc.get("name")?;
        doc.get("version")?.as_str().map(str::to_string)
    })
}

fn session_header(home: &Path) -> Check {
    let check = Check::new(
        "pi.session_header",
        &[Kind::Session],
        "session header of the newest session",
        SOURCES,
    );
    let expected = format!(
        "type={HEADER_TYPE}, version={HEADER_VERSION}, keys {}",
        names(REQUIRED_HEADER_KEYS)
    );
    let Some(newest) = newest_sessions(home, 1).into_iter().next() else {
        return check.found(expected, "no sessions", true);
    };
    let h = header(&newest);
    let kind = h
        .as_ref()
        .and_then(|v| v.get("type")?.as_str().map(str::to_string))
        .unwrap_or_else(|| "none".into());
    let version = h.as_ref().and_then(|v| v.get("version")?.as_i64());
    let missing: BTreeSet<String> = REQUIRED_HEADER_KEYS
        .iter()
        .filter(|k| h.as_ref().is_none_or(|v| v.get(**k).is_none()))
        .map(|k| k.to_string())
        .collect();
    let ok = kind == HEADER_TYPE && version == Some(HEADER_VERSION) && missing.is_empty();
    check.found(
        expected,
        format!(
            "type={kind}, version={}, missing keys: {}",
            version.map_or("none".to_string(), |v| v.to_string()),
            list(&missing)
        ),
        ok,
    )
}

fn entry_types(home: &Path) -> Check {
    let known: BTreeSet<&str> = KNOWN_ENTRY_TYPES.iter().copied().collect();
    let unknown: BTreeSet<String> = newest_sessions(home, SESSION_SAMPLE)
        .iter()
        .filter_map(|p| read_tail(p, TAIL_BYTES))
        .flat_map(|t| json_lines(&t))
        .filter(|v| v.get("id").is_some() && v.get("parentId").is_some())
        .filter_map(|v| v.get("type")?.as_str().map(str::to_string))
        .filter(|t| !known.contains(t.as_str()))
        .collect();
    Check::new(
        "pi.entry_types",
        &[Kind::Session],
        "entry types of the newest sessions",
        SOURCES,
    )
    .found(
        format!(
            "only known types ({}); `custom` entries with customType {MARKER_TYPE} mark replicas",
            names(KNOWN_ENTRY_TYPES)
        ),
        format!("unknown types: {}", list(&unknown)),
        unknown.is_empty(),
    )
}

fn cwd_encoding(home: &Path) -> Check {
    let odd = newest_sessions(home, DIR_SAMPLE)
        .iter()
        .filter(|p| {
            let cwd = header(p).and_then(|h| h.get("cwd")?.as_str().map(str::to_string));
            let dir = p.parent().map(name_of).unwrap_or_default();
            cwd.is_some_and(|c| encode_cwd(&c) != dir)
        })
        .count();
    Check::new(
        "pi.cwd_encoding",
        &[Kind::Session],
        "session directory name is the working directory without its leading separator, with `/`, `\\` and `:` as `-`, wrapped in `--`",
        SOURCES,
    )
    .found(
        "0 sampled sessions sit outside encode(cwd)",
        format!("{odd} sampled sessions sit outside encode(cwd)"),
        odd == 0,
    )
}

pub fn probes(home: &Path) -> Vec<Check> {
    vec![session_header(home), entry_types(home), cwd_encoding(home)]
}
