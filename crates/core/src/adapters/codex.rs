use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::error::{Error, Result};
use crate::fsx::{self, BackupSession};
use crate::model::{Adapter, AgentId, Item, ItemKey, Kind, Observed, Scope};
use crate::session::{is_session_id, SessionOptions, SessionReport, SessionScan};
use crate::slug::slugify;

use super::codex_sessions::CodexSessions;
use super::sections::{self, block_span, region_span, InstructionFile, BLOCK_BEGIN, BLOCK_END};

const AGENT: &str = "codex";
const INSTRUCTION_FILE: &str = "AGENTS.md";
const INDEX_FILE: &str = "MEMORY.md";
const SCOPE_FILE: &str = "scope.json";
const TASK_GROUP: &str = "# Task Group:";

/// Replica store paths. The layout follows Codex's memory-extension convention and is
/// confined to this module so it can change in one place.
mod layout {
    use super::*;

    pub fn resources(home: &Path) -> PathBuf {
        home.join("memories/extensions/nativemem/resources")
    }

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

    pub fn project_dir(home: &Path, key: &str) -> PathBuf {
        resources(home).join(key)
    }

    pub fn index(dir: &Path) -> PathBuf {
        dir.join(INDEX_FILE)
    }

    pub fn scope(dir: &Path) -> PathBuf {
        dir.join(SCOPE_FILE)
    }

    pub fn originals(home: &Path) -> PathBuf {
        home.join("memories").join(INDEX_FILE)
    }
}

pub struct CodexAdapter {
    home: PathBuf,
    state_dir: PathBuf,
    backups: BackupSession,
    instructions: InstructionFile,
    sessions: Option<CodexSessions>,
}

struct Replica {
    origin: String,
    id: String,
    name: String,
    description: String,
    category: Option<String>,
    body: String,
}

struct OriginalBlock {
    title: String,
    cwd: String,
    text: String,
}

