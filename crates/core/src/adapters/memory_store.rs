//! Project-memory replica store shared by the agents that have no native memory format of
//! their own to write: item files plus a per-project `MEMORY.md` index and `scope.json` below a
//! base directory, and a routing block in the agent's global `AGENTS.md` that points the agent
//! at the index of the project it is working in.

use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::error::{Error, Result};
use crate::fsx::{self, BackupSession};
use crate::model::{AgentId, Item, ItemKey, Kind, Observed, Scope};
use crate::slug::slugify;

use super::sections::{self, block_span, region_span, BLOCK_BEGIN, BLOCK_END};

pub const INDEX_FILE: &str = "MEMORY.md";
const SCOPE_FILE: &str = "scope.json";

pub struct MemoryStore {
    agent: AgentId,
    home: PathBuf,
    state_dir: PathBuf,
    backups: BackupSession,
    base: PathBuf,
    keep: PathBuf,
    agents_file: PathBuf,
}

pub struct Replica {
    pub origin: String,
    pub id: String,
    pub name: String,
    pub description: String,
    pub category: Option<String>,
    pub body: String,
}

fn take_line(text: &str) -> Option<(&str, &str)> {
    if text.is_empty() {
        return None;
    }
    Some(match text.find('\n') {
        Some(i) => (&text[..i], &text[i + 1..]),
        None => (text, ""),
    })
}

/// `name` and `description` each occupy a single line in the replica format; collapse any
/// embedded newlines so a multi-line value can never shift the parse anchors.
fn oneline(s: &str) -> String {
    s.chars()
        .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
        .collect()
}

pub fn render(key: &ItemKey, item: &Item) -> String {
    let name = oneline(&item.name);
    let description = oneline(&item.description);
    let mut s = format!(
        "{}\n# {name}\n\n{description}\n\n",
        sections::marker(key.origin.as_str(), &key.native_id),
    );
    match &item.category {
        Some(c) => s.push_str(&format!("Category: {c}\n\n")),
        // An empty category line keeps a body that starts with `Category:` unambiguous.
        None if item.body.starts_with("Category:") => s.push_str("Category:\n\n"),
        None => {}
    }
    s.push_str(&item.body);
    s
}

pub fn parse_replica(text: &str) -> Option<Replica> {
    let (marker, rest) = take_line(text)?;
    let (origin, id) = sections::parse_marker(marker)?;
    let (heading, rest) = take_line(rest)?;
    let name = heading.strip_prefix("# ")?;
    let (blank, rest) = take_line(rest)?;
    if !blank.is_empty() {
        return None;
    }
    let (description, rest) = take_line(rest)?;
    let (blank, mut rest) = take_line(rest)?;
    if !blank.is_empty() {
        return None;
    }
    let mut category = None;
    if rest.starts_with("Category:") {
        let (line, after) = take_line(rest)?;
        let value = line["Category:".len()..].trim();
        if !value.is_empty() {
            category = Some(value.to_string());
        }
        let (blank, after) = take_line(after)?;
        if !blank.is_empty() {
            return None;
        }
        rest = after;
    }
    Some(Replica {
        origin,
        id,
        name: name.to_string(),
        description: description.to_string(),
        category,
        body: rest.to_string(),
    })
}

pub fn read_opt(path: &Path) -> Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(s) => Ok(Some(s)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Error::io(path, e)),
    }
}

fn write_if_changed(path: &Path, content: &str) -> Result<()> {
    if read_opt(path)?.as_deref() == Some(content) {
        return Ok(());
    }
    fsx::atomic_write(path, content.as_bytes())
}

/// Directory name of a project: its last path component plus 8 hex digits of the path's hash.
pub fn project_key(cwd: &Path) -> String {
    let path = cwd.to_string_lossy();
    let last = cwd
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let digest = Sha256::digest(path.as_bytes());
    let hex: String = digest.iter().take(4).map(|b| format!("{b:02x}")).collect();
    format!("{}-{hex}", slugify(&last))
}

