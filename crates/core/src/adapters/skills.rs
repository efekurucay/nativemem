//! Skills: a directory with a top-level `SKILL.md` directly inside an agent's own skills
//! directory (`<agent home>/skills/<name>/`). The directory name is the identity. Skills are
//! copied as whole trees, never rewritten. An item's body is a manifest (one line per file with
//! its executable bit, size and SHA-256), so the engine compares skills without holding contents;
//! `Item::source_dir` points the writers at the real files.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::memory_store::project_key;
use crate::compat::Check;
use crate::error::{Error, Result};
use crate::fsx::{self, BackupSession};
use crate::model::{content_hash, AgentId, Item, Kind, Observed, Scope};

/// Prefix of a skill's native id; it keeps skill ids apart from every other kind's ids.
pub const ID_PREFIX: &str = "skill:";
const SKILLS_DIR: &str = "skills";
const SKILL_FILE: &str = "SKILL.md";
/// Skills the agent manages itself (synced from its own cloud); never read or written.
const AGENT_MANAGED_DIR: &str = "synced";
/// Larger skills are skipped and reported.
pub const MAX_SKILL_BYTES: u64 = 50 * 1024 * 1024;
const IGNORED_FILES: &[&str] = &[".DS_Store"];
const IGNORED_EXTENSIONS: &[&str] = &["pyc"];
const IGNORED_DIRS: &[&str] = &["__pycache__", ".git", "node_modules"];
const SOURCES: &[&str] = &["crates/core/src/adapters/skills.rs"];
/// What the layout probe expects of an agent's skills directory.
const LAYOUT_EXPECTED: &str = "absent, empty, or most directories hold a top-level SKILL.md";

pub fn native_id(name: &str) -> String {
    format!("{ID_PREFIX}{name}")
}

/// The native id of a project skill.
pub fn project_native_id(project_key: &str, name: &str) -> String {
    format!("{ID_PREFIX}{project_key}/{name}")
}

/// The skill name inside a native id, `None` for ids of other kinds.
/// Works for both global (`skill:demo`) and project (`skill:proj-abc/demo`) ids.
pub fn skill_name(native_id: &str) -> Option<&str> {
    let rest = native_id.strip_prefix(ID_PREFIX)?;
    Some(rest.rsplit('/').next().unwrap_or(rest))
}

/// The project key of a project skill id, or `None` for a global skill.
pub fn skill_project_key(native_id: &str) -> Option<&str> {
    let rest = native_id.strip_prefix(ID_PREFIX)?;
    rest.rsplit_once('/').map(|(key, _)| key)
}

/// What an adapter reports about its skills apart from the items themselves.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SkillReport {
    /// `(skill, reason)` for skills or files that cannot take part.
    pub skipped: Vec<(String, String)>,
    /// Skills whose own copy differs from the copy in a visible root.
    pub shadowed: Vec<String>,
}

struct Entry {
    rel: String,
    path: PathBuf,
    size: u64,
    exec: bool,
    hash: String,
}

struct Tree {
    files: Vec<Entry>,
    notices: Vec<String>,
}

enum TreeRead {
    Tree(Tree),
    TooLarge,
}

