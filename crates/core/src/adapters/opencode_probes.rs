//! Compatibility probes for OpenCode: its global config directory and `AGENTS.md`, which are
//! all this tool reads and writes. There is no database access.

use std::ffi::OsStr;
use std::path::{Component, Path};

use crate::compat::Check;
use crate::model::Kind;
use crate::probe::resolve_executable;

const SOURCES: &[&str] = &[
    "crates/core/src/adapters/agents_file.rs",
    "crates/core/src/adapters/sections.rs",
    "crates/core/src/adapters/opencode_probes.rs",
];

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