struct Originals {
    blocks: Vec<OriginalBlock>,
    skipped: Vec<(String, String)>,
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

fn render(key: &ItemKey, item: &Item) -> String {
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

fn parse_replica(text: &str) -> Option<Replica> {
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

fn read_opt(path: &Path) -> Result<Option<String>> {
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

fn read_scope_cwd(dir: &Path) -> Result<Option<String>> {
    let path = layout::scope(dir);
    let Some(text) = read_opt(&path)? else {
        return Ok(None);
    };
    let value: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| Error::Parse(format!("{}: {e}", path.display())))?;
    Ok(value.get("cwd").and_then(|c| c.as_str()).map(String::from))
}

fn project_dirs(home: &Path) -> Result<Vec<(String, PathBuf)>> {
    let root = layout::resources(home);
    let entries = match fs::read_dir(&root) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(Error::io(&root, e)),
    };
    let mut out = Vec::new();
    for entry in entries {
        let path = entry.map_err(|e| Error::io(&root, e))?.path();
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

fn parse_originals(text: &str) -> Originals {
    let mut chunks: Vec<Vec<&str>> = vec![Vec::new()];
    for line in text.split_inclusive('\n') {
        if line.starts_with(TASK_GROUP) {
            chunks.push(Vec::new());
        }
        chunks.last_mut().expect("never empty").push(line);
    }
    let mut out = Originals {
        blocks: Vec::new(),
        skipped: Vec::new(),
    };
    let preamble = chunks.remove(0).concat();
    if !preamble.trim().is_empty() {
        out.skipped.push((
            "memories/MEMORY.md (preamble)".into(),
            "content before the first task group".into(),
        ));
    }
    for chunk in chunks {
        let block = chunk.concat();
        let title = chunk[0][TASK_GROUP.len()..].trim().to_string();
        let cwd = chunk
            .iter()
            .filter_map(|l| l.strip_prefix("applies_to:"))
            .find_map(|v| {
                let rest = &v[v.find("cwd=")? + 4..];
                let path = rest.split(';').next()?.trim();
                (!path.is_empty()).then(|| path.to_string())
            });
        match cwd {
            Some(cwd) => out.blocks.push(OriginalBlock {
                title,
                cwd,
                text: block.trim_end().to_string(),
            }),
            None => out
                .skipped
                .push((title, "task group has no applies_to cwd".into())),
        }
    }
    out
}

impl CodexAdapter {
    pub fn new(codex_home: PathBuf, state_dir: PathBuf, backups: BackupSession) -> Self {
        CodexAdapter {
            instructions: InstructionFile::new(
                Self::agent(),
                codex_home.clone(),
                state_dir.clone(),
                backups.clone(),
                INSTRUCTION_FILE,
            ),
            home: codex_home,
            state_dir,
            backups,
            sessions: None,
        }
    }

    /// Reads rollouts as session originals; without this call sessions stay out. Codex never
    /// receives sessions from this tool.
    pub fn with_sessions(mut self, options: SessionOptions) -> Self {
        self.sessions = Some(CodexSessions::new(self.home.clone(), options));
        self
    }

    fn scan_sessions(&self) -> Result<SessionScan> {
        match &self.sessions {
            Some(s) => s.scan(&Self::agent()),
            None => Ok(SessionScan::default()),
        }
    }

    /// Rollouts that cannot be synced and how many are automated.
    pub fn session_report(&self) -> Result<SessionReport> {
        Ok(self.scan_sessions()?.report())
    }

    /// Codex-generated content that cannot be mirrored: `(label, reason)`.
    pub fn skipped(&self) -> Result<Vec<(String, String)>> {
        Ok(self.originals()?.skipped)
    }

    fn agent() -> AgentId {
        AgentId::new(AGENT)
    }

    fn originals(&self) -> Result<Originals> {
        let text = read_opt(&layout::originals(&self.home))?.unwrap_or_default();
        Ok(parse_originals(&text))
    }

    fn regenerate_index(&self, dir: &Path) -> Result<()> {
        let items = project_items(dir)?;
        write_if_changed(&layout::index(dir), &index_text(&items))
    }

    fn sync_routing(&self) -> Result<()> {
        let mut projects = Vec::new();
        for (_, dir) in project_dirs(&self.home)? {
            let index = layout::index(&dir);
            if let (Some(cwd), true) = (read_scope_cwd(&dir)?, index.is_file()) {
                projects.push((cwd, index));
            }
        }
        projects.sort();
        let block = (!projects.is_empty()).then(|| routing_block(&projects));
        let path = self.home.join("AGENTS.md");
        let existing = read_opt(&path)?;
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
                &Self::agent(),
                &self.home,
                &path,
            )?;
        }
        fsx::atomic_write(&path, updated.as_bytes())
    }
}

impl Adapter for CodexAdapter {
    fn id(&self) -> AgentId {
        Self::agent()
    }

    fn supports(&self, kind: Kind, scope: &Scope) -> bool {
        matches!(
            (kind, scope),
            (Kind::Memory, Scope::Project(_)) | (Kind::Instruction, Scope::Global)
        )
    }

    fn scan(&self) -> Result<Vec<Observed>> {
        let mut out = Vec::new();
        for (key, dir) in project_dirs(&self.home)? {
            let Some(cwd) = read_scope_cwd(&dir)? else {
                continue;
            };
            for (file, r) in project_items(&dir)? {
                out.push(Observed {
                    agent: Self::agent(),
                    native_id: format!("{key}/{file}"),
                    path: dir.join(&file),
                    item: Item {
                        kind: Kind::Memory,
                        scope: Scope::Project(PathBuf::from(&cwd)),
                        name: r.name,
                        description: r.description,
                        category: r.category,
                        body: r.body,
                    },
                    replica_of: Some(ItemKey {
                        origin: AgentId::new(r.origin),
                        native_id: r.id,
                    }),
                });
            }
        }
        let originals = self.originals()?;
        let path = layout::originals(&self.home);
        let mut seen: Vec<String> = Vec::new();
        for b in originals.blocks {
            let slug = slugify(&b.title);
            let base = format!("{}#{slug}", b.cwd);
            let mut native_id = base.clone();
            let mut n = 2;
            while seen.contains(&native_id) {
                native_id = format!("{base}-{n}");
                n += 1;
            }
            seen.push(native_id.clone());
            out.push(Observed {
                agent: Self::agent(),
                native_id,
                path: path.clone(),
                item: Item {
                    kind: Kind::Memory,
                    scope: Scope::Project(PathBuf::from(&b.cwd)),
                    name: slug,
                    description: b.title,
                    category: None,
                    body: b.text,
                },
                replica_of: None,
            });
        }
        out.extend(self.instructions.scan()?);
        out.extend(self.scan_sessions()?.observed);
        Ok(out)
    }

    fn write_replica(&self, key: &ItemKey, item: &Item) -> Result<(String, PathBuf)> {
        if item.kind == Kind::Instruction {
            return self.instructions.write_replica(key, item);
        }
        if item.kind == Kind::Session {
            return Err(Error::unsupported(AGENT, "write_replica session"));
        }
        let Scope::Project(cwd) = &item.scope else {
            return Err(Error::unsupported(AGENT, "write_replica global scope"));
        };
        let project = layout::project_key(cwd);
        let dir = layout::project_dir(&self.home, &project);
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
        write_if_changed(&layout::scope(&dir), &scope.to_string())?;
        self.sync_routing()?;
        Ok((format!("{project}/{file}"), path))
    }

    fn write_original(&self, native_id: &str, item: &Item) -> Result<()> {
        if is_session_id(native_id) {
            return Err(Error::unsupported(AGENT, "write_original session"));
        }
        if self.instructions.owns(native_id) {
            return self.instructions.write_original(native_id, item);
        }
        Err(Error::unsupported(AGENT, "write_original"))
    }

    fn remove(&self, native_id: &str) -> Result<()> {
        if is_session_id(native_id) {
            return Err(Error::unsupported(AGENT, "remove session"));
        }
        if self.instructions.owns(native_id) {
            return self.instructions.remove(native_id);
        }
        if native_id.contains('#') {
            return Err(Error::unsupported(AGENT, "remove original"));
        }
        let (project, file) = native_id
            .split_once('/')
            .filter(|(p, f)| {
                ![p, f]
                    .iter()
                    .any(|s| s.is_empty() || **s == ".." || s.contains('/'))
            })
            .ok_or_else(|| Error::Parse(format!("invalid codex replica id {native_id:?}")))?;
        let dir = layout::project_dir(&self.home, project);
        fsx::remove_with_backup(
            &self.backups,
            &self.state_dir,
            &Self::agent(),
            &self.home,
            &dir.join(file),
        )?;
        if project_items(&dir)?.is_empty() {
            for f in [layout::index(&dir), layout::scope(&dir)] {
                if f.exists() {
                    fs::remove_file(&f).map_err(|e| Error::io(&f, e))?;
                }
            }
            // Stays in place when the user keeps other files in it.
            let _ = fs::remove_dir(&dir);
        } else {
            self.regenerate_index(&dir)?;
        }
        self.sync_routing()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::content_hash;

    struct Fx {
        _t: tempfile::TempDir,
        home: PathBuf,
        state: PathBuf,
        a: CodexAdapter,
    }

    fn fx() -> Fx {
        let t = tempfile::tempdir().unwrap();
        let home = t.path().join("codex");
        let state = t.path().join("state");
        fs::create_dir_all(&home).unwrap();
        let a = CodexAdapter::new(
            home.clone(),
            state.clone(),
            BackupSession::with_stamp("20260101T000000Z"),
        );
        Fx {
            _t: t,
            home,
            state,
            a,
        }
    }

    fn item(cwd: &str, name: &str, category: Option<&str>, body: &str) -> Item {
        Item {
            kind: Kind::Memory,
            scope: Scope::Project(PathBuf::from(cwd)),
            name: name.into(),
            description: format!("about {name}"),
            category: category.map(String::from),
            body: body.into(),
        }
    }

    fn key(id: &str) -> ItemKey {
        ItemKey {
            origin: AgentId::new("claude"),
            native_id: id.into(),
        }
    }

    fn agents(f: &Fx) -> String {
        fs::read_to_string(f.home.join("AGENTS.md")).unwrap()
    }

    #[test]
    fn replica_round_trips_tricky_bodies() {
        let f = fx();
        let bodies = [
            "# Heading first\n\ntext",
            "Category: not really\nmore",
            "---\nyaml-like\n---\n",
            "\nstarts blank",
            "see [[wiki-link]] and [[other]]\n\n\nspaced",
            "",
            "# Title\n\nCategory: x\n\n- a\n- b",
        ];
        for (i, body) in bodies.iter().enumerate() {
            for cat in [None, Some("feedback")] {
                let id = format!("proj/mem {i}#{}--x>y", cat.is_some());
                let it = item(
                    "/work/app",
                    &format!("item {i} {}", cat.is_some()),
                    cat,
                    body,
                );
                f.a.write_replica(&key(&id), &it).unwrap();
                let found =
                    f.a.scan()
                        .unwrap()
                        .into_iter()
                        .find(|o| o.replica_of == Some(key(&id)))
                        .unwrap();
                assert_eq!(
                    content_hash(&found.item),
                    content_hash(&it),
                    "body {body:?}"
                );
                assert_eq!(found.item.category, it.category);
                assert_eq!(found.item.scope, it.scope);
            }
        }
    }

    #[test]
    fn multiline_name_and_description_round_trip() {
        let f = fx();
        // Newlines in name/description would shift the parse anchors; render collapses
        // them to spaces so the replica still scans back.
        let it = Item {
            kind: Kind::Memory,
            scope: Scope::Project(PathBuf::from("/w/app")),
            name: "with\nnewline".into(),
            description: "first line\nsecond line".into(),
            category: None,
            body: "body".into(),
        };
        f.a.write_replica(&key("k"), &it).unwrap();
        let got =
            f.a.scan()
                .unwrap()
                .into_iter()
                .find(|o| o.replica_of == Some(key("k")))
                .unwrap();
        assert_eq!(got.item.name, "with newline");
        assert_eq!(got.item.description, "first line second line");
    }

    #[test]
    fn marker_id_round_trips_odd_characters() {
        for id in ["a/b c#d--e>f", "100% sure %41", "ünï/çode", "x -->y"] {
            let k = ItemKey {
                origin: AgentId::new("claude"),
                native_id: id.into(),
            };
            let text = render(&k, &item("/p", "n", None, "b"));
            let r = parse_replica(&text).unwrap();
            assert_eq!((r.origin.as_str(), r.id.as_str()), ("claude", id));
        }
    }

    #[test]
    fn update_in_place_keeps_file() {
        let f = fx();
        let (id1, p1) =
            f.a.write_replica(&key("k"), &item("/w/a", "first", None, "one"))
                .unwrap();
        let (id2, p2) =
            f.a.write_replica(&key("k"), &item("/w/a", "renamed", None, "two"))
                .unwrap();
        assert_eq!((id1, &p1), (id2, &p2));
        let all: Vec<_> = f.a.scan().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].item.name, "renamed");
        assert_eq!(all[0].item.body, "two");
    }

    #[test]
    fn filename_collision_gets_suffix() {
        let f = fx();
        let (a, _) =
            f.a.write_replica(&key("k1"), &item("/w/a", "Same Name", None, "1"))
                .unwrap();
        let (b, _) =
            f.a.write_replica(&key("k2"), &item("/w/a", "same name", None, "2"))
                .unwrap();
        let (c, _) =
            f.a.write_replica(&key("k3"), &item("/w/a", "SAME-name", None, "3"))
                .unwrap();
        assert!(a.ends_with("/same-name.md"));
        assert!(b.ends_with("/same-name-2.md"));
        assert!(c.ends_with("/same-name-3.md"));
    }

    #[test]
    fn memory_slug_does_not_clobber_index() {
        let f = fx();
        let (id, _) =
            f.a.write_replica(&key("k"), &item("/w/a", "memory", None, "x"))
                .unwrap();
        assert!(id.ends_with("/memory-2.md"));
        assert_eq!(f.a.scan().unwrap().len(), 1);
    }

    #[test]
    fn project_key_is_stable_and_distinct() {
        let k = layout::project_key(Path::new("/work/My App"));
        assert_eq!(k, layout::project_key(Path::new("/work/My App")));
        assert!(k.starts_with("my-app-"));
        assert_eq!(k.len(), "my-app-".len() + 8);
        assert_ne!(k, layout::project_key(Path::new("/other/My App")));
    }

    #[test]
    fn replica_layout_scope_and_index() {
        let f = fx();
        f.a.write_replica(&key("b"), &item("/w/app", "Beta", None, "b"))
            .unwrap();
        let (id, path) =
            f.a.write_replica(&key("a"), &item("/w/app", "Alpha", Some("user"), "a"))
                .unwrap();
        let (project, file) = id.split_once('/').unwrap();
        assert_eq!(file, "alpha.md");
        let dir = layout::project_dir(&f.home, project);
        assert_eq!(path, dir.join("alpha.md"));
        let scope: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(dir.join("scope.json")).unwrap()).unwrap();
        assert_eq!(scope["cwd"], "/w/app");
        assert_eq!(
            fs::read_to_string(dir.join("MEMORY.md")).unwrap(),
            "# Memory Index\n\n- [Alpha](alpha.md) — about Alpha\n- [Beta](beta.md) — about Beta\n"
        );
        assert_eq!(
            fs::read_to_string(path).unwrap(),
            "<!-- nativemem origin=claude id=a -->\n# Alpha\n\nabout Alpha\n\nCategory: user\n\na"
        );
    }

