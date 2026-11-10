//! Compatibility probes for OpenCode: its global config directory and `AGENTS.md`, and, when its
//! database exists, the tables and the installed version the session reader and writer were
//! written against.

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::path::{Component, Path};
use std::time::Duration;

use rusqlite::{Connection, OpenFlags};

use crate::compat::Check;
use crate::model::Kind;
use crate::probe::{list, resolve_executable};
use crate::session::sha_hex;

const SOURCES: &[&str] = &[
    "crates/core/src/adapters/agents_file.rs",
    "crates/core/src/adapters/sections.rs",
    "crates/core/src/adapters/opencode_probes.rs",
];

const SESSION_SOURCES: &[&str] = &[
    "crates/core/src/adapters/opencode_sessions.rs",
    "crates/core/src/adapters/opencode_project.rs",
    "crates/core/src/adapters/opencode_probes.rs",
];

/// `table.name:type:notnull:default:primary key` of every column of the tables the session
/// reader and writer use: `session_v2`, `session_message`, `project` and `event_sequence`.
pub const SESSION_COLUMNS: &[&str] = &[
    "event_sequence.aggregate_id:TEXT:0::1",
    "event_sequence.seq:INTEGER:1::0",
    "event_sequence.owner_id:TEXT:0::0",
    "project.id:TEXT:0::1",
    "project.worktree:TEXT:1::0",
    "project.vcs:TEXT:0::0",
    "project.name:TEXT:0::0",
    "project.icon_url:TEXT:0::0",
    "project.icon_color:TEXT:0::0",
    "project.time_created:INTEGER:1::0",
    "project.time_updated:INTEGER:1::0",
    "project.time_initialized:INTEGER:0::0",
    "project.sandboxes:TEXT:1::0",
    "project.commands:TEXT:0::0",
    "project.icon_url_override:TEXT:0::0",
    "project.time_active:INTEGER:1:0:0",
    "project.time_archived:INTEGER:0::0",
    "session_message.id:TEXT:0::1",
    "session_message.session_id:TEXT:1::0",
    "session_message.type:TEXT:1::0",
    "session_message.seq:INTEGER:1::0",
    "session_message.time_created:INTEGER:1::0",
    "session_message.time_updated:INTEGER:1::0",
    "session_message.data:TEXT:1::0",
    "session_v2.id:TEXT:0::1",
    "session_v2.project_id:TEXT:1::0",
    "session_v2.workspace_id:TEXT:0::0",
    "session_v2.parent_id:TEXT:0::0",
    "session_v2.fork_session_id:TEXT:0::0",
    "session_v2.fork_boundary:TEXT:0::0",
    "session_v2.slug:TEXT:1::0",
    "session_v2.directory:TEXT:1::0",
    "session_v2.path:TEXT:0::0",
    "session_v2.title:TEXT:0::0",
    "session_v2.version:TEXT:1::0",
    "session_v2.share_url:TEXT:0::0",
    "session_v2.summary_additions:INTEGER:0::0",
    "session_v2.summary_deletions:INTEGER:0::0",
    "session_v2.summary_files:INTEGER:0::0",
    "session_v2.summary_diffs:TEXT:0::0",
    "session_v2.metadata:TEXT:0::0",
    "session_v2.cost:REAL:1:0:0",
    "session_v2.tokens_input:INTEGER:1:0:0",
    "session_v2.tokens_output:INTEGER:1:0:0",
    "session_v2.tokens_reasoning:INTEGER:1:0:0",
    "session_v2.tokens_cache_read:INTEGER:1:0:0",
    "session_v2.tokens_cache_write:INTEGER:1:0:0",
    "session_v2.revert:TEXT:0::0",
    "session_v2.permission:TEXT:0::0",
    "session_v2.agent:TEXT:0::0",
    "session_v2.model:TEXT:0::0",
    "session_v2.time_created:INTEGER:1::0",
    "session_v2.time_updated:INTEGER:1::0",
    "session_v2.time_compacting:INTEGER:0::0",
    "session_v2.time_archived:INTEGER:0::0",
    "session_v2.time_suspended:INTEGER:0::0",
    "session_v2.resume_attempts:INTEGER:1:0:0",
    "session_v2.time_idle:INTEGER:0::0",
    "session_v2.time_viewed:INTEGER:0::0",
    "session_v2.idle_outcome:TEXT:0::0",
    "session_v2.time_busy:INTEGER:0::0",
];
/// The major version of OpenCode whose database layout the session adapter was written against.
pub const SUPPORTED_MAJOR: &str = "2";