impl Tree {
    fn manifest(&self) -> String {
        self.files
            .iter()
            .map(|f| {
                format!(
                    "{} {} {} {}",
                    if f.exec { 'x' } else { '-' },
                    f.size,
                    f.hash,
                    f.rel
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

struct Walk {
    files: Vec<Entry>,
    notices: Vec<String>,
    total: u64,
    root: PathBuf,
}

fn ignored_file(name: &str) -> bool {
    IGNORED_FILES.contains(&name)
        || Path::new(name)
            .extension()
            .is_some_and(|e| IGNORED_EXTENSIONS.iter().any(|i| e == *i))
}

#[cfg(unix)]
fn is_executable(meta: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_: &fs::Metadata) -> bool {
    false
}

fn file_hash(path: &Path) -> Result<String> {
    let mut f = fs::File::open(path).map_err(|e| Error::io(path, e))?;
    let mut h = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf).map_err(|e| Error::io(path, e))?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

impl Walk {
    /// `false` when the skill is larger than the limit.
    fn dir(&mut self, dir: &Path, rel: &str, canonical: &mut Vec<PathBuf>) -> Result<bool> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .map_err(|e| Error::io(dir, e))?
            .filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().into_string().ok())
            .collect();
        names.sort();
        for name in names {
            let path = dir.join(&name);
            let shown = format!("{rel}{name}");
            let Ok(link_meta) = fs::symlink_metadata(&path) else {
                continue;
            };
            let (meta, here) = if link_meta.file_type().is_symlink() {
                let Ok(target) = fs::canonicalize(&path) else {
                    self.notices
                        .push(format!("symlink {shown} is broken and was skipped"));
                    continue;
                };
                if !target.starts_with(&self.root) {
                    self.notices.push(format!(
                        "symlink {shown} points outside the skill and was skipped"
                    ));
                    continue;
                }
                let Ok(meta) = fs::metadata(&target) else {
                    continue;
                };
                (meta, target)
            } else {
                let here = canonical
                    .last()
                    .map_or_else(PathBuf::new, |c| c.join(&name));
                (link_meta, here)
            };
            if meta.is_dir() {
                if IGNORED_DIRS.contains(&name.as_str()) {
                    continue;
                }
                if canonical.contains(&here) {
                    self.notices
                        .push(format!("symlink {shown} loops and was skipped"));
                    continue;
                }
                canonical.push(here);
                let fits = self.dir(&path, &format!("{shown}/"), canonical)?;
                canonical.pop();
                if !fits {
                    return Ok(false);
                }
            } else if meta.is_file() && !ignored_file(&name) {
                self.total += meta.len();
                if self.total > MAX_SKILL_BYTES {
                    return Ok(false);
                }
                self.files.push(Entry {
                    hash: file_hash(&path)?,
                    rel: shown,
                    size: meta.len(),
                    exec: is_executable(&meta),
                    path,
                });
            }
        }
        Ok(true)
    }
}

/// Every regular file of the skill tree, sorted by relative path. Symlinks are followed only
/// when they resolve inside the skill.
fn read_tree(dir: &Path) -> Result<TreeRead> {
    let root = fs::canonicalize(dir).map_err(|e| Error::io(dir, e))?;
    let mut walk = Walk {
        files: Vec::new(),
        notices: Vec::new(),
        total: 0,
        root: root.clone(),
    };
    if !walk.dir(dir, "", &mut vec![root])? {
        return Ok(TreeRead::TooLarge);
    }
    walk.files.sort_by(|a, b| a.rel.cmp(&b.rel));
    Ok(TreeRead::Tree(Tree {
        files: walk.files,
        notices: walk.notices,
    }))
}

/// The `description` of the SKILL.md frontmatter; empty when there is none or it cannot be read.
fn description_of(text: &str) -> String {
    let mut lines = text.lines();
    if lines.next().map(str::trim_end) != Some("---") {
        return String::new();
    }
    let mut found: Option<String> = None;
    let mut block = false;
    for line in lines {
        if line.trim_end() == "---" {
            break;
        }
        if block {
            if line.starts_with([' ', '\t']) {
                if let Some(d) = found.as_mut() {
                    if !d.is_empty() {
                        d.push(' ');
                    }
                    d.push_str(line.trim());
                }
                continue;
            }
            block = false;
        }
        if let Some(value) = line.strip_prefix("description:") {
            let value = value.trim();
            if value.starts_with(['>', '|']) {
                block = true;
                found = Some(String::new());
            } else {
                found = Some(unquote(value).to_string());
            }
        }
    }
    found.unwrap_or_default().trim().to_string()
}

fn unquote(v: &str) -> &str {
    for q in ['"', '\''] {
        if let Some(inner) = v.strip_prefix(q).and_then(|r| r.strip_suffix(q)) {
            return inner;
        }
    }
    v
}

struct Scanned {
    item: Item,
    notices: Vec<String>,
}

/// The skill in `dir` as an item; `None` when it is too large.
fn read_skill(name: &str, dir: &Path) -> Result<Option<Scanned>> {
    let tree = match read_tree(dir)? {
        TreeRead::Tree(t) => t,
        TreeRead::TooLarge => return Ok(None),
    };
    let text = fs::read(dir.join(SKILL_FILE)).unwrap_or_default();
    Ok(Some(Scanned {
        item: Item {
            kind: Kind::Skill,
            scope: Scope::Global,
            name: name.to_string(),
            description: description_of(&String::from_utf8_lossy(&text)),
            category: None,
            body: tree.manifest(),
            source_dir: Some(dir.to_path_buf()),
        },
        notices: tree.notices,
    }))
}

fn valid_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.starts_with('.')
        || name == AGENT_MANAGED_DIR
        || name.contains(['/', '\\', '\0'])
    {
        return Err(Error::Parse(format!("not a skill name: {name:?}")));
    }
    Ok(())
}

fn is_skill_dir(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|m| m.is_dir()) && path.join(SKILL_FILE).is_file()
}

/// The skills below `<agent home>/skills` and the copies of them the agent also reads from
/// `visible_roots`.
pub struct SkillStore {
    agent: AgentId,
    home: PathBuf,
    dir: PathBuf,
    project_subdir: Option<PathBuf>,
    project_visible_roots: Vec<PathBuf>,
    state_dir: PathBuf,
    backups: BackupSession,
    visible_roots: Vec<PathBuf>,
}

impl SkillStore {
    pub fn new(agent: AgentId, home: PathBuf, state_dir: PathBuf, backups: BackupSession) -> Self {
        SkillStore {
            dir: home.join(SKILLS_DIR),
            project_subdir: None,
            project_visible_roots: Vec::new(),
            agent,
            home,
            state_dir,
            backups,
            visible_roots: Vec::new(),
        }
    }

