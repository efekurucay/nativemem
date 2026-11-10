//! OpenCode's project identity for a working directory: which `project` row a session belongs to,
//! the worktree it records and the session's `path` below it. The rules are the ones observed on
//! OpenCode's own rows (`wiki/agents/opencode.md`): the id cached in `<git common dir>/opencode`,
//! else SHA-1 of `git-remote:<normalised origin url>`, else the first root commit; a directory
//! without a version-control marker is `directory:<path>`. Everything here only reads.

use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

/// The `project` row and the session `path` of one working directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    pub id: String,
    pub worktree: PathBuf,
    pub vcs: Option<&'static str>,
    /// The working directory relative to the directory that holds the version-control marker;
    /// empty at the root.
    pub path: String,
}

/// SHA-1 as 40 lowercase hex digits. OpenCode ids are SHA-1; the crate set has no SHA-1.
pub fn sha1_hex(data: &[u8]) -> String {
    let mut h: [u32; 5] = [
        0x6745_2301,
        0xEFCD_AB89,
        0x98BA_DCFE,
        0x1032_5476,
        0xC3D2_E1F0,
    ];
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for block in msg.chunks(64) {
        let mut w = [0u32; 80];
        for (i, word) in block.chunks(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5A82_7999),
                20..=39 => (b ^ c ^ d, 0x6ED9_EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
                _ => (b ^ c ^ d, 0xCA62_C1D6),
            };
            let t = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = t;
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e]) {
            *x = x.wrapping_add(y);
        }
    }
    h.iter().map(|x| format!("{x:08x}")).collect()
}

/// `host/path` of a remote URL: host lowercased, no leading `/`, no trailing `/` or `.git`.
/// `file:` remotes and local paths have none.
pub fn normalise_remote(url: &str) -> Option<String> {
    let url = url.trim();
    if url.is_empty() || url.starts_with("file:") {
        return None;
    }
    let (host, path) = if let Some((_, rest)) = url.split_once("://") {
        let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
        let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
        (host.split(':').next()?, path)
    } else {
        let (user_host, path) = url.split_once(':')?;
        let (_, host) = user_host.split_once('@')?;
        (host, path)
    };
    let path = path.trim_start_matches('/').trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let path = path.trim_end_matches('/');
    (!host.is_empty()).then(|| format!("{}/{path}", host.to_lowercase()))
}

/// The `url` of `[remote "origin"]` in a git config.
fn origin_url(config: &str) -> Option<String> {
    let mut in_origin = false;
    for line in config.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_origin = line == "[remote \"origin\"]";
        } else if in_origin {
            if let Some((key, value)) = line.split_once('=') {
                if key.trim().eq_ignore_ascii_case("url") {
                    return Some(value.trim().trim_matches('"').to_string());
                }
            }
        }
    }
    None
}

/// `path` without `.` and `..` components, resolved by text so no symlink changes its spelling.
fn lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// The common git directory of the repository whose marker is `dot_git`, and the working tree
/// that owns it. A `.git` file (linked worktree, submodule) points at the git directory, and a
/// linked worktree's `commondir` file at the shared one.
fn common_dir(root: &Path, dot_git: &Path) -> Option<(PathBuf, PathBuf)> {
    if dot_git.is_dir() {
        return Some((dot_git.to_path_buf(), root.to_path_buf()));
    }
    let text = fs::read_to_string(dot_git).ok()?;
    let target = text.lines().next()?.strip_prefix("gitdir:")?.trim();
    let git_dir = root.join(target);
    let common = match fs::read_to_string(git_dir.join("commondir")) {
        Ok(c) => lexical(&git_dir.join(c.trim())),
        Err(_) => lexical(&git_dir),
    };
    let owner = match (common.file_name(), common.parent()) {
        (Some(n), Some(parent)) if n == ".git" => parent.to_path_buf(),
        _ => root.to_path_buf(),
    };
    Some((common, owner))
}

/// The first root commit in sorted order. Runs `git rev-list`, which only reads the repository.
fn first_root_commit(root: &Path) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-list", "--max-parents=0", "HEAD"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    let text = String::from_utf8(out.stdout).ok()?;
    let mut roots: Vec<&str> = text.split_whitespace().collect();
    roots.sort_unstable();
    roots.first().map(|s| s.to_string())
}

fn git_id(common: &Path, root: &Path) -> Option<String> {
    if let Ok(cached) = fs::read_to_string(common.join("opencode")) {
        let cached = cached.trim();
        if !cached.is_empty() && cached.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Some(cached.to_string());
        }
    }
    if let Some(origin) = fs::read_to_string(common.join("config"))
        .ok()
        .and_then(|c| origin_url(&c))
        .and_then(|u| normalise_remote(&u))
    {
        return Some(sha1_hex(format!("git-remote:{origin}").as_bytes()));
    }
    first_root_commit(root)
}

fn relative(root: &Path, cwd: &Path) -> String {
    cwd.strip_prefix(root)
        .map(|rel| {
            rel.components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/")
        })
        .unwrap_or_default()
}

