use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use crate::compat::Check;
use crate::error::{Error, Result};
use crate::fsx::{self, BackupSession};
use crate::model::{Adapter, AgentId, Item, ItemKey, Kind, Observed, Scope};
use crate::session::{
    derived_uuid, is_session_id, marker_text, set_mtime, Session, SessionOptions, SessionReport,
    SessionScan,
};
use crate::slug::slugify;

use super::claude_probes;
use super::claude_sessions::{self, ClaudeSessions, MARKER_KEY};
use super::sections::InstructionFile;

const INSTRUCTION_FILE: &str = "CLAUDE.md";
const INDEX_FILE: &str = "MEMORY.md";
const ORIGIN_KEY: &str = "nativemem_origin";
const HELPER_FILE: &str = "nativemem.jsonl";
const NEW_INDEX: &str = "# Memory Index\n\n";
/// Bump when the written session format changes; unmodified replicas are then rewritten.
const SESSION_WRITER_REVISION: u32 = 1;

pub struct ClaudeAdapter {
    claude_home: PathBuf,
    state_dir: PathBuf,
    backups: BackupSession,
    fs_root: Option<PathBuf>,
    instructions: InstructionFile,
    sessions: Option<ClaudeSessions>,
    blocked: Vec<Kind>,
    retention: Retention,
}

/// How long Claude Code keeps a transcript, by file modification time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Retention {
    /// `cleanupPeriodDays`; 0 is read as retention disabled `[UNVERIFIED]`.
    pub days: u64,
    /// The value comes from `settings.json`, not from the default.
    pub configured: bool,
}

const DEFAULT_RETENTION_DAYS: u64 = 30;
const DAY_MS: u64 = 86_400_000;