    /// Directories the agent reads skills from besides its own; copies there are never written.
    pub fn with_visible_roots(mut self, roots: Vec<PathBuf>) -> Self {
        self.visible_roots = roots;
        self
    }

    /// Enable project-level skills. `subdir` is the directory inside a project the agent reads
    /// skills from (e.g. `.claude/skills`). `visible_roots` are other directories in the same
    /// project the agent also reads (e.g. `.agents/skills`); copies there are never written.
    pub fn with_project_skills(mut self, subdir: PathBuf, visible_roots: Vec<PathBuf>) -> Self {
        self.project_subdir = Some(subdir);
        self.project_visible_roots = visible_roots;
        self
    }

    /// Whether this store has a project skills directory configured.
    pub fn has_project(&self) -> bool {
        self.project_subdir.is_some()
    }

    fn names(&self) -> Vec<String> {
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        let mut names: Vec<String> = entries
            .filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| !n.starts_with('.') && n != AGENT_MANAGED_DIR)
            .collect();
        names.sort();
        names
    }

    pub(crate) fn scan_with_report(&self) -> Result<(Vec<Observed>, SkillReport)> {
        let mut out = Vec::new();
        let mut report = SkillReport::default();
        for name in self.names() {
            let path = self.dir.join(&name);
            if !is_skill_dir(&path) {
                continue;
            }
            let Some(scanned) = read_skill(&name, &path)? else {
                report.skipped.push((
                    name,
                    format!("larger than {} MB", MAX_SKILL_BYTES / 1024 / 1024),
                ));
                continue;
            };
            report
                .skipped
                .extend(scanned.notices.into_iter().map(|n| (name.clone(), n)));
            if self
                .visible_hash(&name)
                .is_some_and(|h| h != content_hash(&scanned.item))
            {
                report.shadowed.push(name.clone());
            }
            out.push(Observed {
                agent: self.agent.clone(),
                native_id: native_id(&name),
                path,
                item: scanned.item,
                replica_of: None,
            });
        }
        Ok((out, report))
    }

    pub fn scan(&self) -> Result<Vec<Observed>> {
        Ok(self.scan_with_report()?.0)
    }

    pub fn report(&self) -> Result<SkillReport> {
        Ok(self.scan_with_report()?.1)
    }

    /// Hash of the copy of skill `native_id` in the first visible root that holds one.
    pub fn visible_copy(&self, native_id: &str) -> Option<String> {
        self.visible_hash(skill_name(native_id)?)
    }

    fn visible_hash(&self, name: &str) -> Option<String> {
        valid_name(name).ok()?;
        let path = self
            .visible_roots
            .iter()
            .map(|r| r.join(name))
            .find(|p| is_skill_dir(p))?;
        read_skill(name, &path)
            .ok()
            .flatten()
            .map(|s| content_hash(&s.item))
    }