    #[test]
    fn global_scope_unsupported() {
        let f = fx();
        let mut it = item("/w", "n", None, "b");
        it.scope = Scope::Global;
        assert!(matches!(
            f.a.write_replica(&key("k"), &it),
            Err(Error::Unsupported { .. })
        ));
    }

    #[test]
    fn routing_block_insert_update_remove_preserves_surroundings() {
        let f = fx();
        let agents_path = f.home.join("AGENTS.md");
        let user = "# My rules\n\nbe nice\n";
        fs::write(&agents_path, user).unwrap();

        let (id_a, _) =
            f.a.write_replica(&key("a"), &item("/w/a", "A", None, "a"))
                .unwrap();
        let first = agents(&f);
        assert!(first.starts_with(&format!("{user}\n{BLOCK_BEGIN}\n")));
        assert!(first.contains("## Project memory (managed by nativemem)"));
        assert!(first.contains("- /w/a → "));
        assert!(first.ends_with(&format!("{BLOCK_END}\n")));
        assert!(f
            .state
            .join("backups/20260101T000000Z/codex/AGENTS.md")
            .exists());

        let (id_b, _) =
            f.a.write_replica(&key("b"), &item("/w/b", "B", None, "b"))
                .unwrap();
        let second = agents(&f);
        assert!(second.starts_with(user));
        assert!(second.contains("- /w/a → ") && second.contains("- /w/b → "));
        assert!(second.find("- /w/a").unwrap() < second.find("- /w/b").unwrap());
        assert_eq!(second.matches(BLOCK_BEGIN).count(), 1);

        f.a.remove(&id_a).unwrap();
        let third = agents(&f);
        assert!(!third.contains("/w/a") && third.contains("/w/b"));
        f.a.remove(&id_b).unwrap();
        assert_eq!(agents(&f), user);
    }

