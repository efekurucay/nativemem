use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use nativemem_core::testkit::{ClaudeHome, CodexHome, FrontmatterVariant};

fn run(claude: &Path, codex: &Path, state: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_nativemem"))
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("CODEX_HOME")
        .arg("--claude-home")
        .arg(claude)
        .arg("--codex-home")
        .arg(codex)
        .arg("--state-dir")
        .arg(state)
        .args(args)
        .output()
        .unwrap()
}

fn text(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

#[test]
fn status_sync_apply_then_nothing_to_do() {
    let t = tempfile::tempdir().unwrap();
    let (claude, codex, state) = (
        t.path().join("claude"),
        t.path().join("codex"),
        t.path().join("state"),
    );
    fs::create_dir_all(&codex).unwrap();
    let ch = ClaudeHome::new(&claude);
    let dir = ch.add_project(Path::new("/w/app"));
    ch.add_memory(
        &dir,
        "a.md",
        FrontmatterVariant::Nested,
        "alpha",
        "about alpha",
        "user",
        "alpha body",
    );
    CodexHome::new(&codex)
        .write_memory_md("# Task Group: Fix Login\napplies_to: cwd=/w/app; reuse_rule=x\nnotes\n");

    let o = run(&claude, &codex, &state, &["status"]);
    assert_eq!(o.status.code(), Some(0));
    let out = text(&o);
    assert!(out.contains("claude: 1 originals, 0 replicas"), "{out}");
    assert!(out.contains("codex: 1 originals, 0 replicas"), "{out}");
    assert!(out.contains("ledger: 0 items"), "{out}");
    assert!(!state.exists());

    let o = run(&claude, &codex, &state, &["sync"]);
    assert_eq!(o.status.code(), Some(0));
    let out = text(&o);
    println!("{out}");
    assert!(
        out.contains("+ codex  /w/app  alpha   (from claude)"),
        "{out}"
    );
    assert!(
        out.contains("+ claude  /w/app  fix-login   (from codex)"),
        "{out}"
    );
    assert!(
        out.contains("2 to add, 0 to update, 0 to remove, 0 conflicts, 0 skipped"),
        "{out}"
    );
    assert!(!state.exists());
    assert!(!codex.join("AGENTS.md").exists());

    let o = run(&claude, &codex, &state, &["sync", "--apply"]);
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(codex.join("AGENTS.md").exists());
    assert!(state.join("ledger.db").exists());

    let o = run(&claude, &codex, &state, &["sync"]);
    assert_eq!(o.status.code(), Some(0));
    assert_eq!(
        text(&o).trim(),
        "0 to add, 0 to update, 0 to remove, 0 conflicts, 0 skipped"
    );

    let out = text(&run(&claude, &codex, &state, &["status"]));
    assert!(out.contains("claude: 1 originals, 1 replicas"), "{out}");
    assert!(out.contains("ledger: 2 items"), "{out}");
}
