//! The compatibility probes against fixtures built by the testkit, in temp directories only.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use nativemem_core::adapters::agents_file::AgentsFileAdapter;
use nativemem_core::adapters::claude::ClaudeAdapter;
use nativemem_core::adapters::codex::CodexAdapter;
use nativemem_core::adapters::pi::PiAdapter;
use nativemem_core::compat::Check;
use nativemem_core::fsx::BackupSession;
use nativemem_core::model::{Adapter, Kind, Scope};
use nativemem_core::testkit::{ClaudeHome, CodexHome, FrontmatterVariant, PiHome};

const ID: &str = "01a1c682-fd13-7263-8871-4ca47590d7d1";

fn stamp() -> BackupSession {
    BackupSession::with_stamp("T")
}

fn failing(checks: &[Check]) -> Vec<&str> {
    checks
        .iter()
        .filter(|c| !c.ok)
        .map(|c| c.id.as_str())
        .collect()
}

fn claude(dir: &Path) -> (ClaudeAdapter, PathBuf) {
    let h = ClaudeHome::new(&dir.join("claude"));
    let cwd = Path::new("/work/app");
    let session = h.add_session(cwd, "s1", Some("t"), &[("user", "hi"), ("assistant", "yo")]);
    let project = h.add_project(cwd);
    h.add_memory(
        &project,
        "a.md",
        FrontmatterVariant::Nested,
        "a",
        "d",
        "user",
        "b",
    );
    h.add_memory(
        &project,
        "b.md",
        FrontmatterVariant::Flat,
        "b",
        "d",
        "feedback",
        "b",
    );
    (
        ClaudeAdapter::new(h.root.clone(), dir.join("state"), stamp()),
        session,
    )
}