fn read_retention(claude_home: &Path) -> Retention {
    let configured = fs::read_to_string(claude_home.join("settings.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v.get("cleanupPeriodDays")?.as_u64());
    Retention {
        days: configured.unwrap_or(DEFAULT_RETENTION_DAYS),
        configured: configured.is_some(),
    }
}

/// Directory listings allowed per project name before the name is treated as unresolved.
const MAX_LISTED_DIRS: usize = 10_000;

struct Scan {
    items: Vec<Observed>,
    skipped_projects: Vec<(String, String)>,
    skipped_files: Vec<(String, String)>,
}

/// Frontmatter lines keep their line endings so untouched keys survive byte for byte.
struct Doc {
    fm: Vec<String>,
    body: String,
}

impl ClaudeAdapter {
    pub fn new(claude_home: PathBuf, state_dir: PathBuf, backups: BackupSession) -> Self {
        let claude_home_for_retention = claude_home.clone();
        ClaudeAdapter {
            instructions: InstructionFile::new(
                AgentId::new("claude"),
                claude_home.clone(),
                state_dir.clone(),
                backups.clone(),
                INSTRUCTION_FILE,
            ),
            claude_home,
            state_dir,
            backups,
            fs_root: default_fs_root(),
            sessions: None,
            blocked: Vec::new(),
            retention: read_retention(&claude_home_for_retention),
        }
    }

    pub fn retention(&self) -> Retention {
        self.retention
    }

    /// Kinds this adapter neither reads nor writes while its compatibility checks fail.
    pub fn with_blocked_kinds(mut self, kinds: &[Kind]) -> Self {
        self.blocked = kinds.to_vec();
        self
    }

    /// Takes part in session sync; without this call sessions stay out.
    pub fn with_sessions(mut self, options: SessionOptions) -> Self {
        self.sessions = Some(ClaudeSessions::new(self.claude_home.clone(), options));
        self
    }

    fn scan_sessions(&self) -> Result<SessionScan> {
        if self.blocked.contains(&Kind::Session) {
            return Ok(SessionScan::default());
        }
        match &self.sessions {
            Some(s) => s.scan(&self.id()),
            None => Ok(SessionScan::default()),
        }
    }

    /// Transcripts that cannot be synced and how many are automated.
    pub fn session_report(&self) -> Result<SessionReport> {
        Ok(self.scan_sessions()?.report())
    }

    /// Writes the replica transcript into the project directory of the session's cwd; the file
    /// name and session id derive from the origin, so a refresh rewrites the same file.
    fn write_session_replica(&self, key: &ItemKey, item: &Item) -> Result<(String, PathBuf)> {
        let session = Session::from_body(&item.body)
            .ok_or_else(|| Error::Parse("session body is not a session".into()))?;
        let dir = match self
            .project_cwds()?
            .into_iter()
            .find(|(_, c)| c.as_deref().ok() == Some(session.cwd.as_path()))
        {
            Some((d, _)) => d,
            None => encode_name(&session.cwd.to_string_lossy()),
        };
        let uuid = derived_uuid(&format!("claude-replica:{}", marker_text(key)));
        let file = format!("{uuid}.jsonl");
        let path = self.projects_dir().join(&dir).join(&file);
        let text = claude_sessions::render(&session, key, &uuid);
        if read_optional(&path)?.as_deref() != Some(text.as_str()) {
            self.overwrite(&path, &text)?;
        }
        set_mtime(&path, session.updated_ms);
        Ok((format!("{dir}/{file}"), path))
    }

    /// Resolves directory names against `root` instead of `/`; the encoded name then
    /// describes the path below `root` (at `/` the leading `-` of the name is dropped).
    pub fn with_fs_root(mut self, root: PathBuf) -> Self {
        self.fs_root = Some(root);
        self
    }

    /// Project dirs holding memory files whose working directory cannot be resolved: (dir, reason).
    pub fn skipped_projects(&self) -> Result<Vec<(String, String)>> {
        Ok(self.collect()?.skipped_projects)
    }

    /// Memory files without parsable frontmatter: (native_id, reason).
    pub fn skipped_files(&self) -> Result<Vec<(String, String)>> {
        Ok(self.collect()?.skipped_files)
    }

    fn projects_dir(&self) -> PathBuf {
        self.claude_home.join("projects")
    }

    fn collect(&self) -> Result<Scan> {
        let mut scan = Scan {
            items: Vec::new(),
            skipped_projects: Vec::new(),
            skipped_files: Vec::new(),
        };
        let projects = if self.blocked.contains(&Kind::Memory) {
            Vec::new()
        } else {
            self.project_cwds()?
        };
        for (dir, cwd) in projects {
            let mem = self.projects_dir().join(&dir).join("memory");
            let files = memory_files(&mem)?;
            let cwd = match cwd {
                Ok(cwd) => cwd,
                Err(reason) => {
                    if !files.is_empty() {
                        scan.skipped_projects.push((dir, reason));
                    }
                    continue;
                }
            };
            for path in files {
                let native_id = format!("{dir}/{}", file_name(&path));
                let text = match fs::read(&path) {
                    Ok(b) => match String::from_utf8(b) {
                        Ok(t) => t,
                        Err(_) => {
                            scan.skipped_files
                                .push((native_id, "not valid UTF-8".into()));
                            continue;
                        }
                    },
                    Err(e) => return Err(Error::io(&path, e)),
                };
                let Some(doc) = parse_doc(&text).filter(|d| field(&d.fm, "name", false).is_some())
                else {
                    scan.skipped_files
                        .push((native_id, "no parsable frontmatter with name".into()));
                    continue;
                };
                let replica_of = field(&doc.fm, ORIGIN_KEY, true).and_then(|v| {
                    let (agent, id) = v.split_once(':')?;
                    Some(ItemKey {
                        origin: AgentId::new(agent),
                        native_id: id.to_string(),
                    })
                });
                let item = Item {
                    kind: Kind::Memory,
                    scope: Scope::Project(cwd.clone()),
                    name: field(&doc.fm, "name", false).unwrap_or_default(),
                    description: field(&doc.fm, "description", false).unwrap_or_default(),
                    category: field(&doc.fm, "type", true)
                        .or_else(|| field(&doc.fm, "type", false)),
                    body: doc.body,
                };
                scan.items.push(Observed {
                    agent: self.id(),
                    native_id,
                    path,
                    item,
                    replica_of,
                });
            }
        }
        if !self.blocked.contains(&Kind::Instruction) {
            scan.items.extend(self.instructions.scan()?);
        }
        Ok(scan)
    }

    /// Every project dir name with its cwd (from jsonl files, else decoded from the name) or
    /// the reason it is unresolved, sorted by name.
    fn project_cwds(&self) -> Result<Vec<(String, std::result::Result<PathBuf, String>)>> {
        let root = self.projects_dir();
        if !root.is_dir() {
            return Ok(Vec::new());
        }
        let mut dirs: Vec<PathBuf> = read_dir_paths(&root)?
            .into_iter()
            .filter(|p| p.is_dir())
            .collect();
        dirs.sort();
        dirs.into_iter()
            .map(|d| {
                let name = file_name(&d);
                let cwd = match project_cwd(&d)? {
                    Some(cwd) => Ok(cwd),
                    None => self.decode_dir_name(&name),
                };
                Ok((name, cwd))
            })
            .collect()
    }

    /// Decodes a project dir name by walking real directories; exactly one match is accepted.
    fn decode_dir_name(&self, name: &str) -> std::result::Result<PathBuf, String> {
        const UNRESOLVED: &str =
            "no cwd in jsonl and directory name does not resolve to an existing path";
        let Some(root) = &self.fs_root else {
            return Err(UNRESOLVED.into());
        };
        let rest = if root.parent().is_none() {
            name.strip_prefix('-').unwrap_or(name)
        } else {
            name
        };
        let mut listed = 0;
        let mut found = Vec::new();
        walk_dirs(root, rest, &mut listed, &mut found);
        match found.len() {
            _ if listed > MAX_LISTED_DIRS => Err(UNRESOLVED.into()),
            0 => Err(UNRESOLVED.into()),
            1 => Ok(found.remove(0)),
            n => Err(format!(
                "no cwd in jsonl and directory name resolves to {n} paths"
            )),
        }
    }

    fn split_native_id<'a>(&self, native_id: &'a str) -> Result<(&'a str, &'a str)> {
        match native_id.split_once('/') {
            Some((dir, file))
                if !dir.is_empty()
                    && !file.is_empty()
                    && dir != ".."
                    && file != ".."
                    && !file.contains('/') =>
            {
                Ok((dir, file))
            }
            _ => Err(Error::Parse(format!(
                "invalid claude native id: {native_id}"
            ))),
        }
    }

    fn backup(&self, path: &Path) -> Result<()> {
        fsx::backup(
            &self.backups,
            &self.state_dir,
            &self.id(),
            &self.claude_home,
            path,
        )
        .map(|_| ())
    }

    /// Write `text` to `path`, backing up the previous content when the file exists.
    fn overwrite(&self, path: &Path, text: &str) -> Result<()> {
        if path.exists() {
            self.backup(path)?;
        }
        fsx::atomic_write(path, text.as_bytes())
    }

    fn update_index(&self, mem: &Path, file: &str, name: &str, description: &str) -> Result<()> {
        let path = mem.join(INDEX_FILE);
        let old = read_optional(&path)?.unwrap_or_else(|| NEW_INDEX.to_string());
        let line = format!("- [{name}]({file}) — {description}");
        let mut out = String::new();
        let mut placed = false;
        for l in old.split_inclusive('\n') {
            if is_index_line(l, file) {
                if !placed {
                    let eol = &l[l.trim_end_matches(['\n', '\r']).len()..];
                    out.push_str(&line);
                    out.push_str(if eol.is_empty() { "\n" } else { eol });
                    placed = true;
                }
            } else {
                out.push_str(l);
            }
        }
        if !placed {
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(&line);
            out.push('\n');
        }
        if path.exists() && out == old {
            return Ok(());
        }
        self.overwrite(&path, &out)
    }

    fn drop_index_line(&self, mem: &Path, file: &str) -> Result<()> {
        let path = mem.join(INDEX_FILE);
        let Some(old) = read_optional(&path)? else {
            return Ok(());
        };
        let out: String = old
            .split_inclusive('\n')
            .filter(|l| !is_index_line(l, file))
            .collect();
        if out == old {
            return Ok(());
        }
        self.overwrite(&path, &out)
    }

    fn find_replica(&self, mem: &Path, marker: &str) -> Result<Option<PathBuf>> {
        for path in memory_files(mem)? {
            let Some(text) = read_optional(&path)? else {
                continue;
            };
            if parse_doc(&text)
                .and_then(|d| field(&d.fm, ORIGIN_KEY, true))
                .is_some_and(|v| v == marker)
            {
                return Ok(Some(path));
            }
        }
        Ok(None)
    }
}

impl Adapter for ClaudeAdapter {
    fn id(&self) -> AgentId {
        AgentId::new("claude")
    }

    /// A session older than the retention minus a day would be deleted by Claude soon after it is
    /// written (it carries its original's modification time).
    fn accepts(&self, item: &Item) -> bool {
        if item.kind != Kind::Session || self.retention.days == 0 {
            return true;
        }
        let Some(session) = Session::from_body(&item.body) else {
            return true;
        };
        let keep_ms = (self.retention.days - 1)
            .saturating_mul(DAY_MS)
            .min(i64::MAX as u64 / 2) as i64;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64);
        now.saturating_sub(session.updated_ms) <= keep_ms
    }

    fn version(&self) -> Option<String> {
        claude_probes::version(&self.claude_home)
    }

    fn probes(&self) -> Vec<Check> {
        claude_probes::probes(&self.claude_home)
    }

    fn supports(&self, kind: Kind, scope: &Scope) -> bool {
        !self.blocked.contains(&kind)
            && matches!(
                (kind, scope),
                (Kind::Memory | Kind::Session, Scope::Project(_))
                    | (Kind::Instruction, Scope::Global)
            )
    }

    fn writer_revision(&self, kind: Kind) -> u32 {
        if kind == Kind::Session {
            SESSION_WRITER_REVISION
        } else {
            0
        }
    }

    fn scan(&self) -> Result<Vec<Observed>> {
        let mut out = self.collect()?.items;
        out.extend(self.scan_sessions()?.observed);
        Ok(out)
    }

    fn write_replica(&self, key: &ItemKey, item: &Item) -> Result<(String, PathBuf)> {
        if item.kind == Kind::Instruction {
            return self.instructions.write_replica(key, item);
        }
        if item.kind == Kind::Session {
            return self.write_session_replica(key, item);
        }
        let Scope::Project(cwd) = &item.scope else {
            return Err(Error::unsupported("claude", "write_replica global scope"));
        };
        let existing = self
            .project_cwds()?
            .into_iter()
            .find(|(_, c)| c.as_deref().ok() == Some(cwd.as_path()))
            .map(|(d, _)| d);
        let dir = match existing {
            Some(d) => d,
            None => {
                let encoded = encode_name(&cwd.to_string_lossy());
                let cwd_json = serde_json::json!({ "cwd": cwd.to_string_lossy() });
                let marker_file = self.projects_dir().join(&encoded).join(HELPER_FILE);
                self.overwrite(&marker_file, &format!("{cwd_json}\n"))?;
                encoded
            }
        };
        let mem = self.projects_dir().join(&dir).join("memory");
        let marker = format!("{}:{}", key.origin, key.native_id);
        let path = match self.find_replica(&mem, &marker)? {
            Some(p) => p,
            None => {
                let stem = slugify(&item.name);
                let mut n = 1;
                loop {
                    let candidate = if n == 1 {
                        format!("{stem}.md")
                    } else {
                        format!("{stem}-{n}.md")
                    };
                    if !mem.join(&candidate).exists() {
                        break mem.join(candidate);
                    }
                    n += 1;
                }
            }
        };
        let file = file_name(&path);

        let mut fm = format!(
            "name: {}\ndescription: {}\nmetadata:\n",
            quote(&item.name),
            quote(&item.description)
        );
        if let Some(c) = &item.category {
            fm.push_str(&format!("  type: {}\n", quote(c)));
        }
        fm.push_str(&format!("  {ORIGIN_KEY}: {}\n", quote(&marker)));
        self.overwrite(&path, &render(&fm, &item.body))?;
        self.update_index(&mem, &file, &item.name, &item.description)?;
        Ok((format!("{dir}/{file}"), path))
    }

    fn write_original(&self, native_id: &str, item: &Item) -> Result<()> {
        if is_session_id(native_id) {
            return Err(Error::unsupported("claude", "write_original session"));
        }
        if self.instructions.owns(native_id) {
            return self.instructions.write_original(native_id, item);
        }
        let (dir, file) = self.split_native_id(native_id)?;
        let mem = self.projects_dir().join(dir).join("memory");
        let path = mem.join(file);
        let text = fs::read_to_string(&path).map_err(|e| Error::io(&path, e))?;
        let mut doc = parse_doc(&text)
            .ok_or_else(|| Error::Parse(format!("no frontmatter in {}", path.display())))?;
        set_top(&mut doc.fm, "name", &item.name);
        set_top(&mut doc.fm, "description", &item.description);
        match &item.category {
            Some(c) => set_category(&mut doc.fm, c),
            None => clear_category(&mut doc.fm),
        }
        self.overwrite(&path, &render(&doc.fm.concat(), &item.body))?;
        self.update_index(&mem, file, &item.name, &item.description)
    }

    fn remove(&self, native_id: &str) -> Result<()> {
        if is_session_id(native_id) {
            return Err(Error::unsupported("claude", "remove session"));
        }
        if self.instructions.owns(native_id) {
            return self.instructions.remove(native_id);
        }
        let (dir, file) = self.split_native_id(native_id)?;
        let mem = self.projects_dir().join(dir).join("memory");
        fsx::remove_with_backup(
            &self.backups,
            &self.state_dir,
            &self.id(),
            &self.claude_home,
            &mem.join(file),
        )?;
        self.drop_index_line(&mem, file)
    }
}