    #[test]
    fn routing_block_keeps_text_without_trailing_newline_and_after_block() {
        let f = fx();
        let agents_path = f.home.join("AGENTS.md");
        fs::write(&agents_path, "top").unwrap();
        f.a.write_replica(&key("a"), &item("/w/a", "A", None, "a"))
            .unwrap();
        assert!(agents(&f).starts_with("top\n\n"));

        let text = agents(&f);
        let tail = format!("{text}tail without newline");
        fs::write(&agents_path, &tail).unwrap();
        f.a.write_replica(&key("b"), &item("/w/b", "B", None, "b"))
            .unwrap();
        let updated = agents(&f);
        assert!(updated.starts_with("top\n\n"));
        assert!(updated.ends_with(&format!("{BLOCK_END}\ntail without newline")));
        assert!(updated.contains("/w/b"));
    }

    #[test]
    fn routing_block_created_when_agents_md_absent() {
        let f = fx();
        assert!(!f.home.join("AGENTS.md").exists());
        let (id, _) =
            f.a.write_replica(&key("a"), &item("/w/a", "A", None, "a"))
                .unwrap();
        let text = agents(&f);
        assert!(text.starts_with(BLOCK_BEGIN) && text.ends_with(&format!("{BLOCK_END}\n")));
        assert!(!f.state.join("backups").exists());
        f.a.remove(&id).unwrap();
        assert_eq!(agents(&f), "");
    }