#[test]
fn claude_probes_pass_on_the_fixtures_and_read_the_newest_version() {
    let t = tempfile::tempdir().unwrap();
    let (a, session) = claude(t.path());
    assert_eq!(failing(&a.probes()), Vec::<&str>::new());
    assert_eq!(a.probes().len(), 6);
    assert_eq!(a.version(), None);
    let mut f = fs::OpenOptions::new().append(true).open(session).unwrap();
    writeln!(f, r#"{{"type":"system","version":"2.1.9"}}"#).unwrap();
    assert_eq!(a.version().as_deref(), Some("2.1.9"));
}

#[test]
fn claude_drift_fails_the_matching_check_and_pauses_only_its_kinds() {
    let t = tempfile::tempdir().unwrap();
    let (a, session) = claude(t.path());
    let mut f = fs::OpenOptions::new().append(true).open(&session).unwrap();
    writeln!(f, r#"{{"type":"brand-new-line"}}"#).unwrap();
    writeln!(f, r#"{{"type":"user","message":{{"role":"user"}}}}"#).unwrap();
    let checks = a.probes();
    assert_eq!(
        failing(&checks),
        vec!["claude.line_types", "claude.message_keys"]
    );
    let types = checks.iter().find(|c| c.id == "claude.line_types").unwrap();
    assert_eq!(types.observed, "unknown types: brand-new-line");
    assert_eq!(types.feature_kinds, vec![Kind::Session]);
    assert!(!types.sources.is_empty());

    let blocked = ClaudeAdapter::new(t.path().join("claude"), t.path().join("state"), stamp())
        .with_blocked_kinds(&[Kind::Session]);
    let project = Scope::Project("/work/app".into());
    assert!(!blocked.supports(Kind::Session, &project));
    assert!(blocked.supports(Kind::Memory, &project));
    assert!(blocked.supports(Kind::Instruction, &Scope::Global));
}

#[test]
fn claude_memory_and_encoding_drift_is_caught() {
    let t = tempfile::tempdir().unwrap();
    let (a, _) = claude(t.path());
    let mem = t.path().join("claude/projects/-work-app/memory");
    fs::write(
        mem.join("c.md"),
        "---\nname: c\ndescription: d\nbrand_new: 1\n---\n\nx\n",
    )
    .unwrap();
    let mut index = fs::OpenOptions::new()
        .append(true)
        .open(mem.join("MEMORY.md"))
        .unwrap();
    writeln!(index, "- [c](c.md) - dash instead of em dash").unwrap();
    let encoded = t.path().join("claude/projects/_work_app");
    fs::create_dir_all(&encoded).unwrap();
    fs::write(encoded.join("s.jsonl"), "{\"cwd\":\"/work/app\"}\n").unwrap();
    let checks = a.probes();
    assert_eq!(
        failing(&checks),
        vec![
            "claude.memory_frontmatter",
            "claude.memory_index",
            "claude.project_dir_encoding"
        ]
    );
    let frontmatter = checks
        .iter()
        .find(|c| c.id == "claude.memory_frontmatter")
        .unwrap();
    assert!(frontmatter
        .observed
        .contains("unknown top-level: brand_new"));
    // Free-form metadata keys are not drift.
    fs::write(
        mem.join("c.md"),
        "---\nname: c\ndescription: d\nmetadata:\n  type: user\n  anything: 1\n---\n\nx\n",
    )
    .unwrap();
    assert!(!failing(&a.probes()).contains(&"claude.memory_frontmatter"));
}

fn codex(dir: &Path) -> (CodexAdapter, CodexHome) {
    let h = CodexHome::new(&dir.join("codex"));
    h.add_rollout(
        ID,
        Path::new("/work/app"),
        &[("user", "hi"), ("assistant", "yo")],
        false,
    );
    h.write_state_db(5, &[]);
    h.write_import_ledger(&[ID]);
    h.write_version("0.167.0");
    (
        CodexAdapter::new(h.root.clone(), dir.join("state"), stamp()),
        h,
    )
}

#[test]
fn codex_probes_pass_on_the_fixtures() {
    let t = tempfile::tempdir().unwrap();
    let (a, h) = codex(t.path());
    assert_eq!(failing(&a.probes()), Vec::<&str>::new());
    assert_eq!(a.probes().len(), 6);
    assert_eq!(a.version().as_deref(), Some("0.167.0"));
    fs::remove_file(h.root.join("version.json")).unwrap();
    assert_eq!(a.version(), None);
    // No database, ledger or rollout is no drift.
    let bare = CodexHome::new(&t.path().join("bare"));
    fs::create_dir_all(&bare.root).unwrap();
    let b = CodexAdapter::new(bare.root.clone(), t.path().join("state"), stamp());
    assert_eq!(failing(&b.probes()), Vec::<&str>::new());
}

#[test]
fn codex_version_falls_back_to_the_newest_rollout() {
    let t = tempfile::tempdir().unwrap();
    let (a, h) = codex(t.path());
    fs::remove_file(h.root.join("version.json")).unwrap();
    let rollout = fs::read_dir(h.root.join("sessions/2026/11/10"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let text = fs::read_to_string(&rollout).unwrap().replacen(
        "\"source\":\"cli\"",
        "\"source\":\"cli\",\"cli_version\":\"0.9.0\"",
        1,
    );
    fs::write(&rollout, text).unwrap();
    assert_eq!(a.version().as_deref(), Some("0.9.0"));
}

#[test]
fn codex_drift_is_caught_and_the_database_is_never_written() {
    let t = tempfile::tempdir().unwrap();
    let (a, h) = codex(t.path());
    let db = h.root.join("state_5.sqlite");
    h.write_state_db(6, &[]);
    let checks = a.probes();
    assert_eq!(failing(&checks), vec!["codex.state_db"]);
    fs::remove_file(h.root.join("state_6.sqlite")).unwrap();

    fs::remove_file(&db).unwrap();
    h.write_state_db(5, &["brand_new INTEGER"]);
    let checks = a.probes();
    assert_eq!(failing(&checks), vec!["codex.threads_columns"]);
    let c = checks
        .iter()
        .find(|c| c.id == "codex.threads_columns")
        .unwrap();
    assert!(
        c.observed.contains("44 columns") && c.observed.contains("added: brand_new"),
        "{}",
        c.observed
    );
    assert!(c.expected.contains("43 columns"));
    let names: Vec<String> = fs::read_dir(&h.root)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        names
            .iter()
            .all(|n| !n.ends_with("-wal") && !n.ends_with("-journal") && !n.ends_with("-shm")),
        "{names:?}"
    );

    let again = fs::read(&db).unwrap();
    a.probes();
    assert_eq!(again, fs::read(&db).unwrap());

    let blocked = CodexAdapter::new(h.root.clone(), t.path().join("state"), stamp())
        .with_blocked_kinds(&[Kind::Session]);
    let project = Scope::Project("/work/app".into());
    assert!(!blocked.supports(Kind::Session, &project));
    assert!(blocked.supports(Kind::Memory, &project));
}

#[test]
fn codex_rollout_ledger_and_layout_drift_is_caught() {
    let t = tempfile::tempdir().unwrap();
    let (a, h) = codex(t.path());
    let ledger = h.root.join("external_agent_session_imports.json");
    let text = fs::read_to_string(&ledger)
        .unwrap()
        .replace("\"title\":\"t\"", "\"title\":\"t\",\"brand_new\":1");
    fs::write(&ledger, text).unwrap();
    let flat = h
        .root
        .join("sessions/rollout-2026-11-11T10-00-00-01a1c682-fd13-7263-8871-4ca47590d7d2.jsonl");
    fs::write(
        &flat,
        "{\"timestamp\":\"t\",\"type\":\"turn_context\",\"payload\":{}}\n",
    )
    .unwrap();
    let checks = a.probes();
    assert_eq!(
        failing(&checks),
        vec![
            "codex.session_meta_keys",
            "codex.import_ledger_keys",
            "codex.rollout_layout"
        ]
    );
    let ledger_check = checks
        .iter()
        .find(|c| c.id == "codex.import_ledger_keys")
        .unwrap();
    assert!(ledger_check.observed.contains("added keys: brand_new"));
}

#[cfg(unix)]
fn fake_executable(dir: &Path, name: &str, install: &str) -> std::ffi::OsString {
    let target = dir.join(install);
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(&target, "#!/bin/sh\n").unwrap();
    let bin = dir.join("bin");
    fs::create_dir_all(&bin).unwrap();
    std::os::unix::fs::symlink(&target, bin.join(name)).unwrap();
    bin.into_os_string()
}

#[cfg(unix)]
#[test]
fn pi_probes_pass_on_the_fixtures_and_read_the_package_version() {
    let t = tempfile::tempdir().unwrap();
    let h = PiHome::new(&t.path().join("pi"));
    h.add_session(
        Path::new("/work/app"),
        ID,
        Some("t"),
        &[("user", "hi"), ("assistant", "yo")],
    );
    let path = fake_executable(t.path(), "pi", "lib/pi-coding-agent/dist/cli.js");
    fs::write(
        t.path().join("lib/pi-coding-agent/package.json"),
        r#"{"name":"pi-coding-agent","version":"1.5.0"}"#,
    )
    .unwrap();
    let a = PiAdapter::new(h.root.clone(), t.path().join("state"), stamp()).with_search_path(path);
    assert_eq!(failing(&a.probes()), Vec::<&str>::new());
    assert_eq!(a.probes().len(), 4);
    assert_eq!(a.version().as_deref(), Some("1.5.0"));
    let without = PiAdapter::new(h.root.clone(), t.path().join("state"), stamp())
        .with_search_path("/nonexistent".into());
    assert_eq!(without.version(), None);
}

#[test]
fn pi_header_entry_and_directory_drift_is_caught() {
    let t = tempfile::tempdir().unwrap();
    let h = PiHome::new(&t.path().join("pi"));
    let file = h.add_session(Path::new("/work/app"), ID, None, &[("user", "hi")]);
    let a = PiAdapter::new(h.root.clone(), t.path().join("state"), stamp());
    let text = fs::read_to_string(&file).unwrap();
    fs::write(&file, text.replace("\"version\":3", "\"version\":4")).unwrap();
    let mut f = fs::OpenOptions::new().append(true).open(&file).unwrap();
    writeln!(f, r#"{{"type":"brand_new","id":"zz","parentId":null}}"#).unwrap();
    let checks = a.probes();
    assert_eq!(
        failing(&checks),
        vec!["pi.session_header", "pi.entry_types"]
    );
    assert!(checks[0].observed.starts_with("type=session, version=4"));

    let moved = h.root.join("sessions/elsewhere");
    fs::create_dir_all(&moved).unwrap();
    fs::rename(&file, moved.join("s.jsonl")).unwrap();
    assert!(failing(&a.probes()).contains(&"pi.cwd_encoding"));

    let blocked = a.with_blocked_kinds(&[Kind::Session]);
    assert!(!blocked.supports(Kind::Session, &Scope::Project("/work/app".into())));
    assert!(blocked.supports(Kind::Instruction, &Scope::Global));
}

#[cfg(unix)]
#[test]
fn opencode_probes_cover_the_config_dir_and_agents_file() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("opencode");
    fs::create_dir_all(&home).unwrap();
    let path = fake_executable(t.path(), "opencode", "Cellar/opencode/2.2.6/bin/opencode");
    let a = AgentsFileAdapter::new("opencode", home.clone(), t.path().join("state"), stamp())
        .with_search_path(path);
    assert_eq!(failing(&a.probes()), Vec::<&str>::new());
    assert_eq!(a.version().as_deref(), Some("2.2.6"));
    fs::create_dir(home.join("AGENTS.md")).unwrap();
    let checks = a.probes();
    assert_eq!(failing(&checks), vec!["opencode.agents_md"]);
    assert_eq!(
        checks[1].feature_kinds,
        vec![Kind::Instruction, Kind::Memory]
    );

    let blocked = a.with_blocked_kinds(&[Kind::Instruction, Kind::Memory]);
    assert!(!blocked.supports(Kind::Memory, &Scope::Project("/work/app".into())));
    let pi_like = AgentsFileAdapter::new("pi", home, t.path().join("state"), stamp());
    assert_eq!(pi_like.probes().len(), 1);
    assert!(pi_like.version().is_none());
}