#[cfg(unix)]
fn default_fs_root() -> Option<PathBuf> {
    Some(PathBuf::from("/"))
}

#[cfg(not(unix))]
fn default_fs_root() -> Option<PathBuf> {
    None
}

/// Claude's project dir naming: every non-ASCII-alphanumeric character becomes `-`.
pub(super) fn encode_name(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// Collects complete directory paths below `dir` whose encoded components spell `rest`.
/// Unreadable directories count as empty; stops once the listing budget is spent.
fn walk_dirs(dir: &Path, rest: &str, listed: &mut usize, found: &mut Vec<PathBuf>) {
    if *listed > MAX_LISTED_DIRS {
        return;
    }
    *listed += 1;
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let encoded = encode_name(&file_name(&path));
        if encoded == rest {
            found.push(path);
        } else if let Some(next) = rest
            .strip_prefix(encoded.as_str())
            .and_then(|r| r.strip_prefix('-'))
        {
            walk_dirs(&path, next, listed, found);
        }
    }
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn read_dir_paths(dir: &Path) -> Result<Vec<PathBuf>> {
    fs::read_dir(dir)
        .map_err(|e| Error::io(dir, e))?
        .map(|e| e.map(|e| e.path()).map_err(|e| Error::io(dir, e)))
        .collect()
}

fn read_optional(path: &Path) -> Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(t) => Ok(Some(t)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Error::io(path, e)),
    }
}

/// Sorted `*.md` files directly inside `mem`, excluding the index.
fn memory_files(mem: &Path) -> Result<Vec<PathBuf>> {
    if !mem.is_dir() {
        return Ok(Vec::new());
    }
    let mut files: Vec<PathBuf> = read_dir_paths(mem)?
        .into_iter()
        .filter(|p| {
            p.is_file() && p.extension().is_some_and(|e| e == "md") && file_name(p) != INDEX_FILE
        })
        .collect();
    files.sort();
    Ok(files)
}