    #[test]
    fn unchanged_routing_is_not_rewritten() {
        let f = fx();
        let it = item("/w/a", "A", None, "a");
        f.a.write_replica(&key("a"), &it).unwrap();
        let p = f.home.join("AGENTS.md");
        let before = fs::metadata(&p).unwrap().modified().unwrap();
        fs::remove_dir_all(f.state.join("backups")).ok();
        std::thread::sleep(std::time::Duration::from_millis(20));
        f.a.write_replica(&key("a"), &it).unwrap();
        assert_eq!(fs::metadata(&p).unwrap().modified().unwrap(), before);
        assert!(!f.state.join("backups").exists());
    }

    #[test]
    fn remove_backs_up_and_cleans_empty_project() {
        let f = fx();
        let (id1, p1) =
            f.a.write_replica(&key("a"), &item("/w/a", "One", None, "1"))
                .unwrap();
        let (id2, p2) =
            f.a.write_replica(&key("b"), &item("/w/a", "Two", None, "2"))
                .unwrap();
        let dir = p1.parent().unwrap().to_path_buf();

        f.a.remove(&id1).unwrap();
        assert!(!p1.exists());
        assert!(f.state.join("backups/20260101T000000Z/codex").exists());
        let index = fs::read_to_string(dir.join("MEMORY.md")).unwrap();
        assert!(!index.contains("One") && index.contains("Two"));

        f.a.remove(&id2).unwrap();
        assert!(!p2.exists() && !dir.exists());
        assert!(f.a.scan().unwrap().is_empty());
    }