    /// Copy the files of `item.source_dir` into `<skills>/<name>`: into a temporary directory
    /// next to the target, then swapped in by rename, so a reader sees the old or the new skill
    /// and never a mix. An existing directory or symlink is backed up first and a symlink is
    /// replaced, never written through. Returns the native id and the skill directory.
    pub fn write(&self, item: &Item) -> Result<(String, PathBuf)> {
        self.write_into(&self.dir, &self.home, item)
    }

    /// Same as `write` but into a project skills directory.
    pub fn write_project(&self, cwd: &Path, item: &Item) -> Result<(String, PathBuf)> {
        let subdir = self
            .project_subdir
            .as_ref()
            .ok_or_else(|| Error::unsupported(self.agent.as_str(), "project skills"))?;
        let dir = cwd.join(subdir);
        let (_, path) = self.write_into(&dir, cwd, item)?;
        self.remember_project(cwd)?;
        let pk = project_key(cwd);
        Ok((project_native_id(&pk, &item.name), path))
    }

    fn write_into(&self, dir: &Path, base: &Path, item: &Item) -> Result<(String, PathBuf)> {
        let name = item.name.as_str();
        valid_name(name)?;
        let source = item
            .source_dir
            .as_deref()
            .ok_or_else(|| Error::Parse(format!("skill {name} has no source directory")))?;
        let tree = match read_tree(source)? {
            TreeRead::Tree(t) => t,
            TreeRead::TooLarge => {
                return Err(Error::Parse(format!(
                    "skill {name} is larger than {} MB",
                    MAX_SKILL_BYTES / 1024 / 1024
                )))
            }
        };
        let target = dir.join(name);
        let id = native_id(name);
        let existing = fs::symlink_metadata(&target).ok();
        if let Some(meta) = &existing {
            if !meta.file_type().is_symlink() {
                if let TreeRead::Tree(current) = read_tree(&target)? {
                    if current.manifest() == tree.manifest() {
                        return Ok((id, target));
                    }
                }
            }
            fsx::backup_dir(
                &self.backups,
                &self.state_dir,
                &self.backup_agent(base),
                base,
                &target,
            )?;
        }
        if !dir.exists() {
            fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
        }
        let pid = std::process::id();
        let tmp = dir.join(format!(".{name}.nativemem-tmp-{pid}"));
        let old = dir.join(format!(".{name}.nativemem-old-{pid}"));
        remove_any(&tmp);
        remove_any(&old);
        if let Err(e) = copy_files(&tree, &tmp) {
            remove_any(&tmp);
            return Err(e);
        }
        if let Err(e) = self.verify_and_swap(&tree, &tmp, &target, &old, existing.is_some()) {
            remove_any(&tmp);
            return Err(e);
        }
        remove_any(&old);
        Ok((id, target))
    }

    fn verify_and_swap(
        &self,
        tree: &Tree,
        tmp: &Path,
        target: &Path,
        old: &Path,
        replace: bool,
    ) -> Result<()> {
        match read_tree(tmp)? {
            TreeRead::Tree(copy) if copy.manifest() == tree.manifest() => {}
            _ => {
                return Err(Error::Parse(format!(
                    "skill {} changed while it was copied",
                    target.display()
                )))
            }
        }
        if replace {
            fs::rename(target, old).map_err(|e| Error::io(target, e))?;
        }
        if let Err(e) = fs::rename(tmp, target) {
            if replace {
                let _ = fs::rename(old, target);
            }
            return Err(Error::io(target, e));
        }
        Ok(())
    }

    /// Back up and delete `<skills>/<name>`; a symlink is removed, its target stays.
    pub fn remove(&self, name: &str) -> Result<()> {
        self.remove_from(&self.dir, &self.home, name)
    }

    fn remove_from(&self, dir: &Path, base: &Path, name: &str) -> Result<()> {
        valid_name(name)?;
        let target = dir.join(name);
        let meta = fs::symlink_metadata(&target).map_err(|e| Error::io(&target, e))?;
        fsx::backup_dir(
            &self.backups,
            &self.state_dir,
            &self.backup_agent(base),
            base,
            &target,
        )?;
        if meta.file_type().is_symlink() {
            fs::remove_file(&target).map_err(|e| Error::io(&target, e))
        } else {
            fs::remove_dir_all(&target).map_err(|e| Error::io(&target, e))
        }
    }

