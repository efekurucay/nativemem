//! Read-only helpers the compatibility probes share. Nothing here writes, and nothing returns
//! message text: callers reduce what they read to structural descriptors.

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::Value;

/// The last `max` bytes of `path`. A cut inside a line drops that partial first line.
pub fn read_tail(path: &Path, max: u64) -> Option<String> {
    let mut file = File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let start = len.saturating_sub(max);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = Vec::new();
    file.take(max).read_to_end(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf).into_owned();
    if start == 0 {
        return Some(text);
    }
    text.split_once('\n').map(|(_, rest)| rest.to_string())
}

/// The first `max` bytes of `path`; a line cut by the limit is dropped.
pub fn read_head(path: &Path, max: u64) -> Option<String> {
    let mut file = File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let mut buf = Vec::new();
    file.by_ref().take(max).read_to_end(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf).into_owned();
    if len <= max {
        return Some(text);
    }
    text.rfind('\n').map(|i| text[..=i].to_string())
}

/// JSON values of the parsable lines of `text`; the rest is ignored.
pub fn json_lines(text: &str) -> Vec<Value> {
    text.lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .collect()
}

/// Files `levels` directories below `root` that satisfy `keep`, newest modification first.
pub fn newest_files(
    root: &Path,
    levels: usize,
    keep: &dyn Fn(&Path) -> bool,
    limit: usize,
) -> Vec<PathBuf> {
    let mut found: Vec<(SystemTime, PathBuf)> = Vec::new();
    collect(root, levels, keep, &mut found);
    found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
    found.into_iter().take(limit).map(|(_, p)| p).collect()
}

fn collect(
    dir: &Path,
    levels: usize,
    keep: &dyn Fn(&Path) -> bool,
    out: &mut Vec<(SystemTime, PathBuf)>,
) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if levels > 0 {
            if path.is_dir() {
                collect(&path, levels - 1, keep, out);
            }
        } else if path.is_file() && keep(&path) {
            if let Ok(modified) = entry.metadata().and_then(|m| m.modified()) {
                out.push((modified, path));
            }
        }
    }
}

/// File name of `path`, empty when it has none.
pub fn name_of(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Name of the directory that holds `path`.
pub fn parent_name(path: &Path) -> String {
    path.parent().map(name_of).unwrap_or_default()
}

/// `a, b, c` for a sorted, de-duplicated set; `none` when empty.
pub fn list(items: &BTreeSet<String>) -> String {
    if items.is_empty() {
        "none".to_string()
    } else {
        items.iter().cloned().collect::<Vec<_>>().join(", ")
    }
}

/// `a, b, c` for the given constant names, in the order given.
pub fn names(items: &[&str]) -> String {
    items.join(", ")
}

/// The first `name` file in the `search_path` directories, with symlinks resolved. The file is
/// only located, never run. `None` when `search_path` is `None` or holds no such file.
pub fn resolve_executable(name: &str, search_path: Option<&OsStr>) -> Option<PathBuf> {
    std::env::split_paths(search_path?)
        .map(|dir| dir.join(name))
        .find(|p| p.is_file())
        .and_then(|p| p.canonicalize().ok())
}

/// The `PATH` of this process.
pub fn process_path() -> Option<std::ffi::OsString> {
    std::env::var_os("PATH")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tail_drops_the_cut_line_and_head_drops_the_cut_line() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("f");
        fs::write(&p, "aaaa\nbbbb\ncccc\n").unwrap();
        assert_eq!(read_tail(&p, 100).unwrap(), "aaaa\nbbbb\ncccc\n");
        assert_eq!(read_tail(&p, 7).unwrap(), "cccc\n");
        assert_eq!(read_head(&p, 100).unwrap(), "aaaa\nbbbb\ncccc\n");
        assert_eq!(read_head(&p, 7).unwrap(), "aaaa\n");
    }

    #[test]
    fn newest_files_sorts_by_modification_time() {
        let t = tempfile::tempdir().unwrap();
        for (dir, file, age) in [("a", "old.jsonl", 5), ("b", "new.jsonl", 1)] {
            fs::create_dir_all(t.path().join(dir)).unwrap();
            let p = t.path().join(dir).join(file);
            fs::write(&p, "x").unwrap();
            crate::testkit::age_days(&p, age);
        }
        let got = newest_files(t.path(), 1, &|_| true, 5);
        assert_eq!(name_of(&got[0]), "new.jsonl");
        assert_eq!(got.len(), 2);
    }

    #[test]
    fn resolves_symlinked_executables_without_running_them() {
        let t = tempfile::tempdir().unwrap();
        let real = t.path().join("Cellar/tool/1.2.3/bin");
        fs::create_dir_all(&real).unwrap();
        fs::write(real.join("tool"), "#!/bin/sh\n").unwrap();
        let bin = t.path().join("bin");
        fs::create_dir_all(&bin).unwrap();
        std::os::unix::fs::symlink(real.join("tool"), bin.join("tool")).unwrap();
        let got = resolve_executable("tool", Some(bin.as_os_str())).unwrap();
        assert!(got.ends_with("Cellar/tool/1.2.3/bin/tool"));
        assert!(resolve_executable("absent", Some(bin.as_os_str())).is_none());
        assert!(resolve_executable("tool", None).is_none());
    }
}