    #[test]
    fn remove_rejects_traversal() {
        let f = fx();
        assert!(matches!(f.a.remove("../x/y.md"), Err(Error::Parse(_))));
        assert!(matches!(f.a.remove("nodir"), Err(Error::Parse(_))));
    }

    fn write_originals(f: &Fx, text: &str) {
        let p = f.home.join("memories/MEMORY.md");
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    }

    #[test]
    fn task_groups_parse_with_unscoped_and_duplicates() {
        let f = fx();
        write_originals(
            &f,
            "Intro text\n\n\
             # Task Group: Fix Login Bug\napplies_to: cwd=/w/app; reuse_rule=x\nnotes\n\n\
             # Task Group: No scope here\nnothing\n\n\
             # Task Group: Fix login bug\n- applies_to: nope\napplies_to: cwd=/w/app\nmore\n\n\
             # Task Group: Elsewhere\napplies_to: cwd=/w/other \n",
        );
        let obs = f.a.scan().unwrap();
        let ids: Vec<_> = obs.iter().map(|o| o.native_id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "/w/app#fix-login-bug",
                "/w/app#fix-login-bug-2",
                "/w/other#elsewhere"
            ]
        );
        let first = &obs[0];
        assert_eq!(first.item.name, "fix-login-bug");
        assert_eq!(first.item.description, "Fix Login Bug");
        assert_eq!(first.item.scope, Scope::Project("/w/app".into()));
        assert_eq!(first.item.category, None);
        assert!(first.item.body.starts_with("# Task Group: Fix Login Bug"));
        assert!(first.item.body.ends_with("notes"));
        assert!(first.replica_of.is_none());
        assert_eq!(first.path, f.home.join("memories/MEMORY.md"));