const PROBE_BUSY: Duration = Duration::from_secs(2);

/// The version in the `Cellar/opencode/<version>` component of the resolved executable path.
pub fn version(search_path: Option<&OsStr>) -> Option<String> {
    let exe = resolve_executable("opencode", search_path)?;
    let parts: Vec<String> = exe
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    parts
        .windows(3)
        .find(|w| w[0] == "Cellar" && w[1] == "opencode")
        .map(|w| w[2].clone())
}

fn config_dir(home: &Path) -> Check {
    let observed = if home.is_dir() {
        "directory"
    } else {
        "not a directory"
    };
    Check::new(
        "opencode.config_dir",
        &[Kind::Instruction, Kind::Memory],
        "global config directory",
        SOURCES,
    )
    .found("directory", observed, home.is_dir())
}

fn agents_file(home: &Path) -> Check {
    let path = home.join("AGENTS.md");
    let observed = if !path.exists() {
        "absent"
    } else if !path.is_file() {
        "not a regular file"
    } else if std::fs::read_to_string(&path).is_err() {
        "not UTF-8 text"
    } else {
        "UTF-8 text file"
    };
    Check::new(
        "opencode.agents_md",
        &[Kind::Instruction, Kind::Memory],
        "AGENTS.md in the global config directory",
        SOURCES,
    )
    .found(
        "absent or a UTF-8 text file",
        observed,
        matches!(observed, "absent" | "UTF-8 text file"),
    )
}

pub fn probes(home: &Path) -> Vec<Check> {
    vec![config_dir(home), agents_file(home)]
}

fn signature(cols: &[String]) -> String {
    format!(
        "{} columns, sha256 {}",
        cols.len(),
        &sha_hex(cols.join("\n").as_bytes())[..12]
    )
}

/// Sorted `table.name:type:notnull:default:pk` lines of the session tables. Read-only, and not
/// `immutable`: a migration OpenCode just ran may still sit in the write-ahead log.
fn session_columns(db: &Path) -> Result<Vec<String>, rusqlite::Error> {
    let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    conn.busy_timeout(PROBE_BUSY)?;
    let mut cols = Vec::new();
    for table in ["event_sequence", "project", "session_message", "session_v2"] {
        let mut stmt = conn
            .prepare("SELECT name, type, \"notnull\", dflt_value, pk FROM pragma_table_info(?1)")?;
        let rows = stmt.query_map([table], |r| {
            Ok(format!(
                "{table}.{}:{}:{}:{}:{}",
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, Option<String>>(3)?.unwrap_or_default(),
                r.get::<_, i64>(4)?
            ))
        })?;
        for row in rows {
            cols.push(row?);
        }
    }
    cols.sort();
    Ok(cols)
}

fn session_schema(db: &Path) -> Check {
    let mut expected: Vec<String> = SESSION_COLUMNS.iter().map(|c| c.to_string()).collect();
    expected.sort();
    let check = Check::new(
        "opencode.session_schema",
        &[Kind::Session],
        "session_v2, session_message, project and event_sequence column signature (name, type, not null, default, primary key)",
        SESSION_SOURCES,
    );
    let want = signature(&expected);
    let cols = match session_columns(db) {
        Ok(cols) => cols,
        Err(rusqlite::Error::SqliteFailure(f, _))
            if matches!(
                f.code,
                rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
            ) =>
        {
            return check.found(want, "database is busy, not checked", true);
        }
        Err(_) => return check.found(want, "no readable database", false),
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
    check.found(want, observed, ok)
}

fn session_version(version: Option<&str>) -> Check {
    let observed = version.unwrap_or("unknown");
    Check::new(
        "opencode.version",
        &[Kind::Session],
        "installed OpenCode major version",
        SESSION_SOURCES,
    )
    .found(
        format!("{SUPPORTED_MAJOR}.x"),
        observed,
        version.is_none_or(|v| v.split('.').next() == Some(SUPPORTED_MAJOR)),
    )
}

/// The session store checks; they run only while the database exists.
pub fn session_probes(db: &Path, version: Option<&str>) -> Vec<Check> {
    vec![session_schema(db), session_version(version)]
}