/// `cwd` of the first jsonl line that has one; unparsable lines are ignored. Session replica
/// lines (they carry the marker) prove nothing about the folder, while the helper file stays
/// evidence: it is what keeps a memory replica's new project directory resolvable.
fn project_cwd(dir: &Path) -> Result<Option<PathBuf>> {
    let mut logs: Vec<PathBuf> = read_dir_paths(dir)?
        .into_iter()
        .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == "jsonl"))
        .collect();
    logs.sort();
    for log in logs {
        let file = fs::File::open(&log).map_err(|e| Error::io(&log, e))?;
        let mut reader = BufReader::new(file);
        let mut buf = Vec::new();
        loop {
            buf.clear();
            if reader
                .read_until(b'\n', &mut buf)
                .map_err(|e| Error::io(&log, e))?
                == 0
            {
                break;
            }
            if !buf.windows(5).any(|w| w == b"\"cwd\"")
                || buf
                    .windows(MARKER_KEY.len())
                    .any(|w| w == MARKER_KEY.as_bytes())
            {
                continue;
            }
            let Ok(v) = serde_json::from_slice::<serde_json::Value>(&buf) else {
                continue;
            };
            if let Some(cwd) = v
                .get("cwd")
                .and_then(|c| c.as_str())
                .filter(|c| !c.is_empty())
            {
                return Ok(Some(PathBuf::from(cwd)));
            }
        }
    }
    Ok(None)
}

fn parse_doc(text: &str) -> Option<Doc> {
    let mut lines = text.split_inclusive('\n');
    if lines.next()?.trim_end() != "---" {
        return None;
    }
    let mut fm = Vec::new();
    let mut consumed = text.find('\n')? + 1;
    for l in lines {
        consumed += l.len();
        if l.trim_end() == "---" {
            let rest = &text[consumed..];
            let body = rest
                .strip_prefix("\r\n")
                .or_else(|| rest.strip_prefix('\n'))
                .unwrap_or(rest);
            return Some(Doc {
                fm,
                body: body.to_string(),
            });
        }
        fm.push(l.to_string());
    }
    None
}