/// Item files of a project dir as `(file name, replica)`, sorted by file name.
fn project_items(dir: &Path) -> Result<Vec<(String, Replica)>> {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(Error::io(dir, e)),
    };
    let mut out = Vec::new();
    for entry in entries {
        let path = entry.map_err(|e| Error::io(dir, e))?.path();
        let Some(file) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else {
            continue;
        };
        if file == INDEX_FILE || !file.ends_with(".md") || !path.is_file() {
            continue;
        }
        let text = fs::read_to_string(&path).map_err(|e| Error::io(&path, e))?;
        if let Some(r) = parse_replica(&text) {
            out.push((file, r));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

fn index_path(dir: &Path) -> PathBuf {
    dir.join(INDEX_FILE)
}

fn scope_path(dir: &Path) -> PathBuf {
    dir.join(SCOPE_FILE)
}

fn read_scope_cwd(dir: &Path) -> Result<Option<String>> {
    let path = scope_path(dir);
    let Some(text) = read_opt(&path)? else {
        return Ok(None);
    };
    let value: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| Error::Parse(format!("{}: {e}", path.display())))?;
    Ok(value.get("cwd").and_then(|c| c.as_str()).map(String::from))
}

fn index_text(items: &[(String, Replica)]) -> String {
    let mut s = String::from("# Memory Index\n\n");
    for (file, r) in items {
        s.push_str(&format!("- [{}]({file}) — {}\n", r.name, r.description));
    }
    s
}

fn routing_block(projects: &[(String, PathBuf)]) -> String {
    let mut s = format!(
        "{BLOCK_BEGIN}\n## Project memory (managed by nativemem)\n\
         When your working directory is, or is inside, one of the directories below, read that \
         project's memory index first and open the item files it lists when they are relevant \
         to the task.\n"
    );
    for (cwd, index) in projects {
        s.push_str(&format!("- {cwd} → {}\n", index.display()));
    }
    s.push_str(BLOCK_END);
    s
}

fn splice_block(text: &str, block: Option<&str>) -> Result<String> {
    Ok(match (block_span(text)?, block, region_span(text)?) {
        (Some((s, e)), Some(b), _) => format!("{}{b}{}", &text[..s], &text[e..]),
        // The instruction region stays the last managed region of the file.
        (None, Some(b), Some((r, _))) => {
            let (before, rest) = text.split_at(r);
            let lead = if before.is_empty() || before.ends_with("\n\n") {
                ""
            } else if before.ends_with('\n') {
                "\n"
            } else {
                "\n\n"
            };
            let gap = if before.is_empty() { "\n" } else { "\n\n" };
            format!("{before}{lead}{b}{gap}{rest}")
        }
        (None, Some(b), None) => {
            let sep = if text.is_empty() {
                ""
            } else if text.ends_with('\n') {
                "\n"
            } else {
                "\n\n"
            };
            format!("{text}{sep}{b}\n")
        }
        (Some((s, e)), None, _) => {
            let mut before = &text[..s];
            if before.ends_with("\n\n") {
                before = &before[..before.len() - 1];
            }
            let after = text[e..].strip_prefix('\n').unwrap_or(&text[e..]);
            format!("{before}{after}")
        }
        (None, None, _) => text.to_string(),
    })
}

impl MemoryStore {
    /// `home` is the agent's home directory (backups are stored relative to it), `base` holds the
    /// project directories, `keep` is the deepest directory that is never removed when the last
    /// item of a project goes, and `agents_file` is the global file that receives the routing
    /// block.
    pub fn new(
        agent: AgentId,
        home: PathBuf,
        state_dir: PathBuf,
        backups: BackupSession,
        base: PathBuf,
        keep: PathBuf,
        agents_file: PathBuf,
    ) -> Self {
        MemoryStore {
            agent,
            home,
            state_dir,
            backups,
            base,
            keep,
            agents_file,
        }
    }

    fn project_dir(&self, key: &str) -> PathBuf {
        self.base.join(key)
    }

    fn project_dirs(&self) -> Result<Vec<(String, PathBuf)>> {
        let entries = match fs::read_dir(&self.base) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(Error::io(&self.base, e)),
        };
        let mut out = Vec::new();
        for entry in entries {
            let path = entry.map_err(|e| Error::io(&self.base, e))?.path();
            if path.is_dir() {
                let key = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                out.push((key, path));
            }
        }
        out.sort();
        Ok(out)
    }

    /// Every item file in the store; all of them are replicas.
    pub fn scan(&self) -> Result<Vec<Observed>> {
        let mut out = Vec::new();
        for (key, dir) in self.project_dirs()? {
            let Some(cwd) = read_scope_cwd(&dir)? else {
                continue;
            };
            for (file, r) in project_items(&dir)? {
                out.push(Observed {
                    agent: self.agent.clone(),
                    native_id: format!("{key}/{file}"),
                    path: dir.join(&file),
                    item: Item {
                        kind: Kind::Memory,
                        scope: Scope::Project(PathBuf::from(&cwd)),
                        name: r.name,
                        description: r.description,
                        category: r.category,
                        body: r.body,
                        source_dir: None,
                    },
                    replica_of: Some(ItemKey {
                        origin: AgentId::new(r.origin),
                        native_id: r.id,
                    }),
                });
            }
        }
        Ok(out)
    }

    pub fn write_replica(&self, key: &ItemKey, item: &Item) -> Result<(String, PathBuf)> {
        let Scope::Project(cwd) = &item.scope else {
            return Err(Error::unsupported(
                self.agent.as_str(),
                "write_replica global scope",
            ));
        };
        let project = project_key(cwd);
        let dir = self.project_dir(&project);
        let items = project_items(&dir)?;
        let existing = items
            .iter()
            .find(|(_, r)| r.origin == key.origin.as_str() && r.id == key.native_id)
            .map(|(f, _)| f.clone());
        let file = match existing {
            Some(f) => f,
            None => {
                let slug = slugify(&item.name);
                let taken = |f: &str| f.eq_ignore_ascii_case(INDEX_FILE) || dir.join(f).exists();
                let mut file = format!("{slug}.md");
                let mut n = 2;
                while taken(&file) {
                    file = format!("{slug}-{n}.md");
                    n += 1;
                }
                file
            }
        };
        let path = dir.join(&file);
        write_if_changed(&path, &render(key, item))?;
        self.regenerate_index(&dir)?;
        let scope = serde_json::json!({ "cwd": cwd.to_string_lossy() });
        write_if_changed(&scope_path(&dir), &scope.to_string())?;
        self.sync_routing()?;
        Ok((format!("{project}/{file}"), path))
    }

    /// Removes one replica by its native id and tidies what that leaves empty. Ids that name a
    /// native original (they contain `#`) are not this store's to remove.
    pub fn remove(&self, native_id: &str) -> Result<()> {
        let agent = self.agent.as_str();
        if native_id.contains('#') {
            return Err(Error::unsupported(agent, "remove original"));
        }
        let (project, file) = native_id
            .split_once('/')
            .filter(|(p, f)| {
                ![p, f]
                    .iter()
                    .any(|s| s.is_empty() || **s == ".." || s.contains('/'))
            })
            .ok_or_else(|| Error::Parse(format!("invalid {agent} replica id {native_id:?}")))?;
        let dir = self.project_dir(project);
        fsx::remove_with_backup(
            &self.backups,
            &self.state_dir,
            &self.agent,
            &self.home,
            &dir.join(file),
        )?;
        if project_items(&dir)?.is_empty() {
            for f in [index_path(&dir), scope_path(&dir)] {
                if f.exists() {
                    fs::remove_file(&f).map_err(|e| Error::io(&f, e))?;
                }
            }
            self.prune_empty(&dir);
        } else {
            self.regenerate_index(&dir)?;
        }
        self.sync_routing()
    }

    /// Removes `dir` and its empty parents below `keep`. A directory that still holds files
    /// (for example the user's own) stays in place.
    fn prune_empty(&self, dir: &Path) {
        let mut current = dir;
        while current != self.keep && current.starts_with(&self.keep) {
            if fs::remove_dir(current).is_err() {
                return;
            }
            let Some(parent) = current.parent() else {
                return;
            };
            current = parent;
        }
    }

    fn regenerate_index(&self, dir: &Path) -> Result<()> {
        let items = project_items(dir)?;
        write_if_changed(&index_path(dir), &index_text(&items))
    }

    fn sync_routing(&self) -> Result<()> {
        let mut projects = Vec::new();
        for (_, dir) in self.project_dirs()? {
            let index = index_path(&dir);
            if let (Some(cwd), true) = (read_scope_cwd(&dir)?, index.is_file()) {
                projects.push((cwd, index));
            }
        }
        projects.sort();
        let block = (!projects.is_empty()).then(|| routing_block(&projects));
        let path = &self.agents_file;
        let existing = read_opt(path)?;
        let updated = match (&existing, &block) {
            (None, None) => return Ok(()),
            (None, Some(b)) => format!("{b}\n"),
            (Some(text), b) => splice_block(text, b.as_deref())?,
        };
        if existing.as_deref() == Some(updated.as_str()) {
            return Ok(());
        }
        if existing.is_some() {
            fsx::backup(
                &self.backups,
                &self.state_dir,
                &self.agent,
                &self.home,
                path,
            )?;
        }
        fsx::atomic_write(path, updated.as_bytes())
    }
}
