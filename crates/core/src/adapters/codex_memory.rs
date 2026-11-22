//! Projects selected by Codex's native memory importer. This adapter leaves the
//! resulting resource layout and consolidation to Codex.

use std::fs;
use std::path::{Component, Path};

use serde_json::{json, Value};

use super::codex_app_server::AppServer;
use crate::error::{Error, Result};
use crate::fsx::{self, BackupSession};
use crate::model::AgentId;

const IMPORT_ROOT: &str = "memories_v2/extensions/external_agent_import";
const LABEL: &str = "native project memory";
type Notes = Vec<(String, String)>;

fn malformed() -> Error {
    Error::unsupported("codex", "unrecognised native memory import response")
}

fn clean_source(projects: &Path, name: &str) -> Result<bool> {
    let mut components = Path::new(name).components();
    if !matches!(components.next(), Some(Component::Normal(_))) || components.next().is_some() {
        return Err(malformed());
    }
    let project = projects.join(name);
    let memory = project.join("memory");
    for dir in [&project, &memory] {
        match fs::symlink_metadata(dir) {
            Ok(meta) if !meta.is_dir() => return Ok(false),
            Ok(_) => {}
            // The native importer schedules removals for projects whose source vanished.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(true),
            Err(e) => return Err(Error::io(dir, e)),
        }
    }
    let mut directories = vec![memory];
    while let Some(dir) = directories.pop() {
        for entry in fs::read_dir(&dir).map_err(|e| Error::io(&dir, e))? {
            let entry = entry.map_err(|e| Error::io(&dir, e))?;
            let path = entry.path();
            let kind = entry.file_type().map_err(|e| Error::io(&path, e))?;
            if kind.is_dir() {
                directories.push(path);
                continue;
            }
            if !kind.is_file() {
                return Ok(false);
            }
            if path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
            {
                let text = fs::read_to_string(&path).map_err(|e| Error::io(&path, e))?;
                if text.lines().map(str::trim_start).any(|line| {
                    line.starts_with("nativemem_origin:") || line.starts_with("<!-- nativemem")
                }) {
                    return Ok(false);
                }
            }
        }
    }
    Ok(true)
}

fn select(projects: &Path, detected: &Value) -> Result<(Vec<Value>, Notes)> {
    let mut items = Vec::new();
    let mut notes = Vec::new();
    for item in detected["items"].as_array().ok_or_else(malformed)? {
        if item["itemType"] != "MEMORY" {
            continue;
        }
        let mut selected = Vec::new();
        for project in item["details"]["memory"].as_array().ok_or_else(malformed)? {
            let name = project.as_str().ok_or_else(malformed)?;
            if clean_source(projects, name)? {
                selected.push(project.clone());
            } else {
                notes.push((
                    name.to_string(),
                    "native memory source contains synchronization markers or links".into(),
                ));
            }
        }
        if !selected.is_empty() {
            let mut item = item.clone();
            item["details"] = json!({ "memory": selected });
            items.push(item);
        }
    }
    Ok((items, notes))
}

pub(super) fn sync(
    exe: &Path,
    codex: &Path,
    claude: &Path,
    state_dir: &Path,
    backups: &BackupSession,
) -> Result<Notes> {
    let projects = claude.join("projects");
    let imported = codex.join(IMPORT_ROOT);
    if !imported.exists()
        && !fs::read_dir(&projects)
            .into_iter()
            .flatten()
            .flatten()
            .any(|entry| entry.path().join("memory").is_dir())
    {
        return Ok(Vec::new());
    }
    let default = std::env::home_dir().map(|path| path.join(".claude"));
    if default.and_then(|path| path.canonicalize().ok()) != claude.canonicalize().ok() {
        return Ok(vec![(
            LABEL.into(),
            "Codex imports memory from the default Claude profile".into(),
        )]);
    }

    let mut server = AppServer::start_with_config(
        exe,
        codex,
        codex,
        &["features.external_agent_memory_import=true"],
    )?;
    let detected = server.call(
        "externalAgentConfig/detect",
        json!({
            "includeHome": true, "maxSessions": 0,
        }),
    )?;
    let (items, mut notes) = select(&projects, &detected)?;
    if !items.is_empty() {
        for dir in ["memories_v2", "memories_v2/extensions", IMPORT_ROOT] {
            let path = codex.join(dir);
            match fs::symlink_metadata(&path) {
                Ok(meta) if !meta.is_dir() => {
                    return Err(Error::unsupported(
                        "codex",
                        "native memory path is not a directory",
                    ));
                }
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
                Err(e) => return Err(Error::io(path, e)),
            }
        }
        if imported.is_dir() {
            fsx::backup_dir(backups, state_dir, &AgentId::new("codex"), codex, &imported)?;
        }
        let response = server.call(
            "externalAgentConfig/import",
            json!({ "migrationItems": items }),
        )?;
        let id = response["importId"].as_str().ok_or_else(malformed)?;
        let completed = server.wait_for_import(id)?;
        let results = completed["itemTypeResults"]
            .as_array()
            .ok_or_else(malformed)?;
        let mut memory = results
            .iter()
            .filter(|item| item["itemType"] == "MEMORY")
            .peekable();
        if memory.peek().is_none() {
            return Err(malformed());
        }
        for item in memory {
            let failures = item["failures"].as_array().ok_or_else(malformed)?.len();
            if failures > 0 {
                notes.push((
                    LABEL.into(),
                    format!("Codex reported {failures} memory import failures"),
                ));
            }
        }
    }
    server.close()?;
    Ok(notes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_selection_filters_marked_projects_and_keeps_removals() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("marked/memory/reference")).unwrap();
        fs::write(
            root.join("marked/memory/reference/note.md"),
            "  nativemem_origin: pi:example",
        )
        .unwrap();
        fs::create_dir_all(root.join("plain/memory")).unwrap();
        fs::write(root.join("plain/memory/NOTE.MD"), "A native project note.").unwrap();
        let detected = json!({"items": [
            {"itemType": "SESSIONS", "details": {"sessions": ["ignored"]}},
            {"itemType": "MEMORY", "description": "project notes", "cwd": null,
             "details": {"memory": ["marked", "plain", "removed"], "sessions":["ignored"]}}
        ]});
        let (items, notes) = select(root, &detected).unwrap();
        assert_eq!(
            items,
            [
                json!({"itemType": "MEMORY", "description": "project notes", "cwd": null,
            "details": {"memory": ["plain", "removed"]}})
            ]
        );
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].0, "marked");
    }

    #[test]
    fn native_selection_rejects_a_project_path_outside_the_source() {
        let tmp = tempfile::tempdir().unwrap();
        let detected =
            json!({"items":[{"itemType":"MEMORY", "details":{"memory":["../outside"]}}]});
        assert!(matches!(
            select(tmp.path(), &detected),
            Err(Error::Unsupported { .. })
        ));
    }
}