fn render(fm: &str, body: &str) -> String {
    let mut out = format!("---\n{fm}---\n\n{body}");
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

fn is_index_line(line: &str, file: &str) -> bool {
    // Match only the first link target on the line so a description that happens
    // to reference another memory file cannot collide with that file's row.
    let Some(rest) = line.strip_prefix("- [") else {
        return false;
    };
    let Some((_label, after)) = rest.split_once("](") else {
        return false;
    };
    let Some((target, _)) = after.split_once(')') else {
        return false;
    };
    target == file
}

fn kv(line: &str, key: &str) -> Option<String> {
    let (k, v) = line.split_once(':')?;
    (k.trim() == key).then(|| unquote(v.trim()))
}

/// Top-level key, or a key nested one level under `metadata:` when `nested`.
fn field(fm: &[String], key: &str, nested: bool) -> Option<String> {
    let mut in_meta = false;
    for raw in fm {
        let l = raw.trim_end_matches(['\n', '\r']);
        if l.trim().is_empty() || l.trim_start().starts_with('#') {
            continue;
        }
        if l.starts_with([' ', '\t']) {
            if nested && in_meta {
                if let Some(v) = kv(l.trim_start(), key) {
                    return Some(v);
                }
            }
        } else {
            in_meta = l.trim_end() == "metadata:";
            if !nested {
                if let Some(v) = kv(l, key) {
                    return Some(v);
                }
            }
        }
    }
    None
}

/// Replace a top-level key's line, or insert it after `name` (first line for `name` itself).
fn set_top(fm: &mut Vec<String>, key: &str, value: &str) {
    let is_top = |l: &String, k: &str| !l.starts_with([' ', '\t']) && kv(l, k).is_some();
    let new_line = |eol: &str| format!("{key}: {}{eol}", quote(value));
    if let Some(i) = fm.iter().position(|l| is_top(l, key)) {
        let eol = if fm[i].ends_with("\r\n") {
            "\r\n"
        } else {
            "\n"
        };
        fm[i] = new_line(eol);
    } else {
        let at = fm
            .iter()
            .position(|l| is_top(l, "name"))
            .map_or(0, |i| i + 1);
        fm.insert(at, new_line("\n"));
    }
}

/// Line indexes of `metadata.type`, top-level `type` and the `metadata:` line.
fn type_lines(fm: &[String]) -> (Option<usize>, Option<usize>, Option<usize>) {
    let mut in_meta = false;
    let mut nested_at = None;
    let mut top_at = None;
    let mut meta_at = None;
    for (i, raw) in fm.iter().enumerate() {
        let l = raw.trim_end_matches(['\n', '\r']);
        if l.trim().is_empty() || l.trim_start().starts_with('#') {
            continue;
        }
        if l.starts_with([' ', '\t']) {
            if in_meta && nested_at.is_none() && kv(l.trim_start(), "type").is_some() {
                nested_at = Some(i);
            }
        } else {
            in_meta = l.trim_end() == "metadata:";
            if in_meta {
                meta_at = Some(i);
            }
            if top_at.is_none() && kv(l, "type").is_some() {
                top_at = Some(i);
            }
        }
    }
    (nested_at, top_at, meta_at)
}

/// Removes the line `field(.., "type")` would read; other lines stay untouched.
fn clear_category(fm: &mut Vec<String>) {
    let (nested_at, top_at, _) = type_lines(fm);
    if let Some(i) = nested_at.or(top_at) {
        fm.remove(i);
    }
}

/// Updates `metadata.type` or top-level `type` in place; adds `metadata.type` when neither exists.
fn set_category(fm: &mut Vec<String>, value: &str) {
    if field(fm, "type", true)
        .or_else(|| field(fm, "type", false))
        .as_deref()
        == Some(value)
    {
        return;
    }
    let eol_of = |l: &str| if l.ends_with("\r\n") { "\r\n" } else { "\n" };
    let (nested_at, top_at, meta_at) = type_lines(fm);
    if let Some(i) = nested_at {
        let indent: String = fm[i]
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect();
        fm[i] = format!("{indent}type: {}{}", quote(value), eol_of(&fm[i]));
    } else if let Some(i) = top_at {
        fm[i] = format!("type: {}{}", quote(value), eol_of(&fm[i]));
    } else if let Some(i) = meta_at {
        let eol = eol_of(&fm[i]);
        fm.insert(i + 1, format!("  type: {}{eol}", quote(value)));
    } else {
        fm.push(format!("metadata:\n  type: {}\n", quote(value)));
    }
}

fn unquote(v: &str) -> String {
    let b = v.as_bytes();
    if b.len() >= 2 && b[0] == b'"' && b[b.len() - 1] == b'"' {
        let mut out = String::new();
        let mut chars = v[1..v.len() - 1].chars();
        while let Some(c) = chars.next() {
            match (c, chars.clone().next()) {
                ('\\', Some(n @ ('"' | '\\'))) => {
                    out.push(n);
                    chars.next();
                }
                _ => out.push(c),
            }
        }
        out
    } else if b.len() >= 2 && b[0] == b'\'' && b[b.len() - 1] == b'\'' {
        v[1..v.len() - 1].replace("''", "'")
    } else {
        v.to_string()
    }
}

fn quote(v: &str) -> String {
    let quote_chars = ['"', '\''];
    let needs = v.contains([':', '#'])
        || v.starts_with(char::is_whitespace)
        || v.ends_with(char::is_whitespace)
        || v.starts_with(quote_chars)
        || v.ends_with(quote_chars);
    if needs {
        format!("\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        v.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::content_hash;
    use crate::testkit::{ClaudeHome, FrontmatterVariant};

    struct Fx {
        _t: tempfile::TempDir,
        root: PathBuf,
        state: PathBuf,
        home: ClaudeHome,
        adapter: ClaudeAdapter,
    }

    fn fx() -> Fx {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("claude");
        let state = t.path().join("state");
        fs::create_dir_all(&root).unwrap();
        let adapter =
            ClaudeAdapter::new(root.clone(), state.clone(), BackupSession::with_stamp("T"))
                .with_fs_root(t.path().join("fsroot"));
        Fx {
            home: ClaudeHome::new(&root),
            _t: t,
            root,
            state,
            adapter,
        }
    }

    fn item(cwd: &str, name: &str, desc: &str, cat: Option<&str>, body: &str) -> Item {
        Item {
            kind: Kind::Memory,
            scope: Scope::Project(PathBuf::from(cwd)),
            name: name.into(),
            description: desc.into(),
            category: cat.map(String::from),
            body: body.into(),
        }
    }

    fn key(origin: &str, id: &str) -> ItemKey {
        ItemKey {
            origin: AgentId::new(origin),
            native_id: id.into(),
        }
    }

    fn mem(f: &Fx, dir: &str) -> PathBuf {
        f.root.join("projects").join(dir).join("memory")
    }

    fn backup_of(f: &Fx, rel: &str) -> PathBuf {
        f.state.join("backups/T/claude").join(rel)
    }

    #[test]
    fn scan_both_variants_and_quoted() {
        let f = fx();
        let dir = f.home.add_project(Path::new("/work/a"));
        f.home.add_memory(
            &dir,
            "n.md",
            FrontmatterVariant::Nested,
            "n",
            "nested d",
            "user",
            "nb",
        );
        f.home.add_memory(
            &dir,
            "f.md",
            FrontmatterVariant::Flat,
            "f",
            "flat d",
            "feedback",
            "fb",
        );
        let m = mem(&f, &dir);
        fs::write(
            m.join("q.md"),
            "---\nname: q\ndescription: \"Note: a #tag\"\nmetadata:\n  type: project\n---\n\nqb\n",
        )
        .unwrap();
        fs::write(m.join("notes.txt"), "x").unwrap();
        fs::create_dir_all(m.join("sub.md")).unwrap();
        fs::write(m.join("bare.md"), "no frontmatter").unwrap();

        let got = f.adapter.scan().unwrap();
        let by = |id: &str| {
            got.iter()
                .find(|o| o.native_id == format!("{dir}/{id}"))
                .unwrap()
        };
        assert_eq!(got.len(), 3);
        assert_eq!(by("n.md").item.category.as_deref(), Some("user"));
        assert_eq!(by("f.md").item.category.as_deref(), Some("feedback"));
        assert_eq!(by("q.md").item.description, "Note: a #tag");
        assert_eq!(by("q.md").item.category.as_deref(), Some("project"));
        assert_eq!(by("n.md").item.scope, Scope::Project("/work/a".into()));
        assert_eq!(by("n.md").item.body.trim_end(), "nb");
        assert!(by("n.md").replica_of.is_none());
        let skipped = f.adapter.skipped_files().unwrap();
        assert_eq!(skipped.len(), 1);
        assert_eq!(skipped[0].0, format!("{dir}/bare.md"));
    }

    #[test]
    fn cwd_resolution_and_skipped_project() {
        let f = fx();
        assert!(f.adapter.scan().unwrap().is_empty());

        let p = f.root.join("projects/-lossy-dir");
        fs::create_dir_all(p.join("memory")).unwrap();
        fs::write(
            p.join("a.jsonl"),
            "not json\n{\"type\":\"x\"}\n{\"cwd\":\"/real/path\",\"x\":1}\n{\"cwd\":\"/later\"}\n",
        )
        .unwrap();
        fs::write(
            p.join("memory/m.md"),
            "---\nname: m\ndescription: d\n---\n\nb\n",
        )
        .unwrap();

        let orphan = f.root.join("projects/-orphan/memory");
        fs::create_dir_all(&orphan).unwrap();
        fs::write(
            orphan.join("m.md"),
            "---\nname: m\ndescription: d\n---\n\nb\n",
        )
        .unwrap();

        let got = f.adapter.scan().unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].item.scope, Scope::Project("/real/path".into()));
        let sp = f.adapter.skipped_projects().unwrap();
        assert_eq!(sp.len(), 1);
        assert_eq!(sp[0].0, "-orphan");
        assert_eq!(
            sp[0].1,
            "no cwd in jsonl and directory name does not resolve to an existing path"
        );
    }

    fn orphan_project(f: &Fx, dir: &str) {
        let m = f.root.join("projects").join(dir).join("memory");
        fs::create_dir_all(&m).unwrap();
        fs::write(m.join("m.md"), "---\nname: m\ndescription: d\n---\n\nb\n").unwrap();
    }

    fn fs_dir(f: &Fx, rel: &str) -> PathBuf {
        let p = f.root.parent().unwrap().join("fsroot").join(rel);
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn name_decoded_against_filesystem_when_no_jsonl() {
        let f = fx();
        let want = fs_dir(&f, "Users/e-f_e/proj.b2c/web-app");
        orphan_project(&f, "Users-e-f-e-proj-b2c-web-app");
        let got = f.adapter.scan().unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].item.scope, Scope::Project(want));
        assert!(f.adapter.skipped_projects().unwrap().is_empty());
    }

    #[test]
    fn ambiguous_decoding_stays_skipped() {
        let f = fx();
        fs_dir(&f, "a-b");
        fs_dir(&f, "a/b");
        orphan_project(&f, "a-b");
        assert!(f.adapter.scan().unwrap().is_empty());
        let sp = f.adapter.skipped_projects().unwrap();
        assert_eq!(sp.len(), 1);
        assert_eq!(
            sp[0].1,
            "no cwd in jsonl and directory name resolves to 2 paths"
        );
    }

    #[test]
    fn missing_path_stays_skipped() {
        let f = fx();
        fs_dir(&f, "a");
        orphan_project(&f, "a-gone");
        assert!(f.adapter.scan().unwrap().is_empty());
        assert_eq!(f.adapter.skipped_projects().unwrap().len(), 1);
    }

    fn replica_session_item() -> (ItemKey, Item) {
        use crate::session::{Role, Session, Turn};
        let session = Session {
            id: "o".into(),
            cwd: "/gone/app".into(),
            title: "t".into(),
            created_ms: 0,
            updated_ms: 0,
            turns: vec![Turn::new(Role::User, "q", 0)],
        };
        let key = ItemKey {
            origin: AgentId::new("pi"),
            native_id: "--gone-app--/x.jsonl".into(),
        };
        (key, session.to_item())
    }

    #[test]
    fn replica_sessions_do_not_make_a_missing_folder_resolve() {
        let f = fx();
        let (key, item) = replica_session_item();
        let adapter = ClaudeAdapter::new(
            f.root.clone(),
            f.state.clone(),
            BackupSession::with_stamp("T"),
        )
        .with_fs_root(f.root.join("no-fs-root"))
        .with_sessions(crate::session::SessionOptions::default());
        let (native_id, _) = adapter.write_replica(&key, &item).unwrap();
        let dir = native_id.split('/').next().unwrap().to_string();
        orphan_project(&f, &dir);
        let skipped = adapter.skipped_projects().unwrap();
        assert_eq!(skipped.len(), 1, "{skipped:?}");
        assert!(skipped[0].1.starts_with("no cwd in jsonl"));
        let scanned = adapter.scan().unwrap();
        assert!(scanned.iter().all(|o| o.item.kind == Kind::Session));
        assert_eq!(scanned.len(), 1, "the replica session is still listed");
        assert!(scanned[0].replica_of.is_some());
        // Writing again reuses the same file.
        assert_eq!(adapter.write_replica(&key, &item).unwrap().0, native_id);
    }

    #[test]
    fn a_genuine_transcript_next_to_replicas_resolves_the_project() {
        let f = fx();
        let (key, item) = replica_session_item();
        let adapter = ClaudeAdapter::new(
            f.root.clone(),
            f.state.clone(),
            BackupSession::with_stamp("T"),
        )
        .with_fs_root(f.root.join("no-fs-root"));
        let (native_id, _) = adapter.write_replica(&key, &item).unwrap();
        let dir = native_id.split('/').next().unwrap().to_string();
        orphan_project(&f, &dir);
        fs::write(
            f.root.join("projects").join(&dir).join("real.jsonl"),
            "{\"cwd\":\"/gone/app\",\"type\":\"user\"}\n",
        )
        .unwrap();
        assert!(adapter.skipped_projects().unwrap().is_empty());
        assert_eq!(adapter.scan().unwrap().len(), 1);
    }

    #[test]
    fn jsonl_cwd_wins_over_decoding() {
        let f = fx();
        fs_dir(&f, "a/b");
        orphan_project(&f, "a-b");
        fs::write(
            f.root.join("projects/a-b/s.jsonl"),
            "{\"cwd\":\"/elsewhere\"}\n",
        )
        .unwrap();
        let got = f.adapter.scan().unwrap();
        assert_eq!(got[0].item.scope, Scope::Project("/elsewhere".into()));
    }

    #[test]
    fn replica_lands_in_decoded_project_dir() {
        let f = fx();
        let cwd = fs_dir(&f, "w/app");
        orphan_project(&f, "w-app");
        let (id, _) = f
            .adapter
            .write_replica(
                &key("codex", "x"),
                &item(cwd.to_str().unwrap(), "N", "d", None, "b"),
            )
            .unwrap();
        assert!(id.starts_with("w-app/"));
    }

    #[test]
    fn replica_round_trip() {
        let f = fx();
        let body = "intro [[wiki-link]]\n---\nmiddle\n---\n- [[other]] end";
        let it = item(
            "/work/new",
            "My Note",
            "Why: because #1",
            Some("feedback"),
            body,
        );
        let k = key("codex", "/work/new#my-note:v");
        let (id, path) = f.adapter.write_replica(&k, &it).unwrap();
        assert!(path.exists());

        let got = f.adapter.scan().unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].native_id, id);
        assert_eq!(got[0].replica_of, Some(k));
        assert_eq!(content_hash(&got[0].item), content_hash(&it));
        assert_eq!(got[0].item.body.trim_end(), body);
        assert_eq!(got[0].item.description, "Why: because #1");
    }

    #[test]
    fn write_replica_existing_project_and_index() {
        let f = fx();
        let dir = f.home.add_project(Path::new("/work/a"));
        f.home.add_memory(
            &dir,
            "x.md",
            FrontmatterVariant::Flat,
            "x",
            "xd",
            "user",
            "xb",
        );
        let it = item("/work/a", "Fresh", "fd", None, "b");
        let (id, path) = f.adapter.write_replica(&key("codex", "k1"), &it).unwrap();
        assert_eq!(id, format!("{dir}/fresh.md"));
        assert_eq!(path, mem(&f, &dir).join("fresh.md"));
        assert_eq!(
            fs::read_to_string(mem(&f, &dir).join("MEMORY.md")).unwrap(),
            "# Memory Index\n\n- [x](x.md) — xd\n- [Fresh](fresh.md) — fd\n"
        );
        assert_eq!(f.adapter.scan().unwrap().len(), 2);
    }

    #[test]
    fn write_replica_new_project() {
        let f = fx();
        let it = item("/work/My App", "n", "d", Some("user"), "b");
        let (id, _) = f.adapter.write_replica(&key("codex", "k"), &it).unwrap();
        assert_eq!(id, "-work-My-App/n.md");
        let jsonl =
            fs::read_to_string(f.root.join("projects/-work-My-App/nativemem.jsonl")).unwrap();
        assert_eq!(jsonl, "{\"cwd\":\"/work/My App\"}\n");
        assert_eq!(
            fs::read_to_string(mem(&f, "-work-My-App").join("MEMORY.md")).unwrap(),
            "# Memory Index\n\n- [n](n.md) — d\n"
        );
        let got = f.adapter.scan().unwrap();
        assert_eq!(got[0].item.scope, Scope::Project("/work/My App".into()));
        // a second replica lands in the same project dir
        let (id2, _) = f
            .adapter
            .write_replica(
                &key("codex", "k2"),
                &item("/work/My App", "m", "d", None, "b"),
            )
            .unwrap();
        assert_eq!(id2, "-work-My-App/m.md");
    }

    #[test]
    fn filename_collision_and_in_place_update() {
        let f = fx();
        f.home.add_project(Path::new("/w"));
        let (a, _) = f
            .adapter
            .write_replica(&key("codex", "1"), &item("/w", "Same", "d1", None, "b1"))
            .unwrap();
        let (b, _) = f
            .adapter
            .write_replica(&key("codex", "2"), &item("/w", "Same", "d2", None, "b2"))
            .unwrap();
        let (c, _) = f
            .adapter
            .write_replica(&key("codex", "3"), &item("/w", "Same", "d3", None, "b3"))
            .unwrap();
        assert!(a.ends_with("/same.md") && b.ends_with("/same-2.md") && c.ends_with("/same-3.md"));

        let (b2, _) = f
            .adapter
            .write_replica(
                &key("codex", "2"),
                &item("/w", "Same", "d2", None, "edited"),
            )
            .unwrap();
        assert_eq!(b2, b);
        let got = f.adapter.scan().unwrap();
        assert_eq!(got.len(), 3);
        let edited = got.iter().find(|o| o.native_id == b).unwrap();
        assert_eq!(edited.item.body.trim_end(), "edited");
        assert!(backup_of(&f, "projects/-w/memory/same-2.md").exists());
    }

    #[test]
    fn index_line_updated_without_disturbing_others() {
        let f = fx();
        let dir = f.home.add_project(Path::new("/w"));
        let idx = mem(&f, &dir).join("MEMORY.md");
        fs::write(
            &idx,
            "# Memory Index\n\nfree text\n- [other](o.md) — keep\n",
        )
        .unwrap();
        let k = key("codex", "1");
        f.adapter
            .write_replica(&k, &item("/w", "n", "first", None, "b"))
            .unwrap();
        f.adapter
            .write_replica(&k, &item("/w", "n", "second", None, "b"))
            .unwrap();
        assert_eq!(
            fs::read_to_string(&idx).unwrap(),
            "# Memory Index\n\nfree text\n- [other](o.md) — keep\n- [n](n.md) — second\n"
        );
        assert!(backup_of(&f, &format!("projects/{dir}/memory/MEMORY.md")).exists());
    }

    #[test]
    fn index_description_link_does_not_collide_with_other_file_row() {
        let f = fx();
        let dir = f.home.add_project(Path::new("/w"));
        let idx = mem(&f, &dir).join("MEMORY.md");
        // Alpha's description references b.md; it must not be treated as b.md's row.
        fs::write(
            &idx,
            "# Memory Index\n\n- [Alpha](a.md) — see also [Beta](b.md)\n",
        )
        .unwrap();
        let (b_id, b_path) = f
            .adapter
            .write_replica(&key("codex", "b"), &item("/w", "B", "bd", None, "bb"))
            .unwrap();
        assert!(b_path.ends_with("b.md"));
        let text = fs::read_to_string(&idx).unwrap();
        assert!(
            text.contains("- [Alpha](a.md) — see also [Beta](b.md)"),
            "Alpha row clobbered: {text}"
        );
        assert!(
            text.contains("- [B](b.md) — bd"),
            "Beta row missing: {text}"
        );

        // Removing b.md drops only the b.md row, never Alpha's.
        f.adapter.remove(&b_id).unwrap();
        let text = fs::read_to_string(&idx).unwrap();
        assert!(text.contains("- [Alpha](a.md) — see also [Beta](b.md)"));
        assert!(!text.contains("- [B](b.md)"));
    }

    #[test]
    fn write_original_preserves_unknown_keys() {
        let f = fx();
        let dir = f.home.add_project(Path::new("/w"));
        let path = mem(&f, &dir).join("o.md");
        let fm = "custom: keep me\nname: old\nweird:   spaced  \ndescription: old d\nmetadata:\n  type: user\n  node_type: memory\n  extra: [1, 2]\n  originSessionId: s\n";
        fs::write(&path, format!("---\n{fm}---\n\nold body\n")).unwrap();
        fs::write(
            mem(&f, &dir).join("MEMORY.md"),
            "# Memory Index\n\n- [old](o.md) — old d\n- [z](z.md) — z\n",
        )
        .unwrap();

        f.adapter
            .write_original(
                &format!("{dir}/o.md"),
                &item("/w", "new", "Why: new", Some("user"), "new\n---\nbody"),
            )
            .unwrap();

        let want = fm
            .replace("name: old", "name: new")
            .replace("description: old d", "description: \"Why: new\"");
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            format!("---\n{want}---\n\nnew\n---\nbody\n")
        );
        assert_eq!(
            fs::read_to_string(mem(&f, &dir).join("MEMORY.md")).unwrap(),
            "# Memory Index\n\n- [new](o.md) — Why: new\n- [z](z.md) — z\n"
        );
        let bak =
            fs::read_to_string(backup_of(&f, &format!("projects/{dir}/memory/o.md"))).unwrap();
        assert!(bak.contains("old body"));
    }

    #[test]
    fn remove_deletes_file_and_index_line_with_backups() {
        let f = fx();
        let dir = f.home.add_project(Path::new("/w"));
        f.home.add_memory(
            &dir,
            "a.md",
            FrontmatterVariant::Nested,
            "a",
            "ad",
            "user",
            "ab",
        );
        f.home.add_memory(
            &dir,
            "b.md",
            FrontmatterVariant::Nested,
            "b",
            "bd",
            "user",
            "bb",
        );
        f.adapter.remove(&format!("{dir}/a.md")).unwrap();
        assert!(!mem(&f, &dir).join("a.md").exists());
        assert_eq!(
            fs::read_to_string(mem(&f, &dir).join("MEMORY.md")).unwrap(),
            "# Memory Index\n\n- [b](b.md) — bd\n"
        );
        assert!(backup_of(&f, &format!("projects/{dir}/memory/a.md")).exists());
        assert!(backup_of(&f, &format!("projects/{dir}/memory/MEMORY.md")).exists());
        assert!(f.adapter.remove("../x/a.md").is_err());
    }

    #[test]
    fn global_scope_unsupported() {
        let f = fx();
        let mut it = item("/w", "n", "d", None, "b");
        it.scope = Scope::Global;
        let err = f
            .adapter
            .write_replica(&key("codex", "k"), &it)
            .unwrap_err();
        assert!(matches!(err, Error::Unsupported { .. }));
    }

    #[test]
    fn write_original_sets_category_in_place() {
        let f = fx();
        let dir = f.home.add_project(Path::new("/w"));
        let nested = f.home.add_memory(
            &dir,
            "n.md",
            FrontmatterVariant::Nested,
            "n",
            "d",
            "user",
            "b",
        );
        let flat = f.home.add_memory(
            &dir,
            "f.md",
            FrontmatterVariant::Flat,
            "f",
            "d",
            "user",
            "b",
        );
        let bare = mem(&f, &dir).join("b.md");
        fs::write(&bare, "---\nname: b\ndescription: d\n---\n\nb\n").unwrap();
        let before_nested = fs::read_to_string(&nested).unwrap();
        let before_flat = fs::read_to_string(&flat).unwrap();

        for (file, name) in [("n.md", "n"), ("f.md", "f"), ("b.md", "b")] {
            f.adapter
                .write_original(
                    &format!("{dir}/{file}"),
                    &item("/w", name, "d", Some("feedback"), "b"),
                )
                .unwrap();
        }
        assert_eq!(
            fs::read_to_string(&nested).unwrap(),
            before_nested.replace("type: user", "type: feedback")
        );
        assert_eq!(
            fs::read_to_string(&flat).unwrap(),
            before_flat.replace("type: user", "type: feedback")
        );
        assert_eq!(
            fs::read_to_string(&bare).unwrap(),
            "---\nname: b\ndescription: d\nmetadata:\n  type: feedback\n---\n\nb\n"
        );
    }

    #[test]
    fn write_original_without_category_removes_type_line() {
        let f = fx();
        let dir = f.home.add_project(Path::new("/w"));
        let nested = f.home.add_memory(
            &dir,
            "n.md",
            FrontmatterVariant::Nested,
            "n",
            "d",
            "user",
            "b",
        );
        let flat = f.home.add_memory(
            &dir,
            "f.md",
            FrontmatterVariant::Flat,
            "f",
            "d",
            "user",
            "b",
        );
        let before_nested = fs::read_to_string(&nested).unwrap();
        let before_flat = fs::read_to_string(&flat).unwrap();
        for (file, name) in [("n.md", "n"), ("f.md", "f")] {
            f.adapter
                .write_original(&format!("{dir}/{file}"), &item("/w", name, "d", None, "b"))
                .unwrap();
        }
        assert_eq!(
            fs::read_to_string(&nested).unwrap(),
            before_nested.replace("  type: user\n", "")
        );
        assert_eq!(
            fs::read_to_string(&flat).unwrap(),
            before_flat.replace("type: user\n", "")
        );
        let scanned = f.adapter.scan().unwrap();
        assert!(scanned.iter().all(|o| o.item.category.is_none()));
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
    fn missing_claude_md_means_no_instruction_items() {
        let f = fx();
        assert!(f.adapter.scan().unwrap().is_empty());
    }

    #[test]
    fn instruction_originals_and_replicas_are_scanned() {
        let f = fx();
        let text = "# T\n\n## Mine\nmy body\n\n<!-- nativemem:instructions:begin -->\n<!-- nativemem origin=codex id=AGENTS.md%23theirs -->\n## Theirs\n\nbody\n<!-- nativemem:instructions:end -->\n";
        fs::write(f.root.join("CLAUDE.md"), text).unwrap();
        let items = f.adapter.scan().unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].native_id, "CLAUDE.md#mine");
        assert_eq!(items[0].item.kind, Kind::Instruction);
        assert_eq!(items[0].item.scope, Scope::Global);
        assert_eq!(items[0].item.body, "my body");
        assert!(items[0].replica_of.is_none());
        assert_eq!(items[1].native_id, "CLAUDE.md#codex:AGENTS.md#theirs");
        assert_eq!(items[1].replica_of, Some(key("codex", "AGENTS.md#theirs")));
        assert_eq!(items[1].item.name, "theirs");
    }

    #[test]
    fn instruction_replica_creates_claude_md_with_only_the_region() {
        let f = fx();
        let (id, path) = f
            .adapter
            .write_replica(&key("codex", "AGENTS.md#a"), &instruction("A", "body"))
            .unwrap();
        assert_eq!(id, "CLAUDE.md#codex:AGENTS.md#a");
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("<!-- nativemem:instructions:begin -->\n"));
        assert!(text.ends_with("<!-- nativemem:instructions:end -->\n"));
        assert!(!f.state.join("backups").exists());
        f.adapter.remove(&id).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "");
        assert!(f.state.join("backups/T/claude/CLAUDE.md").is_file());
    }

    #[test]
    fn instruction_write_original_and_remove_keep_other_bytes() {
        let f = fx();
        let text = "# T\n@RTK.md\n\n## A\nold\n\n## B\nkeep\n";
        fs::write(f.root.join("CLAUDE.md"), text).unwrap();
        f.adapter
            .write_original("CLAUDE.md#a", &instruction("A", "new"))
            .unwrap();
        let after = fs::read_to_string(f.root.join("CLAUDE.md")).unwrap();
        assert_eq!(after, text.replace("old", "new"));
        assert_eq!(
            fs::read_to_string(f.state.join("backups/T/claude/CLAUDE.md")).unwrap(),
            text
        );
        f.adapter.remove("CLAUDE.md#a").unwrap();
        assert_eq!(
            fs::read_to_string(f.root.join("CLAUDE.md")).unwrap(),
            "# T\n@RTK.md\n\n## B\nkeep\n"
        );
        assert!(f.adapter.remove("CLAUDE.md#a").is_err());
    }

    #[test]
    fn instruction_items_are_global_only() {
        let f = fx();
        let mut it = instruction("A", "b");
        it.scope = Scope::Project(PathBuf::from("/w"));
        assert!(matches!(
            f.adapter.write_replica(&key("codex", "AGENTS.md#a"), &it),
            Err(Error::Unsupported { .. })
        ));
        fs::write(f.root.join("CLAUDE.md"), "## A\nb\n").unwrap();
        assert!(matches!(
            f.adapter.write_original("CLAUDE.md#a", &it),
            Err(Error::Unsupported { .. })
        ));
    }
}
