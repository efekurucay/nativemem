//! Compatibility probes for Claude Code: transcripts (`projects/<dir>/<session>.jsonl`) and
//! project memory (`projects/<dir>/memory/*.md` with a `MEMORY.md` index). The expected values
//! are the formats `claude.rs` and `claude_sessions.rs` were written against; the line types are
//! the ones present in real transcripts at the time the adapter was written.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::compat::Check;
use crate::model::Kind;
use crate::probe::{
    json_lines, list, name_of, names, newest_files, parent_name, read_head, read_tail,
};

use super::claude::encode_name;
use super::claude_sessions::MARKER_KEY;

const SESSION_SOURCES: &[&str] = &[
    "crates/core/src/adapters/claude_sessions.rs",
    "crates/core/src/adapters/claude_probes.rs",
];
const MEMORY_SOURCES: &[&str] = &[
    "crates/core/src/adapters/claude.rs",
    "crates/core/src/adapters/claude_probes.rs",
];

/// Every top-level line `type` the adapter was written against: the ones it reads (`user`,
/// `assistant`, the title lines) and the ones it ignores.
pub const KNOWN_LINE_TYPES: &[&str] = &[
    "agent-name",
    "ai-title",
    "api-request",
    "api-request-blob",
    "api-request-shape",
    "assistant",
    "atis-latch",
    "attachment",
    "cost-state",
    "custom-title",
    "file-history-delta",
    "file-history-snapshot",
    "last-prompt",
    "mode",
    "permission-mode",
    "queue-operation",
    "relocated",
    "session-owner",
    "system",
    "tools-used",
    "user",
    "worktree-state",
];
/// Keys every user and assistant line carries (`message.content` is a nested key).
pub const REQUIRED_MESSAGE_KEYS: &[&str] = &[
    "cwd",
    "message",
    "message.content",
    "sessionId",
    "timestamp",
    "type",
    "uuid",
];
/// Top-level frontmatter keys of a memory file; `metadata` nests free-form keys such as `type`.
pub const KNOWN_TOP_KEYS: &[&str] = &["description", "metadata", "name", "originSessionId", "type"];
/// The shape of a `MEMORY.md` link line: `- [name](file.md) — description`.
pub const INDEX_LINE_SHAPE: &str = "- [name](file.md) — description";

const TAIL_BYTES: u64 = 1 << 20;
const HEAD_BYTES: u64 = 64 << 10;
const SESSION_SAMPLE: usize = 3;
const DIR_SAMPLE: usize = 8;
const MEMORY_SAMPLE: usize = 30;
/// Claude shortens longer directory names with a hash, so they say nothing about the rule.
const MAX_ENCODED_LEN: usize = 200;

fn is_session_file(p: &Path) -> bool {
    p.extension().is_some_and(|e| e == "jsonl")
}

fn is_memory_file(p: &Path) -> bool {
    p.extension().is_some_and(|e| e == "md")
        && name_of(p) != "MEMORY.md"
        && parent_name(p) == "memory"
}

/// Lines of the newest transcripts, newest file first; lines this tool wrote are left out.
fn session_samples(home: &Path) -> Vec<Vec<Value>> {
    newest_files(&home.join("projects"), 1, &is_session_file, SESSION_SAMPLE)
        .iter()
        .filter_map(|p| read_tail(p, TAIL_BYTES))
        .map(|text| {
            let own = |l: &&str| !l.contains(MARKER_KEY);
            json_lines(&text.lines().filter(own).collect::<Vec<_>>().join("\n"))
        })
        .collect()
}

/// The newest `version` found in the most recent transcript lines.
pub fn version(home: &Path) -> Option<String> {
    session_samples(home).iter().find_map(|lines| {
        lines
            .iter()
            .rev()
            .find_map(|v| v.get("version")?.as_str().map(str::to_string))
    })
}

fn has_key(v: &Value, key: &str) -> bool {
    match key.split_once('.') {
        Some((a, b)) => v.get(a).is_some_and(|inner| inner.get(b).is_some()),
        None => v.get(key).is_some(),
    }
}

fn line_types(samples: &[Vec<Value>]) -> Check {
    let known: BTreeSet<&str> = KNOWN_LINE_TYPES.iter().copied().collect();
    let unknown: BTreeSet<String> = samples
        .iter()
        .flatten()
        .filter_map(|v| v.get("type")?.as_str())
        .filter(|t| !known.contains(t))
        .map(str::to_string)
        .collect();
    Check::new(
        "claude.line_types",
        &[Kind::Session],
        "top-level transcript line types",
        SESSION_SOURCES,
    )
    .found(
        format!("only known types ({})", names(KNOWN_LINE_TYPES)),
        format!("unknown types: {}", list(&unknown)),
        unknown.is_empty(),
    )
}

fn message_keys(samples: &[Vec<Value>]) -> Check {
    let missing: BTreeSet<String> = samples
        .iter()
        .flatten()
        .filter(|v| {
            matches!(
                v.get("type").and_then(Value::as_str),
                Some("user" | "assistant")
            )
        })
        .flat_map(|v| {
            REQUIRED_MESSAGE_KEYS
                .iter()
                .filter(|k| !has_key(v, k))
                .map(|k| k.to_string())
                .collect::<Vec<_>>()
        })
        .collect();
    Check::new(
        "claude.message_keys",
        &[Kind::Session],
        "keys on user and assistant lines",
        SESSION_SOURCES,
    )
    .found(
        format!("every line has {}", names(REQUIRED_MESSAGE_KEYS)),
        format!("missing keys: {}", list(&missing)),
        missing.is_empty(),
    )
}