/// The project of `cwd`. A directory that does not exist, or has no `.git` or `.hg` marker on
/// its path, is its own project. `None` for a Mercurial repository, or a git repository whose
/// id cannot be derived: the caller leaves such a session alone rather than guess.
pub fn resolve(cwd: &Path) -> Option<Project> {
    if cwd.is_dir() {
        for dir in cwd.ancestors() {
            let dot_git = dir.join(".git");
            if dot_git.exists() {
                let (common, owner) = common_dir(dir, &dot_git)?;
                return Some(Project {
                    id: git_id(&common, dir)?,
                    worktree: owner,
                    vcs: Some("git"),
                    path: relative(dir, cwd),
                });
            }
            if dir.join(".hg").exists() {
                return None;
            }
        }
    }
    Some(Project {
        id: sha1_hex(format!("directory:{}", cwd.display()).as_bytes()),
        worktree: cwd.to_path_buf(),
        vcs: None,
        path: String::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo(root: &Path, config: &str) {
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::write(root.join(".git/config"), config).unwrap();
    }

    #[test]
    fn sha1_matches_the_published_vectors() {
        assert_eq!(sha1_hex(b""), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(sha1_hex(b"abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(
            sha1_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "84983e441c3bd26ebaae4aa1f95129e5e54670f1"
        );
        assert_eq!(sha1_hex(&[b'a'; 1000]).len(), 40);
    }

    #[test]
    fn remotes_normalise_to_host_and_path() {
        for (url, want) in [
            ("https://Example.com/acme/app.git", "example.com/acme/app"),
            ("git@example.com:acme/app.git", "example.com/acme/app"),
            ("ssh://git@Example.com:22/acme/app/", "example.com/acme/app"),
            (
                "https://user:pw@example.com/acme/app",
                "example.com/acme/app",
            ),
        ] {
            assert_eq!(normalise_remote(url).as_deref(), Some(want), "{url}");
        }
        for url in ["file:///srv/app.git", "/srv/app.git", ""] {
            assert_eq!(normalise_remote(url), None, "{url}");
        }
    }

    #[test]
    fn a_directory_without_a_marker_is_its_own_project() {
        let t = tempfile::tempdir().unwrap();
        let p = resolve(t.path()).unwrap();
        assert_eq!(
            p.id,
            sha1_hex(format!("directory:{}", t.path().display()).as_bytes())
        );
        assert_eq!(
            (p.worktree.as_path(), p.vcs, p.path.as_str()),
            (t.path(), None, "")
        );
    }

    #[test]
    fn a_missing_directory_uses_the_directory_rule() {
        let gone = Path::new("/nonexistent/acme/app");
        let p = resolve(gone).unwrap();
        assert_eq!(p.id, sha1_hex(b"directory:/nonexistent/acme/app"));
        assert_eq!(p.worktree, gone);
    }

    #[test]
    fn a_repository_with_an_origin_hashes_the_normalised_remote() {
        let t = tempfile::tempdir().unwrap();
        repo(
            t.path(),
            "[core]\n\tbare = false\n[remote \"origin\"]\n\turl = git@Example.com:acme/app.git\n\tfetch = +refs/heads/*:refs/remotes/origin/*\n",
        );
        fs::create_dir_all(t.path().join("src/deep")).unwrap();
        let p = resolve(&t.path().join("src/deep")).unwrap();
        assert_eq!(p.id, sha1_hex(b"git-remote:example.com/acme/app"));
        assert_eq!(p.worktree, t.path());
        assert_eq!((p.vcs, p.path.as_str()), (Some("git"), "src/deep"));
        assert_eq!(resolve(t.path()).unwrap().path, "");
    }

    #[test]
    fn the_cached_id_wins_over_the_origin() {
        let t = tempfile::tempdir().unwrap();
        repo(
            t.path(),
            "[remote \"origin\"]\n\turl = https://example.com/a/b\n",
        );
        fs::write(t.path().join(".git/opencode"), "cafe0123\n").unwrap();
        assert_eq!(resolve(t.path()).unwrap().id, "cafe0123");
    }

    #[test]
    fn the_first_root_commit_is_the_last_resort() {
        let t = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            Command::new("git")
                .arg("-C")
                .arg(t.path())
                .args(["-c", "user.name=x", "-c", "user.email=x@example.com"])
                .args(args)
                .output()
                .expect("git is installed")
        };
        git(&["init", "-q"]);
        git(&["commit", "-q", "--allow-empty", "-m", "root"]);
        let head = String::from_utf8(git(&["rev-parse", "HEAD"]).stdout).unwrap();
        assert_eq!(resolve(t.path()).unwrap().id, head.trim());
    }

    #[test]
    fn an_empty_repository_without_origin_has_no_derivable_id() {
        let t = tempfile::tempdir().unwrap();
        repo(t.path(), "[core]\n");
        assert_eq!(resolve(t.path()), None);
    }

    #[test]
    fn a_linked_worktree_belongs_to_the_project_of_the_main_repository() {
        let t = tempfile::tempdir().unwrap();
        let main = t.path().join("main");
        repo(
            &main,
            "[remote \"origin\"]\n\turl = https://example.com/a/b.git\n",
        );
        let admin = main.join(".git/worktrees/w1");
        fs::create_dir_all(&admin).unwrap();
        fs::write(admin.join("commondir"), "../..\n").unwrap();
        let linked = t.path().join("linked");
        fs::create_dir_all(&linked).unwrap();
        fs::write(
            linked.join(".git"),
            format!("gitdir: {}\n", admin.display()),
        )
        .unwrap();
        let p = resolve(&linked).unwrap();
        assert_eq!(p.id, sha1_hex(b"git-remote:example.com/a/b"));
        assert_eq!(p.worktree, main);
        assert_eq!(p.path, "");
    }

    #[test]
    fn mercurial_is_left_alone() {
        let t = tempfile::tempdir().unwrap();
        fs::create_dir_all(t.path().join(".hg")).unwrap();
        assert_eq!(resolve(t.path()), None);
    }
}
