//! A kind that is paused for an agent is neither read nor written, and what already exists for it
//! is left alone. Temp directories only.

use std::fs;
use std::path::{Path, PathBuf};

use nativemem_core::adapters::claude::ClaudeAdapter;
use nativemem_core::adapters::codex::CodexAdapter;
use nativemem_core::engine::{self, ApplyOptions};
use nativemem_core::fsx::BackupSession;
use nativemem_core::ledger::Ledger;
use nativemem_core::model::{Adapter, Kind};
use nativemem_core::testkit::{ClaudeHome, CodexHome};

fn stamp() -> BackupSession {
    BackupSession::with_stamp("T")
}

fn replica(claude: &Path) -> PathBuf {
    fs::read_dir(claude.join("projects/-w-app/memory"))
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.file_name().is_some_and(|n| n != "MEMORY.md")
                && p.extension().is_some_and(|e| e == "md")
        })
        .unwrap()
}

#[test]
fn paused_codex_memory_leaves_existing_replicas_untouched() {
    let t = tempfile::tempdir().unwrap();
    let claude = t.path().join("claude");
    let codex = t.path().join("codex");
    ClaudeHome::new(&claude).add_project(Path::new("/w/app"));
    CodexHome::new(&codex)
        .write_memory_md("# Task Group: Fix Login\napplies_to: cwd=/w/app; reuse_rule=x\nnotes\n");
    let state = t.path().join("state");
    let ledger = Ledger::open(&state).unwrap();
    let claude_adapter = || ClaudeAdapter::new(claude.clone(), state.clone(), stamp());
    let codex_adapter = |blocked: &[Kind]| {
        CodexAdapter::new(codex.clone(), state.clone(), stamp()).with_blocked_kinds(blocked)
    };

    let (c, x) = (claude_adapter(), codex_adapter(&[]));
    let adapters: Vec<&dyn Adapter> = vec![&c, &x];
    let plan = engine::plan(&adapters, &ledger).unwrap();
    assert_eq!(plan.actions.len(), 1);
    engine::apply(
        plan,
        &adapters,
        &ledger,
        &ApplyOptions {
            allow_mass_delete: false,
        },
    )
    .unwrap();
    let file = replica(&claude);
    let mut text = fs::read_to_string(&file).unwrap();
    text.push_str("\nedited on the copy\n");
    fs::write(&file, &text).unwrap();
    let md = fs::read(codex.join("memories/MEMORY.md")).unwrap();

    let paused = codex_adapter(&[Kind::Memory]);
    assert!(paused.scan().unwrap().is_empty());
    let adapters: Vec<&dyn Adapter> = vec![&c, &paused];
    let plan = engine::plan(&adapters, &ledger).unwrap();
    assert!(plan.actions.is_empty() && plan.notes.is_empty(), "{plan}");
    engine::apply(
        plan,
        &adapters,
        &ledger,
        &ApplyOptions {
            allow_mass_delete: false,
        },
    )
    .unwrap();
    assert_eq!(fs::read_to_string(&file).unwrap(), text);
    assert_eq!(fs::read(codex.join("memories/MEMORY.md")).unwrap(), md);
    assert_eq!(ledger.list_items().unwrap().len(), 1);
}