/// Keys of the frontmatter block of `text`: top-level ones and those nested under `metadata:`.
fn frontmatter_keys(text: &str) -> Option<(BTreeSet<String>, BTreeSet<String>)> {
    let mut lines = text.lines();
    if lines.next()?.trim_end() != "---" {
        return None;
    }
    let (mut top, mut nested) = (BTreeSet::new(), BTreeSet::new());
    let mut in_metadata = false;
    for l in lines {
        if l.trim_end() == "---" {
            return Some((top, nested));
        }
        if l.trim().is_empty() || l.trim_start().starts_with('#') {
            continue;
        }
        let key = l.trim_start().split(':').next().unwrap_or("").trim();
        if l.starts_with([' ', '\t']) {
            if in_metadata {
                nested.insert(key.to_string());
            }
        } else {
            in_metadata = l.trim_end() == "metadata:";
            top.insert(key.to_string());
        }
    }
    None
}

/// The adapter reads `name` and `description` at the top level and the category from
/// `metadata.type` or a top-level `type`. Other keys under `metadata` are free-form.
fn memory_frontmatter(files: &[PathBuf]) -> Check {
    let (mut unknown, mut incomplete, mut typed, mut parsed) = (BTreeSet::new(), 0, 0, 0);
    for f in files {
        let Some((top, nested)) = std::fs::read_to_string(f)
            .ok()
            .and_then(|text| frontmatter_keys(&text))
        else {
            continue;
        };
        parsed += 1;
        unknown.extend(
            top.iter()
                .filter(|k| !KNOWN_TOP_KEYS.contains(&k.as_str()))
                .cloned(),
        );
        incomplete += usize::from(!top.contains("name") || !top.contains("description"));
        typed += usize::from(top.contains("type") || nested.contains("type"));
    }
    let ok = unknown.is_empty() && incomplete == 0 && (parsed == 0 || typed > 0);
    Check::new(
        "claude.memory_frontmatter",
        &[Kind::Memory],
        "memory frontmatter keys (flat, or nested under metadata)",
        MEMORY_SOURCES,
    )
    .found(
        format!(
            "top-level keys within {}; name and description on every file; type under metadata or top-level",
            names(KNOWN_TOP_KEYS)
        ),
        format!(
            "unknown top-level: {}; files without name or description: {incomplete}; type: {}",
            list(&unknown),
            if parsed == 0 || typed > 0 { "found" } else { "not found" }
        ),
        ok,
    )
}

fn is_link_line(l: &str) -> bool {
    l.starts_with("- [") && l.contains("](")
}

fn has_index_shape(l: &str) -> bool {
    let Some((_, after)) = l.strip_prefix("- [").and_then(|r| r.split_once("](")) else {
        return false;
    };
    let Some((target, rest)) = after.split_once(')') else {
        return false;
    };
    target.ends_with(".md") && !target.contains(char::is_whitespace) && rest.starts_with(" — ")
}

fn memory_index(files: &[PathBuf]) -> Check {
    let dirs: BTreeSet<&Path> = files.iter().filter_map(|f| f.parent()).collect();
    let odd: usize = dirs
        .iter()
        .filter_map(|d| std::fs::read_to_string(d.join("MEMORY.md")).ok())
        .map(|text| {
            text.lines()
                .filter(|l| is_link_line(l) && !has_index_shape(l))
                .count()
        })
        .sum();
    Check::new(
        "claude.memory_index",
        &[Kind::Memory],
        "MEMORY.md link line shape",
        MEMORY_SOURCES,
    )
    .found(
        format!("every link line is `{INDEX_LINE_SHAPE}`"),
        format!("{odd} link lines differ"),
        odd == 0,
    )
}

/// The directory name is the encoded working directory, or encodes a directory below which the
/// session moved (the name then starts with the encoded working directory).
fn dir_matches(name: &str, cwds: &[String]) -> bool {
    cwds.iter().any(|cwd| {
        let enc = encode_name(cwd);
        name == enc || name.strip_prefix(&enc).is_some_and(|r| r.starts_with('-'))
    })
}

fn project_dir_encoding(home: &Path) -> Check {
    let files = newest_files(&home.join("projects"), 1, &is_session_file, DIR_SAMPLE);
    let dirs: BTreeSet<&Path> = files.iter().filter_map(|f| f.parent()).collect();
    let mut odd = 0;
    for dir in dirs {
        let name = name_of(dir);
        if name.len() >= MAX_ENCODED_LEN {
            continue;
        }
        let cwds: Vec<String> = files
            .iter()
            .filter(|f| f.parent() == Some(dir))
            .filter_map(|f| read_head(f, HEAD_BYTES))
            .flat_map(|text| {
                text.lines()
                    .filter(|l| !l.contains(MARKER_KEY))
                    .filter_map(|l| serde_json::from_str::<Value>(l).ok())
                    .filter_map(|v| v.get("cwd")?.as_str().map(str::to_string))
                    .collect::<Vec<_>>()
            })
            .collect();
        if !cwds.is_empty() && !dir_matches(&name, &cwds) {
            odd += 1;
        }
    }
    Check::new(
        "claude.project_dir_encoding",
        &[Kind::Memory, Kind::Session],
        "project directory name is the working directory with every non-alphanumeric character as `-`",
        &[
            "crates/core/src/adapters/claude.rs",
            "crates/core/src/adapters/claude_probes.rs",
        ],
    )
    .found(
        "0 sampled directories differ from encode(cwd)",
        format!("{odd} sampled directories differ"),
        odd == 0,
    )
}

pub fn probes(home: &Path) -> Vec<Check> {
    let samples = session_samples(home);
    let memory = newest_files(&home.join("projects"), 2, &is_memory_file, MEMORY_SAMPLE);
    vec![
        line_types(&samples),
        message_keys(&samples),
        memory_frontmatter(&memory),
        memory_index(&memory),
        project_dir_encoding(home),
    ]
}