    /// Paths only, not skill contents. Copies in projects with no memory entries must remain
    /// discoverable on subsequent runs, even when this agent has never had a session there.
    fn known_projects(&self, cwds: &[PathBuf]) -> Result<Vec<PathBuf>> {
        let path = self.home.join("nativemem/skill-projects.json");
        let mut projects: std::collections::BTreeSet<PathBuf> = match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|e| Error::Parse(format!("invalid skill project index: {e}")))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Default::default(),
            Err(e) => return Err(Error::io(&path, e)),
        };
        projects.extend(cwds.iter().cloned());
        if projects.iter().any(|cwd| !cwd.is_absolute()) {
            return Err(Error::Parse("skill project paths must be absolute".into()));
        }
        Ok(projects.into_iter().collect())
    }

    fn remember_project(&self, cwd: &Path) -> Result<()> {
        let known = self.known_projects(&[])?;
        if known.iter().any(|p| p == cwd) {
            return Ok(());
        }
        let projects = self.known_projects(&[cwd.to_path_buf()])?;
        let path = self.home.join("nativemem/skill-projects.json");
        if path.exists() {
            fsx::backup(
                &self.backups,
                &self.state_dir,
                &self.agent,
                &self.home,
                &path,
            )?;
        }
        let bytes =
            serde_json::to_vec_pretty(&projects).map_err(|e| Error::Parse(e.to_string()))?;
        fsx::atomic_write(&path, &bytes)
    }

    fn backup_agent(&self, base: &Path) -> AgentId {
        if base == self.home {
            return self.agent.clone();
        }
        AgentId::new(format!(
            "{}/projects/{}",
            self.agent.as_str(),
            project_key(base)
        ))
    }

    /// Remove a project skill by its native id. Scans `cwds` for the matching project key.
    pub fn remove_project(&self, native_id: &str, cwds: &[PathBuf]) -> Result<()> {
        let subdir = self
            .project_subdir
            .as_ref()
            .ok_or_else(|| Error::unsupported(self.agent.as_str(), "project skills"))?;
        let pk = skill_project_key(native_id)
            .ok_or_else(|| Error::unsupported(self.agent.as_str(), "not a project skill id"))?;
        let name = skill_name(native_id)
            .ok_or_else(|| Error::Parse(format!("not a skill id: {native_id:?}")))?;
        let projects = self.known_projects(cwds)?;
        let cwd = projects
            .iter()
            .find(|c| project_key(c) == pk)
            .ok_or_else(|| Error::Parse(format!("project skill not found: {native_id:?}")))?;
        self.remove_from(&cwd.join(subdir), cwd, name)
    }

    /// Scan project skill directories for the given project paths.
    pub fn scan_projects(&self, cwds: &[PathBuf]) -> Result<Vec<Observed>> {
        let Some(ref subdir) = self.project_subdir else {
            return Ok(Vec::new());
        };
        let projects = self.known_projects(cwds)?;
        let global_dir = self.dir.canonicalize().ok();
        let mut out = Vec::new();
        for cwd in &projects {
            // Skip the agent home: its skills directory is the global one, already scanned.
            if let Some(g) = &global_dir {
                if cwd.join(subdir).canonicalize().ok().as_ref() == Some(g) {
                    continue;
                }
            }
            let dir = cwd.join(subdir);
            let Ok(entries) = fs::read_dir(&dir) else {
                continue;
            };
            let mut names: Vec<String> = entries
                .filter_map(|e| e.ok())
                .filter_map(|e| e.file_name().into_string().ok())
                .filter(|n| !n.starts_with('.') && n != AGENT_MANAGED_DIR)
                .collect();
            names.sort();
            for name in names {
                let path = dir.join(&name);
                if !is_skill_dir(&path) {
                    continue;
                }
                let Some(scanned) = read_skill(&name, &path)? else {
                    continue;
                };
                let pk = project_key(cwd);
                let item = Item {
                    scope: Scope::Project(cwd.clone()),
                    ..scanned.item
                };
                out.push(Observed {
                    agent: self.agent.clone(),
                    native_id: project_native_id(&pk, &name),
                    path,
                    item,
                    replica_of: None,
                });
            }
        }
        Ok(out)
    }

    /// Hash of a project skill copy in a visible root, or `None`.
    pub fn project_visible_copy(&self, native_id: &str, cwds: &[PathBuf]) -> Option<String> {
        let pk = skill_project_key(native_id)?;
        let name = skill_name(native_id)?;
        let projects = self.known_projects(cwds).ok()?;
        let cwd = projects.iter().find(|c| project_key(c) == pk)?;
        self.project_visible_roots
            .iter()
            .map(|r| cwd.join(r).join(name))
            .find(|p| is_skill_dir(p))
            .and_then(|path| read_skill(name, &path).ok().flatten())
            .map(|s| content_hash(&s.item))
    }

    /// Whether the layout of the skills directory still looks like what this adapter expects.
    pub fn layout_check(&self) -> Check {
        let check = Check::new(
            &format!("{}.skills_layout", self.agent),
            &[Kind::Skill],
            "skills directory layout",
            SOURCES,
        );
        if !self.dir.exists() {
            return check.found(LAYOUT_EXPECTED, "absent", true);
        }
        if !self.dir.is_dir() {
            return check.found(LAYOUT_EXPECTED, "not a directory", false);
        }
        let dirs: Vec<PathBuf> = self
            .names()
            .into_iter()
            .map(|n| self.dir.join(n))
            .filter(|p| fs::metadata(p).is_ok_and(|m| m.is_dir()))
            .collect();
        let skills = dirs.iter().filter(|p| p.join(SKILL_FILE).is_file()).count();
        let (observed, ok) = if dirs.is_empty() {
            ("empty", true)
        } else if skills * 2 >= dirs.len() {
            ("most directories hold a top-level SKILL.md", true)
        } else {
            ("few directories hold a top-level SKILL.md", false)
        };
        check.found(LAYOUT_EXPECTED, observed, ok)
    }
}

