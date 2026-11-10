//! Builders for fake agent homes. Callers pass a temp directory; nothing here touches real homes.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrontmatterVariant {
    Nested,
    Flat,
}

pub struct ClaudeHome {
    pub root: PathBuf,
}

impl ClaudeHome {
    pub fn new(dir: &Path) -> Self {
        ClaudeHome {
            root: dir.to_path_buf(),
        }
    }

    /// Creates `projects/<encoded>/` with a jsonl carrying `cwd` and an empty `memory/`.
    /// Returns the project directory name.
    pub fn add_project(&self, cwd: &Path) -> String {
        let name: String = cwd
            .to_string_lossy()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        let dir = self.root.join("projects").join(&name);
        fs::create_dir_all(dir.join("memory")).unwrap();
        let cwd_json = cwd
            .to_string_lossy()
            .replace('\\', "\\\\")
            .replace('"', "\\\"");
        fs::write(
            dir.join("session.jsonl"),
            format!("{{\"cwd\":\"{cwd_json}\"}}\n"),
        )
        .unwrap();
        name
    }

    #[allow(clippy::too_many_arguments)]
    /// Writes `memory/<filename>` and appends its line to `MEMORY.md`. Returns the file path.
    pub fn add_memory(
        &self,
        project_dir: &str,
        filename: &str,
        variant: FrontmatterVariant,
        name: &str,
        description: &str,
        category: &str,
        body: &str,
    ) -> PathBuf {
        let mem = self.root.join("projects").join(project_dir).join("memory");
        fs::create_dir_all(&mem).unwrap();
        let fm = match variant {
            FrontmatterVariant::Nested => format!(
                "name: {name}\ndescription: {description}\nmetadata:\n  type: {category}\n  node_type: memory\n  originSessionId: s-1\n  modified: 2026-01-01\n"
            ),
            FrontmatterVariant::Flat => format!(
                "name: {name}\ndescription: {description}\ntype: {category}\noriginSessionId: s-1\n"
            ),
        };
        let path = mem.join(filename);
        fs::write(&path, format!("---\n{fm}---\n\n{body}\n")).unwrap();

        let index = mem.join("MEMORY.md");
        if !index.exists() {
            fs::write(&index, "# Memory Index\n\n").unwrap();
        }
        let mut f = fs::OpenOptions::new().append(true).open(&index).unwrap();
        writeln!(f, "- [{name}]({filename}) — {description}").unwrap();
        path
    }
}

pub struct CodexHome {
    pub root: PathBuf,
}

impl CodexHome {
    pub fn new(dir: &Path) -> Self {
        CodexHome {
            root: dir.to_path_buf(),
        }
    }

    pub fn write_agents_md(&self, text: &str) {
        fs::create_dir_all(&self.root).unwrap();
        fs::write(self.root.join("AGENTS.md"), text).unwrap();
    }

    pub fn write_memory_md(&self, text: &str) {
        let dir = self.root.join("memories");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("MEMORY.md"), text).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_home_layout() {
        let t = tempfile::tempdir().unwrap();
        let h = ClaudeHome::new(t.path());
        let dir = h.add_project(Path::new("/work/my proj"));
        assert_eq!(dir, "-work-my-proj");
        let jsonl =
            fs::read_to_string(t.path().join("projects/-work-my-proj/session.jsonl")).unwrap();
        assert_eq!(jsonl, "{\"cwd\":\"/work/my proj\"}\n");
        let f = h.add_memory(
            &dir,
            "a.md",
            FrontmatterVariant::Flat,
            "a",
            "desc",
            "user",
            "body",
        );
        assert!(fs::read_to_string(f).unwrap().contains("type: user"));
        h.add_memory(
            &dir,
            "b.md",
            FrontmatterVariant::Nested,
            "b",
            "d2",
            "feedback",
            "x",
        );
        let idx =
            fs::read_to_string(t.path().join("projects/-work-my-proj/memory/MEMORY.md")).unwrap();
        assert_eq!(
            idx,
            "# Memory Index\n\n- [a](a.md) — desc\n- [b](b.md) — d2\n"
        );
    }

    #[test]
    fn codex_home_files() {
        let t = tempfile::tempdir().unwrap();
        let h = CodexHome::new(t.path());
        h.write_agents_md("hi");
        h.write_memory_md("m");
        assert_eq!(
            fs::read_to_string(t.path().join("AGENTS.md")).unwrap(),
            "hi"
        );
        assert_eq!(
            fs::read_to_string(t.path().join("memories/MEMORY.md")).unwrap(),
            "m"
        );
    }
}