        let skipped = f.a.skipped().unwrap();
        assert_eq!(skipped.len(), 2);
        assert!(skipped[0].0.contains("preamble"));
        assert_eq!(skipped[1].0, "No scope here");
    }

    #[test]
    fn missing_originals_file_is_empty() {
        let f = fx();
        assert!(f.a.scan().unwrap().is_empty());
        assert!(f.a.skipped().unwrap().is_empty());
    }

    #[test]
    fn originals_are_read_only() {
        let f = fx();
        assert!(matches!(
            f.a.write_original("/w#x", &item("/w", "x", None, "b")),
            Err(Error::Unsupported { .. })
        ));
        assert!(matches!(
            f.a.remove("/w/app#x"),
            Err(Error::Unsupported { .. })
        ));
    }

    fn instruction(heading: &str, body: &str) -> Item {
        Item {
            kind: Kind::Instruction,
            scope: Scope::Global,
            name: slugify(heading),
            description: heading.into(),
            category: None,
            body: body.into(),
        }
    }

    #[test]
    fn instruction_originals_skip_the_routing_block_and_the_region() {
        let f = fx();
        fs::write(
            f.home.join("AGENTS.md"),
            format!(
                "# Mine\n\n## Style\nbe terse\n\n{BLOCK_BEGIN}\n## Project memory (managed by nativemem)\n- /x → /y\n{BLOCK_END}\n"
            ),
        )
        .unwrap();
        f.a.write_replica(&key("CLAUDE.md#a"), &instruction("A", "a body"))
            .unwrap();
        let items = f.a.scan().unwrap();
        let ids: Vec<_> = items.iter().map(|o| o.native_id.as_str()).collect();
        assert_eq!(ids, ["AGENTS.md#style", "AGENTS.md#claude:CLAUDE.md#a"]);
        assert!(items[0].replica_of.is_none());
        assert!(items[1].replica_of.is_some());
    }

    #[test]
    fn routing_block_goes_before_an_existing_region_and_leaves_it_alone() {
        let f = fx();
        f.a.write_replica(&key("CLAUDE.md#a"), &instruction("A", "a body"))
            .unwrap();
        let region_only = agents(&f);
        f.a.write_replica(&key("m"), &item("/w/a", "M", None, "m"))
            .unwrap();
        let both = agents(&f);
        let block = both.find(BLOCK_BEGIN).unwrap();
        let region = both.find("<!-- nativemem:instructions:begin -->").unwrap();
        assert!(block < region);
        assert!(both.ends_with(&region_only));
        let (id, _) =
            f.a.write_replica(&key("m"), &item("/w/a", "M", None, "m2"))
                .unwrap();
        f.a.remove(&id).unwrap();
        assert_eq!(agents(&f), region_only);
    }

    #[test]
    fn instruction_originals_are_editable_but_memory_originals_are_not() {
        let f = fx();
        fs::write(f.home.join("AGENTS.md"), "## Style\nold\n\n## Keep\nk\n").unwrap();
        f.a.write_original("AGENTS.md#style", &instruction("Style", "new"))
            .unwrap();
        assert_eq!(agents(&f), "## Style\nnew\n\n## Keep\nk\n");
        f.a.remove("AGENTS.md#style").unwrap();
        assert_eq!(agents(&f), "## Keep\nk\n");
        assert!(matches!(
            f.a.write_original("/w/app#fix-login", &item("/w/app", "n", None, "b")),
            Err(Error::Unsupported { .. })
        ));
        assert!(matches!(
            f.a.remove("/w/app#fix-login"),
            Err(Error::Unsupported { .. })
        ));
    }
}