fn remove_any(path: &Path) {
    match fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() => {
            let _ = fs::remove_dir_all(path);
        }
        Ok(_) => {
            let _ = fs::remove_file(path);
        }
        Err(_) => {}
    }
}

fn copy_files(tree: &Tree, dest: &Path) -> Result<()> {
    fs::create_dir(dest).map_err(|e| Error::io(dest, e))?;
    for f in &tree.files {
        let to = dest.join(&f.rel);
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
        }
        fs::copy(&f.path, &to).map_err(|e| Error::io(&f.path, e))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(root: &Path) -> SkillStore {
        let home = root.join("home");
        fs::create_dir_all(&home).unwrap();
        SkillStore::new(
            AgentId::new("claude"),
            home,
            root.join("state"),
            BackupSession::with_stamp("T"),
        )
    }

    fn put(dir: &Path, rel: &str, text: &str) {
        let p = dir.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    }

    fn skill(dir: &Path, name: &str) {
        put(
            &dir.join(name),
            "SKILL.md",
            &format!("---\nname: {name}\ndescription: Does {name}\n---\nbody\n"),
        );
    }

    fn names_of(observed: &[Observed]) -> std::collections::BTreeSet<String> {
        observed.iter().map(|o| o.item.name.clone()).collect()
    }

    fn hash(s: &SkillStore, name: &str) -> String {
        let o = s.scan().unwrap();
        content_hash(&o.iter().find(|o| o.item.name == name).unwrap().item)
    }

    #[test]
    fn project_skills_create_parents_deduplicate_scans_and_keep_backups_separate() {
        let t = tempfile::tempdir().unwrap();
        let s = store(t.path()).with_project_skills(".pi/skills".into(), vec![]);
        let source = t.path().join("source");
        skill(&source, "demo");
        let item = read_skill("demo", &source.join("demo"))
            .unwrap()
            .unwrap()
            .item;
        let a = t.path().join("a");
        let b = t.path().join("b");
        for (cwd, content) in [(&a, "fixture A"), (&b, "fixture B")] {
            s.write_project(cwd, &item).unwrap();
            put(&cwd.join(".pi/skills/demo"), "SKILL.md", content);
            s.write_project(cwd, &item).unwrap();
            let backup = s
                .state_dir
                .join("backups/T")
                .join(s.agent.as_str())
                .join("projects")
                .join(project_key(cwd))
                .join(".pi/skills/demo/SKILL.md");
            assert_eq!(fs::read_to_string(backup).unwrap(), content);
        }
        let found = s.scan_projects(&[a.clone(), a.clone(), b.clone()]).unwrap();
        assert_eq!(found.len(), 2);
        assert_ne!(found[0].native_id, found[1].native_id);
        assert_eq!(found[0].item.scope, Scope::Project(a));
    }

    #[test]
    fn description_formats() {
        assert_eq!(
            description_of("---\ndescription: plain text\n---\n"),
            "plain text"
        );
        assert_eq!(
            description_of("---\ndescription: \"quoted\"\n---\n"),
            "quoted"
        );
        assert_eq!(
            description_of("---\nname: a\ndescription: >\n  first\n  second\nother: x\n---\n"),
            "first second"
        );
        assert_eq!(description_of("no frontmatter"), "");
        assert_eq!(description_of("---\nname: a\n---\n"), "");
    }

    #[test]
    fn hash_ignores_noise_and_location_but_sees_content_and_exec_bit() {
        let t = tempfile::tempdir().unwrap();
        let s = store(t.path());
        skill(&s.dir, "demo-skill");
        let base = hash(&s, "demo-skill");
        let d = s.dir.join("demo-skill");
        put(&d, ".DS_Store", "x");
        put(&d, "scripts/__pycache__/a.cpython-312.pyc", "x");
        put(&d, "scripts/b.pyc", "x");
        put(&d, ".git/config", "x");
        put(&d, "node_modules/m/index.js", "x");
        assert_eq!(hash(&s, "demo-skill"), base);

        let t2 = tempfile::tempdir().unwrap();
        let other = store(t2.path());
        skill(&other.dir, "demo-skill");
        assert_eq!(hash(&other, "demo-skill"), base);

        put(&d, "references/a.md", "one");
        let with_ref = hash(&s, "demo-skill");
        assert_ne!(with_ref, base);
        put(&d, "references/a.md", "two");
        assert_ne!(hash(&s, "demo-skill"), with_ref);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let before = hash(&s, "demo-skill");
            let p = d.join("references/a.md");
            fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
            assert_ne!(hash(&s, "demo-skill"), before);
        }
    }

    #[test]
    fn scan_skips_excluded_directories() {
        let t = tempfile::tempdir().unwrap();
        let s = store(t.path());
        skill(&s.dir, "demo-skill");
        skill(&s.dir, ".system");
        skill(&s.dir, "synced");
        put(&s.dir.join("container"), "inner/SKILL.md", "x");
        fs::write(s.dir.join("loose.md"), "x").unwrap();
        let names = names_of(&s.scan().unwrap());
        assert_eq!(
            names,
            std::collections::BTreeSet::from(["demo-skill".to_string()])
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_inside_a_skill_follow_only_inside() {
        let t = tempfile::tempdir().unwrap();
        let s = store(t.path());
        skill(&s.dir, "demo-skill");
        let d = s.dir.join("demo-skill");
        fs::write(t.path().join("outside.txt"), "secret").unwrap();
        put(&d, "real.txt", "inside");
        std::os::unix::fs::symlink(t.path().join("outside.txt"), d.join("out.txt")).unwrap();
        std::os::unix::fs::symlink("real.txt", d.join("in.txt")).unwrap();
        let (observed, report) = s.scan_with_report().unwrap();
        let manifest = &observed[0].item.body;
        assert!(manifest.contains(" in.txt"));
        assert!(manifest.contains(" real.txt"));
        assert!(!manifest.contains("out.txt"));
        assert_eq!(report.skipped.len(), 1);
        assert!(report.skipped[0].1.contains("outside"));
    }

    #[test]
    fn write_copies_atomically_and_leaves_no_temporary_directory() {
        let t = tempfile::tempdir().unwrap();
        let src = store(&t.path().join("a"));
        let dst = store(&t.path().join("b"));
        skill(&src.dir, "demo-skill");
        put(&src.dir.join("demo-skill"), "scripts/run.sh", "#!/bin/sh\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let p = src.dir.join("demo-skill/scripts/run.sh");
            fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let item = src.scan().unwrap().remove(0).item;
        dst.write(&item).unwrap();
        assert_eq!(hash(&dst, "demo-skill"), hash(&src, "demo-skill"));
        let names: Vec<String> = fs::read_dir(&dst.dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names, vec!["demo-skill".to_string()]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(dst.dir.join("demo-skill/scripts/run.sh"))
                .unwrap()
                .permissions()
                .mode();
            assert_ne!(mode & 0o111, 0);
        }
    }

    #[test]
    fn overwrite_and_remove_back_up_the_whole_directory() {
        let t = tempfile::tempdir().unwrap();
        let src = store(&t.path().join("a"));
        let dst = store(&t.path().join("b"));
        skill(&src.dir, "demo-skill");
        skill(&dst.dir, "demo-skill");
        put(&dst.dir.join("demo-skill"), "references/old.md", "old");
        let item = src.scan().unwrap().remove(0).item;
        dst.write(&item).unwrap();
        let backup = dst
            .state_dir
            .join("backups/T/claude/skills/demo-skill/references/old.md");
        assert_eq!(fs::read_to_string(backup).unwrap(), "old");
        assert!(!dst.dir.join("demo-skill/references/old.md").exists());
        dst.remove("demo-skill").unwrap();
        assert!(!dst.dir.join("demo-skill").exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_skill_is_read_through_and_replaced_not_written_through() {
        let t = tempfile::tempdir().unwrap();
        let shared = t.path().join("shared");
        skill(&shared, "demo-skill");
        let dst = store(&t.path().join("b"));
        fs::create_dir_all(&dst.dir).unwrap();
        std::os::unix::fs::symlink(shared.join("demo-skill"), dst.dir.join("demo-skill")).unwrap();
        assert_eq!(dst.scan().unwrap().len(), 1);

        let src = store(&t.path().join("a"));
        skill(&src.dir, "demo-skill");
        put(&src.dir.join("demo-skill"), "references/new.md", "new");
        let item = src.scan().unwrap().remove(0).item;
        dst.write(&item).unwrap();
        assert!(!fs::symlink_metadata(dst.dir.join("demo-skill"))
            .unwrap()
            .file_type()
            .is_symlink());
        assert!(!shared.join("demo-skill/references/new.md").exists());
        let record = dst.state_dir.join("backups/T/claude/skills/demo-skill");
        assert!(fs::read_to_string(record).unwrap().contains("symlink ->"));
    }

    #[test]
    fn visible_copy_hash_matches_an_identical_tree() {
        let t = tempfile::tempdir().unwrap();
        let shared = t.path().join("shared");
        let s = store(t.path()).with_visible_roots(vec![shared.clone()]);
        skill(&s.dir, "demo-skill");
        skill(&shared, "demo-skill");
        assert_eq!(
            s.visible_copy(&native_id("demo-skill")),
            Some(hash(&s, "demo-skill"))
        );
        assert!(s.report().unwrap().shadowed.is_empty());
        put(&shared.join("demo-skill"), "extra.md", "x");
        assert_ne!(
            s.visible_copy(&native_id("demo-skill")),
            Some(hash(&s, "demo-skill"))
        );
        assert_eq!(s.report().unwrap().shadowed, vec!["demo-skill".to_string()]);
    }

    #[test]
    fn layout_check_reports_shares() {
        let t = tempfile::tempdir().unwrap();
        let s = store(t.path());
        assert!(s.layout_check().ok);
        skill(&s.dir, "a");
        skill(&s.dir, "b");
        assert!(s.layout_check().ok);
        for n in ["c", "d", "e"] {
            fs::create_dir_all(s.dir.join(n)).unwrap();
        }
        let c = s.layout_check();
        assert!(!c.ok);
        assert_eq!(c.id, "claude.skills_layout");
    }

    #[test]
    fn names_are_validated() {
        for bad in ["", ".system", "synced", "a/b", "..", "a\\b"] {
            assert!(valid_name(bad).is_err(), "{bad}");
        }
        assert!(valid_name("demo-skill").is_ok());
        assert_eq!(skill_name("skill:demo-skill"), Some("demo-skill"));
        assert_eq!(skill_name("-w-app/x.md"), None);
    }
}
