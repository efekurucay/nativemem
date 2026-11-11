use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use nativemem_core::testkit::{ClaudeHome, CodexHome, FrontmatterVariant, PiHome};

fn run(claude: &Path, codex: &Path, state: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_nativemem"))
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("CODEX_HOME")
        .env_remove("OPENCODE_CONFIG_DIR")
        .env_remove("PI_CODING_AGENT_DIR")
        .env("HOME", state.with_file_name("home"))
        .env_remove("NATIVEMEM_OPENCODE_HOME")
        .env_remove("NATIVEMEM_OPENCODE_DATA_HOME")
        .env_remove("XDG_DATA_HOME")
        .env_remove("NATIVEMEM_PI_HOME")
        .arg("--claude-home")
        .arg(claude)
        .arg("--codex-home")
        .arg(codex)
        .arg("--opencode-home")
        .arg(state.with_file_name("opencode"))
        .arg("--opencode-data-home")
        .arg(state.with_file_name("opencode-data"))
        .arg("--pi-home")
        .arg(state.with_file_name("pi"))
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
    assert!(
        out.contains("claude: 1 originals, 1 replicas, 0 sessions"),
        "{out}"
    );
    assert!(out.contains("ledger: 2 items"), "{out}");
}

use serde_json::{json, Value};

struct Fx {
    _t: tempfile::TempDir,
    claude: std::path::PathBuf,
    codex: std::path::PathBuf,
    state: std::path::PathBuf,
    claude_file: std::path::PathBuf,
}

impl Fx {
    fn new() -> Fx {
        let t = tempfile::tempdir().unwrap();
        let (claude, codex, state) = (
            t.path().join("claude"),
            t.path().join("codex"),
            t.path().join("state"),
        );
        fs::create_dir_all(&codex).unwrap();
        let ch = ClaudeHome::new(&claude);
        let dir = ch.add_project(Path::new("/w/app"));
        let claude_file = ch.add_memory(
            &dir,
            "a.md",
            FrontmatterVariant::Nested,
            "alpha",
            "about alpha",
            "user",
            "alpha body",
        );
        Fx {
            _t: t,
            claude,
            codex,
            state,
            claude_file,
        }
    }

    fn json(&self, args: &[&str]) -> (Option<i32>, Value) {
        let o = run(&self.claude, &self.codex, &self.state, args);
        let doc = serde_json::from_slice(&o.stdout).unwrap_or_else(|e| panic!("{e}: {}", text(&o)));
        (o.status.code(), doc)
    }
}

fn find<'a>(items: &'a Value, agent: &str) -> &'a Value {
    items["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["agent"] == agent)
        .unwrap_or_else(|| panic!("no {agent} item in {items}"))
}

#[test]
fn status_json_shape() {
    let f = Fx::new();
    let (code, doc) = f.json(&["status", "--json"]);
    assert_eq!(code, Some(0));
    assert_eq!(
        doc,
        json!({ "agents": [
            { "id": "claude", "originals": 1, "replicas": 0, "sessions": 0, "skills": 0, "automated_sessions": 0, "skipped": [] },
            { "id": "codex", "originals": 0, "replicas": 0, "sessions": 0, "skills": 0, "automated_sessions": 0, "skipped": [] } ],
          "ledger_items": 0,
          "compat": { "action_required": 0, "updated": 0, "agents": [
            { "id": "claude", "status": "ok", "notices": [] }, { "id": "codex", "status": "ok", "notices": [] },
            { "id": "opencode", "status": "not_installed", "notices": [] }, { "id": "pi", "status": "not_installed", "notices": [] } ] } })
    );
    assert!(!f.state.exists());
}

fn four_agents() -> Fx {
    let f = Fx::new();
    for name in ["opencode", "pi"] {
        fs::create_dir_all(f.state.with_file_name(name)).unwrap();
    }
    fs::write(f.claude.join("CLAUDE.md"), "# Mine\n\n## Style\nbe terse\n").unwrap();
    f
}

#[test]
fn instruction_sections_and_memory_reach_all_four_agents() {
    let f = four_agents();
    let (code, doc) = f.json(&["sync", "--apply", "--json"]);
    assert_eq!(code, Some(0), "{doc}");
    assert_eq!(doc["notes"], json!([]));
    assert_eq!(doc["summary"]["add"], 6);
    let (_, status) = f.json(&["status", "--json"]);
    let agents: Vec<(&str, u64, u64)> = status["agents"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| {
            (
                a["id"].as_str().unwrap(),
                a["originals"].as_u64().unwrap(),
                a["replicas"].as_u64().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        agents,
        [
            ("claude", 2, 0),
            ("codex", 0, 2),
            ("opencode", 0, 2),
            ("pi", 0, 2)
        ]
    );
    for name in ["opencode", "pi"] {
        let home = f.state.with_file_name(name);
        let agents = fs::read_to_string(home.join("AGENTS.md")).unwrap();
        assert!(agents.contains("be terse"));
        assert!(agents.contains("<!-- nativemem:begin -->\n## Project memory"));
        assert!(agents.contains("- /w/app → "));
        assert!(home.join("nativemem/memory").is_dir());
    }
    let (code, doc) = f.json(&["sync", "--json"]);
    assert_eq!(code, Some(0));
    assert_eq!(doc["actions"], json!([]));
}

#[test]
fn agent_without_a_home_directory_takes_no_part() {
    let f = Fx::new();
    fs::write(f.claude.join("CLAUDE.md"), "## Style\nbe terse\n").unwrap();
    f.json(&["sync", "--apply", "--json"]);
    assert!(!f.state.with_file_name("opencode").exists());
    assert!(!f.state.with_file_name("pi").exists());
    let (_, status) = f.json(&["status", "--json"]);
    assert_eq!(status["agents"].as_array().unwrap().len(), 2);
    let (code, doc) = f.json(&["delete", "--agent", "pi", "--id", "AGENTS.md#x", "--json"]);
    assert_eq!(code, Some(1));
    assert_eq!(doc["code"], "not_found");
}

#[test]
fn delete_and_edit_accept_opencode_and_pi() {
    let f = four_agents();
    for name in ["opencode", "pi"] {
        fs::write(
            f.state.with_file_name(name).join("AGENTS.md"),
            format!("## {name} rule\nold\n\n## Other\nkeep\n"),
        )
        .unwrap();
    }
    let body =
        r#"{"name":"opencode-rule","description":"opencode rule","category":null,"body":"new"}"#;
    let (code, doc) = edit(&f, "opencode", "AGENTS.md#opencode-rule", body);
    assert_eq!(code, Some(0), "{doc}");
    let file = f.state.with_file_name("opencode").join("AGENTS.md");
    assert!(fs::read_to_string(file)
        .unwrap()
        .contains("## opencode rule\nnew\n"));
    let (code, doc) = f.json(&[
        "delete",
        "--agent",
        "pi",
        "--id",
        "AGENTS.md#pi-rule",
        "--json",
    ]);
    assert_eq!(code, Some(0), "{doc}");
    let file = f.state.with_file_name("pi").join("AGENTS.md");
    assert_eq!(fs::read_to_string(file).unwrap(), "## Other\nkeep\n");
}

#[test]
fn items_json_before_and_after_apply() {
    let f = Fx::new();
    let (code, doc) = f.json(&["items", "--json"]);
    assert_eq!(code, Some(0));
    assert!(!f.state.exists());
    assert_eq!(doc["items"].as_array().unwrap().len(), 1);
    let orig = find(&doc, "claude");
    assert_eq!(orig["native_id"], "-w-app/a.md");
    assert_eq!(orig["kind"], "memory");
    assert_eq!(
        orig["scope"],
        json!({ "type": "project", "path": "/w/app" })
    );
    assert_eq!(orig["name"], "alpha");
    assert_eq!(orig["description"], "about alpha");
    assert_eq!(orig["category"], "user");
    assert!(orig["body"].as_str().unwrap().contains("alpha body"));
    assert!(orig["replica_of"].is_null());
    assert_eq!(orig["hash"].as_str().unwrap().len(), 64);
    assert_eq!(orig["path"], f.claude_file.to_str().unwrap());

    let (code, _) = f.json(&["sync", "--apply", "--json"]);
    assert_eq!(code, Some(0));
    let (_, doc) = f.json(&["items", "--json"]);
    assert_eq!(doc["items"].as_array().unwrap().len(), 2);
    let replica = find(&doc, "codex");
    assert_eq!(
        replica["replica_of"],
        json!({ "origin": "claude", "native_id": "-w-app/a.md" })
    );
    assert_eq!(replica["hash"], find(&doc, "claude")["hash"]);

    let o = run(&f.claude, &f.codex, &f.state, &["items"]);
    let out = text(&o);
    assert!(out.contains("claude  memory  /w/app  alpha"), "{out}");
    assert!(out.contains("codex  memory  /w/app  alpha"), "{out}");
}

#[test]
fn sync_json_preview_then_apply() {
    let f = Fx::new();
    let (code, doc) = f.json(&["sync", "--json"]);
    assert_eq!(code, Some(0));
    assert_eq!(
        doc,
        json!({
            "applied": false,
            "actions": [ { "op": "add", "agent": "codex", "name": "alpha",
                "scope": { "type": "project", "path": "/w/app" },
                "origin": { "origin": "claude", "native_id": "-w-app/a.md" } } ],
            "notes": [],
            "summary": { "add": 1, "update": 0, "remove": 0, "conflicts": 0, "skipped": 0 }
        })
    );
    assert!(!f.state.exists());

    let (code, doc) = f.json(&["sync", "--apply", "--json"]);
    assert_eq!(code, Some(0));
    assert_eq!(doc["applied"], true);
    assert_eq!(doc["actions"][0]["op"], "add");
    assert_eq!(doc["summary"]["add"], 1);

    let (code, doc) = f.json(&["sync", "--json"]);
    assert_eq!(code, Some(0));
    assert_eq!(doc["actions"], json!([]));
}

#[test]
fn delete_json_then_sync_removes_copy() {
    let f = Fx::new();
    f.json(&["sync", "--apply", "--json"]);
    let (_, items) = f.json(&["items", "--json"]);
    let replica_id = find(&items, "codex")["native_id"]
        .as_str()
        .unwrap()
        .to_string();

    let (code, doc) = f.json(&[
        "delete",
        "--agent",
        "claude",
        "--id",
        "-w-app/a.md",
        "--json",
    ]);
    assert_eq!(code, Some(0));
    assert_eq!(
        doc["deleted"],
        json!({ "agent": "claude", "native_id": "-w-app/a.md" })
    );
    let backup = Path::new(doc["backup_dir"].as_str().unwrap());
    assert!(backup.starts_with(f.state.join("backups")));
    assert!(backup.join("claude").exists());
    assert!(!f.claude_file.exists());

    let (code, doc) = f.json(&["sync", "--apply", "--json"]);
    assert_eq!(code, Some(0));
    assert_eq!(doc["actions"][0]["op"], "remove");
    assert_eq!(doc["actions"][0]["agent"], "codex");
    assert_eq!(doc["summary"]["remove"], 1);
    let (_, items) = f.json(&["items", "--json"]);
    assert_eq!(items["items"], json!([]), "{replica_id}");
}

#[test]
fn delete_unsupported_original_reports_error_document() {
    let f = Fx::new();
    CodexHome::new(&f.codex)
        .write_memory_md("# Task Group: Fix Login\napplies_to: cwd=/w/app; reuse_rule=x\nnotes\n");
    let (_, items) = f.json(&["items", "--json"]);
    let id = find(&items, "codex")["native_id"]
        .as_str()
        .unwrap()
        .to_string();
    let (code, doc) = f.json(&["delete", "--agent", "codex", "--id", &id, "--json"]);
    assert_eq!(code, Some(1));
    assert_eq!(doc["code"], "unsupported");
    assert!(doc["error"].is_string());
}

#[test]
fn unknown_agent_reports_error_document() {
    let f = Fx::new();
    let (code, doc) = f.json(&["delete", "--agent", "gemini", "--id", "x", "--json"]);
    assert_eq!(code, Some(1));
    assert_eq!(doc["code"], "unknown_agent");
    assert!(doc["error"].as_str().unwrap().contains("gemini"));
}

#[test]
fn mass_delete_reports_error_code() {
    let f = Fx::new();
    let ch = ClaudeHome::new(&f.claude);
    let dir = ch.add_project(Path::new("/w/app"));
    let mut files = Vec::new();
    for i in 0..5 {
        files.push(ch.add_memory(
            &dir,
            &format!("m{i}.md"),
            FrontmatterVariant::Nested,
            &format!("m{i}"),
            "d",
            "user",
            "b",
        ));
    }
    f.json(&["sync", "--apply", "--json"]);
    for p in &files {
        fs::remove_file(p).unwrap();
    }
    let (code, doc) = f.json(&["sync", "--apply", "--json"]);
    assert_eq!(code, Some(1));
    assert_eq!(doc["code"], "mass_delete");
    let (code, doc) = f.json(&["sync", "--apply", "--allow-mass-delete", "--json"]);
    assert_eq!(code, Some(0));
    assert_eq!(doc["summary"]["remove"], 5);
}

fn edit(f: &Fx, agent: &str, id: &str, stdin: &str) -> (Option<i32>, Value) {
    run_stdin(f, stdin, &["edit", "--agent", agent, "--id", id, "--json"])
}

fn run_stdin(f: &Fx, stdin: &str, args: &[&str]) -> (Option<i32>, Value) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_nativemem"))
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("CODEX_HOME")
        .env_remove("OPENCODE_CONFIG_DIR")
        .env_remove("PI_CODING_AGENT_DIR")
        .env("HOME", f.state.with_file_name("home"))
        .env_remove("NATIVEMEM_OPENCODE_HOME")
        .env_remove("NATIVEMEM_OPENCODE_DATA_HOME")
        .env_remove("XDG_DATA_HOME")
        .env_remove("NATIVEMEM_PI_HOME")
        .arg("--claude-home")
        .arg(&f.claude)
        .arg("--codex-home")
        .arg(&f.codex)
        .arg("--opencode-home")
        .arg(f.state.with_file_name("opencode"))
        .arg("--opencode-data-home")
        .arg(f.state.with_file_name("opencode-data"))
        .arg("--pi-home")
        .arg(f.state.with_file_name("pi"))
        .arg("--state-dir")
        .arg(&f.state)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    let o = child.wait_with_output().unwrap();
    let doc = serde_json::from_slice(&o.stdout).unwrap_or_else(|e| panic!("{e}: {}", text(&o)));
    (o.status.code(), doc)
}

#[test]
fn edit_original_then_sync_updates_copy() {
    let f = Fx::new();
    f.json(&["sync", "--apply", "--json"]);
    let (code, doc) = edit(
        &f,
        "claude",
        "-w-app/a.md",
        r#"{"name":"alpha","description":"new desc","category":"feedback","body":"new body"}"#,
    );
    assert_eq!(code, Some(0), "{doc}");
    assert_eq!(
        doc["edited"],
        json!({ "agent": "claude", "native_id": "-w-app/a.md" })
    );
    let backup = Path::new(doc["backup_dir"].as_str().unwrap());
    assert!(backup.starts_with(f.state.join("backups")));
    assert!(backup.join("claude").exists());

    let on_disk = fs::read_to_string(&f.claude_file).unwrap();
    assert!(on_disk.contains("description: new desc"), "{on_disk}");
    assert!(on_disk.contains("type: feedback"), "{on_disk}");
    assert!(on_disk.contains("new body"), "{on_disk}");
    assert!(!on_disk.contains("alpha body"), "{on_disk}");
    assert!(on_disk.contains("originSessionId: s-1"), "{on_disk}");
    assert!(on_disk.contains("modified: 2026-01-01"), "{on_disk}");

    let (_, items) = f.json(&["items", "--json"]);
    assert_eq!(doc["hash"], find(&items, "claude")["hash"]);

    let (code, sync) = f.json(&["sync", "--apply", "--json"]);
    assert_eq!(code, Some(0));
    assert_eq!(sync["summary"]["update"], 1, "{sync}");
    assert_eq!(sync["actions"][0]["op"], "update");
    assert_eq!(sync["actions"][0]["agent"], "codex");
    let (_, items) = f.json(&["items", "--json"]);
    let copy = find(&items, "codex");
    assert_eq!(copy["body"].as_str().unwrap().trim_end(), "new body");
    assert_eq!(copy["description"], "new desc");
}

#[test]
fn edit_with_null_or_absent_category_clears_it() {
    for input in [
        r#"{"name":"alpha","description":"d","category":null,"body":"b"}"#,
        r#"{"name":"alpha","description":"d","body":"b"}"#,
    ] {
        let f = Fx::new();
        let (code, doc) = edit(&f, "claude", "-w-app/a.md", input);
        assert_eq!(code, Some(0), "{doc}");
        let on_disk = fs::read_to_string(&f.claude_file).unwrap();
        assert!(!on_disk.contains("\n  type:"), "{on_disk}");
        assert!(on_disk.contains("node_type: memory"), "{on_disk}");
        let (_, items) = f.json(&["items", "--json"]);
        assert_eq!(find(&items, "claude")["category"], Value::Null);
        assert_eq!(doc["hash"], find(&items, "claude")["hash"]);
    }
}

#[test]
fn edit_replica_is_unsupported() {
    let f = Fx::new();
    f.json(&["sync", "--apply", "--json"]);
    let (_, items) = f.json(&["items", "--json"]);
    let id = find(&items, "codex")["native_id"]
        .as_str()
        .unwrap()
        .to_string();
    let (code, doc) = edit(
        &f,
        "codex",
        &id,
        r#"{"name":"alpha","description":"d","category":null,"body":"b"}"#,
    );
    assert_eq!(code, Some(1));
    assert_eq!(doc["code"], "unsupported");
    assert!(doc["error"].as_str().unwrap().contains("original"));
}

#[test]
fn edit_unknown_id_is_not_found() {
    let f = Fx::new();
    let (code, doc) = edit(
        &f,
        "claude",
        "-w-app/missing.md",
        r#"{"name":"n","description":"d","category":null,"body":"b"}"#,
    );
    assert_eq!(code, Some(1));
    assert_eq!(doc["code"], "not_found");
}

#[test]
fn edit_bad_stdin_is_parse_error() {
    let f = Fx::new();
    let before = fs::read_to_string(&f.claude_file).unwrap();
    for input in ["", "not json", r#"{"name":"n"}"#] {
        let (code, doc) = edit(&f, "claude", "-w-app/a.md", input);
        assert_eq!(code, Some(1), "{input}");
        assert_eq!(doc["code"], "parse", "{input}");
    }
    assert_eq!(fs::read_to_string(&f.claude_file).unwrap(), before);
}

#[test]
fn concurrent_run_is_refused_while_lock_is_held() {
    let t = tempfile::tempdir().unwrap();
    let (claude, codex, state) = (
        t.path().join("claude"),
        t.path().join("codex"),
        t.path().join("state"),
    );
    fs::create_dir_all(&state).unwrap();
    let held = fs::File::create(state.join("sync.lock")).unwrap();
    held.try_lock().unwrap();

    let o = run(&claude, &codex, &state, &["sync", "--apply", "--json"]);
    assert_eq!(o.status.code(), Some(1));
    let doc: serde_json::Value = serde_json::from_str(&text(&o)).unwrap();
    assert_eq!(doc["code"], "busy");
    assert_eq!(doc["error"], "another nativemem run is in progress");
    assert!(!state.join("ledger.db").exists());

    let o = run(&claude, &codex, &state, &["sync", "--json"]);
    assert_eq!(o.status.code(), Some(0));
    drop(held);
    let o = run(&claude, &codex, &state, &["sync", "--apply"]);
    assert_eq!(o.status.code(), Some(0));
}

#[cfg(target_os = "macos")]
mod schedule {
    use super::*;

    struct Env {
        _t: tempfile::TempDir,
        agents: std::path::PathBuf,
        state: std::path::PathBuf,
    }

    fn env() -> Env {
        let t = tempfile::tempdir().unwrap();
        let agents = t.path().join("LaunchAgents");
        let state = t.path().join("state & <co>");
        Env {
            _t: t,
            agents,
            state,
        }
    }

    fn sched(e: &Env, extra: &[&str], args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_nativemem"))
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("CODEX_HOME")
            .env_remove("NATIVEMEM_CLAUDE_HOME")
            .env_remove("NATIVEMEM_CODEX_HOME")
            .env_remove("OPENCODE_CONFIG_DIR")
            .env_remove("PI_CODING_AGENT_DIR")
            .env_remove("NATIVEMEM_OPENCODE_HOME")
            .env_remove("NATIVEMEM_OPENCODE_DATA_HOME")
            .env_remove("XDG_DATA_HOME")
            .env_remove("NATIVEMEM_PI_HOME")
            .env_remove("NATIVEMEM_LAUNCH_AGENTS_DIR")
            .arg("--state-dir")
            .arg(&e.state)
            .args(extra)
            .arg("schedule")
            .arg("--no-launchctl")
            .arg("--launch-agents-dir")
            .arg(&e.agents)
            .args(args)
            .output()
            .unwrap()
    }

    fn plist_path(e: &Env) -> std::path::PathBuf {
        e.agents.join("dev.nativemem.sync.plist")
    }

    fn lint(path: &Path) {
        let o = Command::new("plutil")
            .arg("-lint")
            .arg(path)
            .output()
            .unwrap();
        assert!(o.status.success(), "{}", text(&o));
    }

    #[test]
    fn install_writes_plist_and_copies_binary() {
        let e = env();
        let o = sched(&e, &[], &["install", "--every", "15"]);
        assert_eq!(
            o.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&o.stderr)
        );
        let plist = fs::read_to_string(plist_path(&e)).unwrap();
        let bin = e.state.join("bin").join("nativemem");
        assert!(plist.contains("<integer>900</integer>"), "{plist}");
        assert!(
            plist.contains("<key>RunAtLoad</key>\n    <false/>"),
            "{plist}"
        );
        assert!(
            plist.contains("<key>LowPriorityIO</key>\n    <true/>"),
            "{plist}"
        );
        assert!(plist.contains("<string>Background</string>"), "{plist}");
        assert!(
            plist.contains("state &amp; &lt;co&gt;/bin/nativemem"),
            "{plist}"
        );
        assert!(plist.contains(
            "<string>sync</string>\n        <string>--apply</string>\n        <string>--notify</string>"
        ));
        assert!(plist.contains("state &amp; &lt;co&gt;/sync.log"), "{plist}");
        assert!(bin.is_file());
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&bin).unwrap().permissions().mode() & 0o777,
            0o755
        );
        lint(&plist_path(&e));

        let o = sched(&e, &[], &["install"]);
        assert_eq!(o.status.code(), Some(0));
        let again = fs::read_to_string(plist_path(&e)).unwrap();
        assert!(again.contains("<integer>3600</integer>"), "{again}");
        assert_eq!(again.matches("<key>Label</key>").count(), 1);
        assert_eq!(fs::read_dir(e.state.join("bin")).unwrap().count(), 1);
    }

    #[test]
    fn install_passes_explicit_roots_through() {
        let e = env();
        let o = sched(
            &e,
            &[
                "--claude-home",
                "/x/claude",
                "--codex-home",
                "/x/codex",
                "--opencode-home",
                "/x/opencode",
                "--pi-home",
                "/x/pi",
            ],
            &["install"],
        );
        assert_eq!(o.status.code(), Some(0));
        let plist = fs::read_to_string(plist_path(&e)).unwrap();
        for needle in [
            "<string>--claude-home</string>\n        <string>/x/claude</string>",
            "<string>--codex-home</string>\n        <string>/x/codex</string>",
            "<string>--opencode-home</string>\n        <string>/x/opencode</string>",
            "<string>--pi-home</string>\n        <string>/x/pi</string>",
            "<string>--state-dir</string>",
        ] {
            assert!(plist.contains(needle), "{needle}\n{plist}");
        }
        lint(&plist_path(&e));
    }

    #[test]
    fn defaults_are_not_baked_in() {
        let e = env();
        let o = Command::new(env!("CARGO_BIN_EXE_nativemem"))
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("CODEX_HOME")
            .env_remove("NATIVEMEM_CLAUDE_HOME")
            .env_remove("NATIVEMEM_CODEX_HOME")
            .env_remove("OPENCODE_CONFIG_DIR")
            .env_remove("PI_CODING_AGENT_DIR")
            .env_remove("NATIVEMEM_OPENCODE_HOME")
            .env_remove("NATIVEMEM_OPENCODE_DATA_HOME")
            .env_remove("XDG_DATA_HOME")
            .env_remove("NATIVEMEM_PI_HOME")
            .env_remove("NATIVEMEM_STATE_DIR")
            .env("HOME", e.state.parent().unwrap())
            .args(["schedule", "--no-launchctl", "--launch-agents-dir"])
            .arg(&e.agents)
            .arg("install")
            .output()
            .unwrap();
        assert_eq!(o.status.code(), Some(0));
        let plist = fs::read_to_string(plist_path(&e)).unwrap();
        assert!(!plist.contains("--claude-home"), "{plist}");
        assert!(!plist.contains("--state-dir"), "{plist}");
        assert!(e
            .state
            .parent()
            .unwrap()
            .join(".nativemem/bin/nativemem")
            .is_file());
    }

    #[test]
    fn interval_below_five_is_rejected() {
        let e = env();
        let o = sched(&e, &[], &["install", "--every", "4"]);
        assert!(!o.status.success());
        assert!(!plist_path(&e).exists());
    }

    #[test]
    fn uninstall_removes_plist_and_is_idempotent() {
        let e = env();
        sched(&e, &[], &["install"]);
        let o = sched(&e, &[], &["uninstall"]);
        assert_eq!(o.status.code(), Some(0));
        assert!(!plist_path(&e).exists());
        assert!(e.state.join("bin").join("nativemem").is_file());
        let o = sched(&e, &[], &["uninstall"]);
        assert_eq!(o.status.code(), Some(0));
    }

    #[test]
    fn status_reflects_installed_state() {
        let e = env();
        let doc = |e: &Env| -> serde_json::Value {
            serde_json::from_str(&text(&sched(e, &[], &["status", "--json"]))).unwrap()
        };
        let d = doc(&e);
        assert_eq!(d["installed"], false);
        assert_eq!(d["loaded"], false);
        assert!(d["interval_minutes"].is_null());
        assert_eq!(d["log_tail"], serde_json::json!([]));

        sched(&e, &[], &["install", "--every", "30"]);
        fs::write(e.state.join("sync.log"), "1\n2\n3\n4\n5\n6\n7\n").unwrap();
        let d = doc(&e);
        assert_eq!(d["installed"], true);
        assert_eq!(d["loaded"], false);
        assert_eq!(d["interval_minutes"], 30);
        assert_eq!(d["log_tail"], serde_json::json!(["3", "4", "5", "6", "7"]));
    }
}

const CLAUDE_MD: &str = "# Mine\n@RTK.md\n\n## Style\nbe terse\n\n## Tests\nrun them\n";

#[test]
fn instruction_items_carry_kind_and_count_in_status() {
    let f = Fx::new();
    fs::write(f.claude.join("CLAUDE.md"), CLAUDE_MD).unwrap();
    let (_, doc) = f.json(&["items", "--json"]);
    let list = doc["items"].as_array().unwrap();
    assert!(list.iter().all(|i| i["kind"].is_string()));
    let style = list
        .iter()
        .find(|i| i["native_id"] == "CLAUDE.md#style")
        .unwrap();
    assert_eq!(style["kind"], "instruction");
    assert_eq!(style["scope"], json!({ "type": "global" }));
    assert_eq!(style["body"], "be terse");
    assert_eq!(style["description"], "Style");
    assert!(style["category"].is_null());

    let (_, status) = f.json(&["status", "--json"]);
    assert_eq!(status["agents"][0]["originals"], 3);

    let out = text(&run(&f.claude, &f.codex, &f.state, &["items"]));
    assert!(out.contains("claude  instruction  global  style"), "{out}");

    f.json(&["sync", "--apply", "--json"]);
    let (_, doc) = f.json(&["items", "--json"]);
    let replica = doc["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["agent"] == "codex" && i["kind"] == "instruction")
        .unwrap();
    assert_eq!(replica["replica_of"]["origin"], "claude");
}

#[test]
fn edit_instruction_changes_body_only() {
    let f = Fx::new();
    fs::write(f.claude.join("CLAUDE.md"), CLAUDE_MD).unwrap();
    let (code, doc) = edit(
        &f,
        "claude",
        "CLAUDE.md#style",
        r#"{ "name": "style", "description": "Style", "category": "user", "body": "be brief" }"#,
    );
    assert_eq!(code, Some(0), "{doc}");
    assert_eq!(
        fs::read_to_string(f.claude.join("CLAUDE.md")).unwrap(),
        CLAUDE_MD.replace("be terse", "be brief")
    );
    let (_, items) = f.json(&["items", "--json"]);
    let style = items["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["native_id"] == "CLAUDE.md#style")
        .unwrap();
    assert_eq!(doc["hash"], style["hash"]);
}

#[test]
fn edit_instruction_heading_is_invalid_and_writes_nothing() {
    let f = Fx::new();
    fs::write(f.claude.join("CLAUDE.md"), CLAUDE_MD).unwrap();
    for changed in [
        r#"{ "name": "other", "description": "Style", "category": null, "body": "x" }"#,
        r#"{ "name": "style", "description": "Other", "category": null, "body": "x" }"#,
    ] {
        let (code, doc) = edit(&f, "claude", "CLAUDE.md#style", changed);
        assert_eq!(code, Some(1));
        assert_eq!(doc["code"], "invalid");
        assert!(doc["error"].as_str().unwrap().contains("heading"));
    }
    assert_eq!(
        fs::read_to_string(f.claude.join("CLAUDE.md")).unwrap(),
        CLAUDE_MD
    );
    assert!(!f.state.join("backups").exists());
}

fn add(
    f: &Fx,
    agent: &str,
    kind: &str,
    project: Option<&str>,
    stdin: &str,
) -> (Option<i32>, Value) {
    let mut args = vec!["add", "--agent", agent, "--kind", kind, "--json"];
    if let Some(p) = project {
        args.extend(["--project", p]);
    }
    run_stdin(f, stdin, &args)
}

const NEW_MEMORY: &str = r#"{"name":"created-note","description":"made by hand","category":"feedback","body":"hand body"}"#;
const NEW_SECTION: &str =
    r#"{"name":"x","description":"Fresh Rule","category":"user","body":"do it"}"#;

fn items_of(f: &Fx) -> Vec<Value> {
    f.json(&["items", "--json"]).1["items"]
        .as_array()
        .unwrap()
        .clone()
}

#[test]
fn add_memory_in_every_agent_then_sync_spreads_it() {
    for agent in ["claude", "codex", "opencode", "pi"] {
        let f = four_agents();
        let (code, doc) = add(&f, agent, "memory", Some("/w/app/"), NEW_MEMORY);
        assert_eq!(code, Some(0), "{agent}: {doc}");
        assert_eq!(doc["agent"], agent);
        assert!(
            Path::new(doc["path"].as_str().unwrap()).is_file(),
            "{agent}"
        );
        assert!(Path::new(doc["backup_dir"].as_str().unwrap()).starts_with(f.state.join("backups")));
        let id = doc["native_id"].as_str().unwrap();
        let created = items_of(&f)
            .into_iter()
            .find(|i| i["agent"] == agent && i["native_id"] == id)
            .unwrap();
        assert_eq!(
            created["scope"],
            json!({ "type": "project", "path": "/w/app" })
        );
        assert_eq!(created["category"], "feedback");
        assert!(created["replica_of"].is_null());
        assert_eq!(created["hash"], doc["hash"]);

        let (code, sync) = f.json(&["sync", "--apply", "--json"]);
        assert_eq!(code, Some(0), "{agent}: {sync}");
        assert_eq!(sync["notes"], json!([]));
        let copies: Vec<Value> = items_of(&f)
            .into_iter()
            .filter(|i| i["name"] == "created-note" && !i["replica_of"].is_null())
            .collect();
        assert_eq!(copies.len(), 3, "{agent}");
        for c in &copies {
            assert_eq!(c["replica_of"], json!({ "origin": agent, "native_id": id }));
            assert_eq!(c["hash"], doc["hash"]);
        }
        let (_, again) = f.json(&["sync", "--json"]);
        assert_eq!(again["actions"], json!([]), "{agent}");
    }
}

#[test]
fn add_instruction_in_every_agent_then_sync_spreads_it() {
    for agent in ["claude", "codex", "opencode", "pi"] {
        let f = four_agents();
        let (code, doc) = add(&f, agent, "instruction", None, NEW_SECTION);
        assert_eq!(code, Some(0), "{agent}: {doc}");
        let file = if agent == "claude" {
            "CLAUDE.md"
        } else {
            "AGENTS.md"
        };
        assert_eq!(doc["native_id"], format!("{file}#fresh-rule"));
        let created = items_of(&f)
            .into_iter()
            .find(|i| i["agent"] == agent && i["native_id"] == doc["native_id"])
            .unwrap();
        assert_eq!(created["scope"], json!({ "type": "global" }));
        assert_eq!(created["name"], "fresh-rule");
        assert_eq!(created["description"], "Fresh Rule");
        assert_eq!(created["category"], Value::Null);
        assert_eq!(created["hash"], doc["hash"]);

        let (code, sync) = f.json(&["sync", "--apply", "--json"]);
        assert_eq!(code, Some(0), "{agent}: {sync}");
        let copies: Vec<Value> = items_of(&f)
            .into_iter()
            .filter(|i| i["name"] == "fresh-rule" && !i["replica_of"].is_null())
            .collect();
        assert_eq!(copies.len(), 3, "{agent}");
        for c in &copies {
            assert_eq!(c["body"], "do it");
            assert_eq!(c["hash"], doc["hash"]);
        }
    }
}

#[test]
fn add_refuses_a_taken_id_and_leaves_the_file_alone() {
    let f = four_agents();
    for (kind, project, input) in [
        ("memory", Some("/w/app"), NEW_MEMORY),
        ("instruction", None, NEW_SECTION),
    ] {
        let (code, _) = add(&f, "pi", kind, project, input);
        assert_eq!(code, Some(0));
        let before = fs::read_to_string(f.state.with_file_name("pi").join("AGENTS.md")).unwrap();
        let (code, doc) = add(&f, "pi", kind, project, input);
        assert_eq!(code, Some(1), "{kind}");
        assert_eq!(doc["code"], "exists", "{kind}");
        assert_eq!(
            fs::read_to_string(f.state.with_file_name("pi").join("AGENTS.md")).unwrap(),
            before
        );
    }
    let (code, doc) = add(
        &f,
        "claude",
        "memory",
        Some("/w/app"),
        r#"{"name":"a","description":"d","category":null,"body":"b"}"#,
    );
    assert_eq!((code, &doc["code"]), (Some(1), &json!("exists")));
    assert!(fs::read_to_string(&f.claude_file)
        .unwrap()
        .contains("alpha body"));
}

#[test]
fn add_reports_invalid_input_and_writes_nothing() {
    let f = four_agents();
    let section = |description: &str, body: &str| {
        json!({ "name": "n", "description": description, "category": null, "body": body })
            .to_string()
    };
    let memory = |name: &str, description: &str, category: Value| {
        json!({ "name": name, "description": description, "category": category, "body": "b" })
            .to_string()
    };
    let cases: Vec<(&str, Option<&str>, String)> = vec![
        ("memory", None, NEW_MEMORY.into()),
        ("memory", Some("w/app"), NEW_MEMORY.into()),
        ("memory", Some("/w/../app"), NEW_MEMORY.into()),
        ("memory", Some("/w/app"), memory(" ", "d", Value::Null)),
        ("memory", Some("/w/app"), memory("n", "", Value::Null)),
        (
            "memory",
            Some("/w/app"),
            memory("n", "two\nlines", Value::Null),
        ),
        ("memory", Some("/w/app"), memory("n", "d", json!("a\nb"))),
        ("instruction", Some("/w/app"), NEW_SECTION.into()),
        ("instruction", None, section("", "b")),
        ("instruction", None, section("two\nlines", "b")),
        (
            "instruction",
            None,
            section("Fine", "text\n## Another\nmore"),
        ),
    ];
    for (kind, project, input) in &cases {
        for agent in ["claude", "pi"] {
            let (code, doc) = add(&f, agent, kind, *project, input);
            assert_eq!(code, Some(1), "{agent} {kind} {project:?} {input}");
            assert_eq!(doc["code"], "invalid", "{agent} {kind} {project:?} {input}");
        }
    }
    assert_eq!(
        fs::read_to_string(f.claude.join("CLAUDE.md")).unwrap(),
        "# Mine\n\n## Style\nbe terse\n"
    );
    assert!(!f.state.with_file_name("pi").join("AGENTS.md").exists());
    assert!(!f
        .claude
        .join("projects/-w-app/memory/created-note.md")
        .exists());
}

#[test]
fn add_trims_the_project_and_treats_an_empty_category_as_none() {
    let f = four_agents();
    let input = r#"{"name":"n","description":"d","category":"","body":"b"}"#;
    let (code, doc) = add(&f, "pi", "memory", Some("/w/app//"), input);
    assert_eq!(code, Some(0), "{doc}");
    let created = items_of(&f)
        .into_iter()
        .find(|i| i["agent"] == "pi")
        .unwrap();
    assert_eq!(created["scope"]["path"], "/w/app");
    assert_eq!(created["category"], Value::Null);
}

#[test]
fn add_error_codes_for_agents_and_stdin() {
    let f = Fx::new();
    let (code, doc) = add(&f, "pi", "memory", Some("/w/app"), NEW_MEMORY);
    assert_eq!((code, &doc["code"]), (Some(1), &json!("not_installed")));
    assert!(!f.state.with_file_name("pi").exists());
    let (code, doc) = add(&f, "nobody", "memory", Some("/w/app"), NEW_MEMORY);
    assert_eq!((code, &doc["code"]), (Some(1), &json!("unknown_agent")));
    for input in ["", "not json", r#"{"name":"n"}"#] {
        let (code, doc) = add(&f, "claude", "memory", Some("/w/app"), input);
        assert_eq!((code, &doc["code"]), (Some(1), &json!("parse")), "{input}");
    }
    let o = run(
        &f.claude,
        &f.codex,
        &f.state,
        &["add", "--agent", "claude", "--kind", "session", "--json"],
    );
    assert_eq!(o.status.code(), Some(2));
}

#[test]
fn add_is_refused_while_the_lock_is_held() {
    let f = four_agents();
    fs::create_dir_all(&f.state).unwrap();
    let held = fs::File::create(f.state.join("sync.lock")).unwrap();
    held.try_lock().unwrap();
    let (code, doc) = add(&f, "pi", "memory", Some("/w/app"), NEW_MEMORY);
    assert_eq!((code, &doc["code"]), (Some(1), &json!("busy")));
    assert!(!f.state.with_file_name("pi").join("AGENTS.md").exists());
}

const CHAT: &[(&str, &str)] = &[
    ("user", "how do I fix the login?"),
    ("assistant", "check the token expiry"),
];

fn with_session() -> (Fx, std::path::PathBuf) {
    let f = Fx::new();
    fs::create_dir_all(f.state.with_file_name("pi")).unwrap();
    let file =
        ClaudeHome::new(&f.claude).add_session(Path::new("/w/app"), "c1", Some("Fix login"), CHAT);
    (f, file)
}

#[test]
fn sessions_sync_to_pi_and_show_in_status_separately() {
    let (f, _) = with_session();
    let (code, doc) = f.json(&["sync", "--apply", "--json"]);
    assert_eq!(code, Some(0), "{doc}");
    let (_, status) = f.json(&["status", "--json"]);
    let by_id = |id: &str| -> Value {
        status["agents"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["id"] == id)
            .unwrap()
            .clone()
    };
    assert_eq!(
        (
            by_id("claude")["sessions"].clone(),
            by_id("claude")["originals"].clone()
        ),
        (json!(1), json!(1))
    );
    assert_eq!(by_id("pi")["sessions"], 1);
    assert_eq!(by_id("pi")["replicas"], 1, "the memory item");
    assert_eq!(
        (
            by_id("codex")["sessions"].clone(),
            by_id("codex")["replicas"].clone()
        ),
        (json!(1), json!(1))
    );
    assert_eq!(status["ledger_items"], 1, "memory items only");
    let (code, plan) = f.json(&["sync", "--json"]);
    assert_eq!(code, Some(0));
    assert_eq!(plan["actions"], json!([]));
}

#[test]
fn no_sessions_turns_sessions_off() {
    let (f, _) = with_session();
    let o = run(&f.claude, &f.codex, &f.state, &["--no-sessions", "sync"]);
    let out = text(&o);
    assert!(!out.contains("session"), "{out}");
    let o = run(
        &f.claude,
        &f.codex,
        &f.state,
        &["--no-sessions", "status", "--json"],
    );
    let doc: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(doc["agents"][0]["sessions"], 0);
}

#[test]
fn items_leaves_sessions_out_unless_asked_and_never_prints_bodies() {
    let (f, _) = with_session();
    f.json(&["sync", "--apply", "--json"]);
    let (_, items) = f.json(&["items", "--json"]);
    assert!(items["items"]
        .as_array()
        .unwrap()
        .iter()
        .all(|i| i["kind"] != "session"));
    let (_, items) = f.json(&["items", "--kind", "session", "--json"]);
    let list = items["items"].as_array().unwrap();
    assert_eq!(list.len(), 3);
    for i in list {
        assert_eq!(i["kind"], "session");
        assert_eq!(i["body"], "");
        assert_eq!(i["scope"], json!({"type": "project", "path": "/w/app"}));
        assert_eq!(i["description"], "Fix login");
        let session = i["session"].as_object().unwrap();
        assert_eq!(session["turns"], CHAT.len());
        assert!(session["updated_ms"].as_i64().unwrap() > 0);
    }
    assert_eq!(list.iter().filter(|i| i["replica_of"].is_null()).count(), 1);
    let pi = list.iter().find(|i| i["agent"] == "pi").unwrap();
    assert_eq!(pi["replica_of"]["origin"], "claude");
    let human = text(&run(&f.claude, &f.codex, &f.state, &["items"]));
    assert!(!human.contains("session"), "{human}");
}

#[test]
fn memory_items_carry_no_session_object() {
    let f = Fx::new();
    let (_, items) = f.json(&["items", "--json"]);
    assert!(items["items"]
        .as_array()
        .unwrap()
        .iter()
        .all(|i| i.get("session").is_none()));
}

#[test]
fn session_show_prints_the_transcript_read_only() {
    let (f, _) = with_session();
    let (code, doc) = f.json(&[
        "session",
        "show",
        "--agent",
        "claude",
        "--id",
        "-w-app/c1.jsonl",
        "--json",
    ]);
    assert_eq!(code, Some(0), "{doc}");
    assert_eq!(doc["agent"], "claude");
    assert_eq!(doc["native_id"], "-w-app/c1.jsonl");
    assert_eq!(doc["title"], "Fix login");
    assert_eq!(doc["cwd"], "/w/app");
    assert!(doc["created_ms"].as_i64().unwrap() > 0);
    let turns = doc["turns"].as_array().unwrap();
    assert_eq!(turns.len(), CHAT.len());
    for (turn, (role, text)) in turns.iter().zip(CHAT) {
        assert_eq!(
            (turn["role"].as_str(), turn["text"].as_str()),
            (Some(*role), Some(*text))
        );
        assert!(turn["ts_ms"].as_i64().unwrap() > 0);
    }
    let human = text(&run(
        &f.claude,
        &f.codex,
        &f.state,
        &[
            "session",
            "show",
            "--agent",
            "claude",
            "--id",
            "-w-app/c1.jsonl",
        ],
    ));
    assert!(
        human.contains("Fix login") && human.contains("You:") && human.contains("Assistant:"),
        "{human}"
    );
    assert!(!f.state.join("sync.lock").exists(), "no lock is taken");
}

#[test]
fn session_show_reports_unknown_agent_invalid_id_and_missing_session() {
    let (f, _) = with_session();
    let code_of = |agent: &str, id: &str| {
        let (code, doc) = f.json(&["session", "show", "--agent", agent, "--id", id, "--json"]);
        assert_eq!(code, Some(1), "{doc}");
        doc["code"].as_str().unwrap().to_string()
    };
    assert_eq!(code_of("nobody", "-w-app/c1.jsonl"), "unknown_agent");
    assert_eq!(code_of("claude", "memory/x.md"), "invalid");
    assert_eq!(code_of("claude", "-w-app/missing.jsonl"), "not_found");
}

#[test]
fn delete_and_edit_refuse_session_ids() {
    let (f, _) = with_session();
    let id = "-w-app/c1.jsonl";
    let (code, doc) = f.json(&["delete", "--agent", "claude", "--id", id, "--json"]);
    assert_eq!(
        (code, &doc["code"]),
        (Some(1), &json!("unsupported")),
        "{doc}"
    );
    let mut child = Command::new(env!("CARGO_BIN_EXE_nativemem"))
        .env("HOME", f.state.with_file_name("home"))
        .env_remove("NATIVEMEM_OPENCODE_DATA_HOME")
        .env_remove("XDG_DATA_HOME")
        .arg("--claude-home")
        .arg(&f.claude)
        .arg("--codex-home")
        .arg(&f.codex)
        .arg("--state-dir")
        .arg(&f.state)
        .args(["edit", "--agent", "claude", "--id", id, "--json"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(br#"{"name":"n","description":"d","category":null,"body":"b"}"#)
        .unwrap();
    let o = child.wait_with_output().unwrap();
    let doc: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(doc["code"], "unsupported");
    assert!(f.claude.join("projects/-w-app/c1.jsonl").is_file());
}

#[test]
fn pi_session_reaches_claude_through_the_cli() {
    let f = Fx::new();
    let pi = f.state.with_file_name("pi");
    fs::create_dir_all(&pi).unwrap();
    PiHome::new(&pi).add_session(Path::new("/w/app"), "p1", Some("Pi chat"), CHAT);
    let out = text(&run(&f.claude, &f.codex, &f.state, &["sync"]));
    assert!(
        out.contains("+ claude  /w/app  session \"Pi chat\"  (from pi)"),
        "{out}"
    );
}

#[test]
fn automated_sessions_are_counted_and_excluded_on_request() {
    let (f, file) = with_session();
    let text_in = fs::read_to_string(&file).unwrap().replace(
        "\"type\":\"user\"",
        "\"entrypoint\":\"sdk-cli\",\"type\":\"user\"",
    );
    fs::write(&file, text_in).unwrap();
    let count = |extra: &[&str]| -> (Value, Value) {
        let mut args = extra.to_vec();
        args.extend(["status", "--json"]);
        let o = run(&f.claude, &f.codex, &f.state, &args);
        let doc: Value = serde_json::from_slice(&o.stdout).unwrap();
        (
            doc["agents"][0]["sessions"].clone(),
            doc["agents"][0]["automated_sessions"].clone(),
        )
    };
    assert_eq!(count(&[]), (json!(1), json!(1)));
    assert_eq!(
        count(&["--exclude-automated-sessions"]),
        (json!(0), json!(1))
    );
}

#[test]
fn session_days_is_an_opt_in_limit() {
    let (f, file) = with_session();
    nativemem_core::testkit::age_days(&file, 30);
    let sessions = |extra: &[&str]| -> Value {
        let mut args = extra.to_vec();
        args.extend(["status", "--json"]);
        let o = run(&f.claude, &f.codex, &f.state, &args);
        serde_json::from_slice::<Value>(&o.stdout).unwrap()["agents"][0]["sessions"].clone()
    };
    assert_eq!(sessions(&[]), 1);
    assert_eq!(sessions(&["--session-days", "14"]), 0);
}

fn skill_dir(root: &Path, name: &str, body: &str) -> std::path::PathBuf {
    let d = root.join(name);
    fs::create_dir_all(&d).unwrap();
    fs::write(
        d.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: Does {name}\n---\n{body}\n"),
    )
    .unwrap();
    d
}

fn skills_of(f: &Fx, agent: &str) -> std::path::PathBuf {
    let home = match agent {
        "claude" => f.claude.clone(),
        "codex" => f.codex.clone(),
        other => f.state.with_file_name(other),
    };
    home.join("skills")
}

#[test]
fn skills_sync_through_the_cli_and_items_status_and_show_report_them() {
    let f = four_agents();
    let d = skill_dir(&skills_of(&f, "claude"), "demo-skill", "the text");
    fs::write(d.join("run.sh"), "echo secret-looking-content").unwrap();

    let (_, before) = f.json(&["items", "--kind", "skill", "--json"]);
    assert_eq!(before["items"].as_array().unwrap().len(), 1);

    let (code, doc) = f.json(&["sync", "--apply", "--json"]);
    assert_eq!(code, Some(0), "{doc}");
    assert_eq!(doc["notes"], json!([]));
    for agent in ["codex", "opencode", "pi"] {
        assert!(
            skills_of(&f, agent).join("demo-skill/run.sh").is_file(),
            "{agent}"
        );
    }

    let (_, items) = f.json(&["items", "--kind", "skill", "--json"]);
    let list = items["items"].as_array().unwrap();
    assert_eq!(list.len(), 4);
    for i in list {
        assert_eq!(i["kind"], "skill");
        assert_eq!(i["name"], "demo-skill");
        assert_eq!(i["native_id"], "skill:demo-skill");
        assert_eq!(i["description"], "Does demo-skill");
        assert_eq!(i["body"], "");
        assert!(!i.to_string().contains("secret-looking-content"));
    }
    assert!(find(&items, "claude")["replica_of"].is_null());
    assert_eq!(find(&items, "pi")["replica_of"]["origin"], "claude");
    let (_, all) = f.json(&["items", "--json"]);
    assert!(all["items"]
        .as_array()
        .unwrap()
        .iter()
        .any(|i| i["kind"] == "skill"));

    let (_, status) = f.json(&["status", "--json"]);
    for a in status["agents"].as_array().unwrap() {
        assert_eq!(a["skills"], 1, "{a}");
    }
    assert!(text(&run(&f.claude, &f.codex, &f.state, &["status"])).contains("1 skills"));

    let (code, shown) = f.json(&[
        "skill",
        "show",
        "--agent",
        "pi",
        "--name",
        "demo-skill",
        "--json",
    ]);
    assert_eq!(code, Some(0));
    assert_eq!(shown["name"], "demo-skill");
    assert!(shown["text"].as_str().unwrap().contains("the text"));
    let o = run(
        &f.claude,
        &f.codex,
        &f.state,
        &["skill", "show", "--agent", "claude", "--name", "demo-skill"],
    );
    assert!(text(&o).starts_with("---\nname: demo-skill"));
    let (code, missing) = f.json(&["skill", "show", "--agent", "pi", "--name", "nope", "--json"]);
    assert_eq!(code, Some(1));
    assert_eq!(missing["code"], "not_found");

    let (code, doc) = f.json(&["sync", "--json"]);
    assert_eq!(code, Some(0));
    assert_eq!(doc["actions"], json!([]));
}

#[test]
fn pi_project_skills_are_discovered_from_session_projects() {
    let f = four_agents();
    let cwd = f.state.with_file_name("pi-project");
    fs::create_dir_all(&cwd).unwrap();
    PiHome::new(&f.state.with_file_name("pi")).add_session(
        &cwd,
        "p1",
        Some("Demo chat"),
        &[("user", "fixture question")],
    );
    skill_dir(&cwd.join(".pi/skills"), "demo-skill", "project fixture");
    let (code, doc) = f.json(&["items", "--kind", "skill", "--json"]);
    assert_eq!(code, Some(0), "{doc}");
    let skills = doc["items"].as_array().unwrap();
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0]["agent"], "pi");
    assert_eq!(skills[0]["scope"]["path"], cwd.display().to_string());
}

#[test]
fn project_skill_sync_survives_reopening_without_project_memories() {
    let f = four_agents();
    let cwd = f.state.with_file_name("skill-only-project");
    fs::create_dir_all(&cwd).unwrap();
    ClaudeHome::new(&f.claude).add_project(&cwd);
    skill_dir(&cwd.join(".claude/skills"), "demo-skill", "project fixture");
    let id = nativemem_core::adapters::skills::project_native_id(
        &nativemem_core::adapters::memory_store::project_key(&cwd),
        "demo-skill",
    );
    let (code, applied) = f.json(&["sync", "--apply", "--json"]);
    assert_eq!(code, Some(0), "{applied}");
    assert!(cwd.join(".pi/skills/demo-skill/SKILL.md").is_file());
    assert!(cwd.join(".opencode/skills/demo-skill/SKILL.md").is_file());
    let (code, unchanged) = f.json(&["sync", "--json"]);
    assert_eq!(code, Some(0), "{unchanged}");
    assert_eq!(unchanged["actions"], json!([]));
    let (code, shown) = f.json(&["skill", "show", "--agent", "pi", "--id", &id, "--json"]);
    assert_eq!(code, Some(0), "{shown}");
    assert!(shown["text"].as_str().unwrap().contains("project fixture"));
    let (code, removed) = f.json(&["delete", "--agent", "pi", "--id", &id, "--json"]);
    assert_eq!(code, Some(0), "{removed}");
    let (code, deleted) = f.json(&["sync", "--apply", "--json"]);
    assert_eq!(code, Some(0), "{deleted}");
    assert!(!cwd.join(".claude/skills/demo-skill").exists());
    assert!(!cwd.join(".opencode/skills/demo-skill").exists());
}

#[test]
fn skill_show_by_id_distinguishes_projects_from_global_skills() {
    let f = four_agents();
    let cwd = f.state.with_file_name("project");
    fs::create_dir_all(&cwd).unwrap();
    ClaudeHome::new(&f.claude).add_project(&cwd);
    skill_dir(&skills_of(&f, "claude"), "demo-skill", "global fixture");
    let project = skill_dir(&cwd.join(".claude/skills"), "demo-skill", "project fixture");
    let id = nativemem_core::adapters::skills::project_native_id(
        &nativemem_core::adapters::memory_store::project_key(&cwd),
        "demo-skill",
    );
    let (code, shown) = f.json(&["skill", "show", "--agent", "claude", "--id", &id, "--json"]);
    assert_eq!(code, Some(0), "{shown}");
    assert_eq!(
        shown["path"],
        project.join("SKILL.md").display().to_string()
    );
    assert!(shown["text"].as_str().unwrap().contains("project fixture"));
    assert!(!shown["text"].as_str().unwrap().contains("global fixture"));
    let (code, global) = f.json(&[
        "skill",
        "show",
        "--agent",
        "claude",
        "--name",
        "demo-skill",
        "--json",
    ]);
    assert_eq!(code, Some(0));
    assert!(global["text"].as_str().unwrap().contains("global fixture"));
    let (code, invalid) = f.json(&[
        "skill",
        "show",
        "--agent",
        "claude",
        "--id",
        "not-a-skill",
        "--json",
    ]);
    assert_eq!(code, Some(1));
    assert_eq!(invalid["code"], "invalid");
}

#[test]
fn skill_delete_backs_up_and_the_next_sync_propagates_it() {
    let f = four_agents();
    skill_dir(&skills_of(&f, "claude"), "demo-skill", "text");
    f.json(&["sync", "--apply", "--json"]);
    let (code, doc) = f.json(&[
        "delete",
        "--agent",
        "codex",
        "--id",
        "skill:demo-skill",
        "--json",
    ]);
    assert_eq!(code, Some(0), "{doc}");
    let backup = Path::new(doc["backup_dir"].as_str().unwrap());
    assert!(backup.join("codex/skills/demo-skill/SKILL.md").is_file());
    assert!(!skills_of(&f, "codex").join("demo-skill").exists());
    assert!(skills_of(&f, "claude").join("demo-skill").exists());

    let (code, doc) = f.json(&["sync", "--apply", "--json"]);
    assert_eq!(code, Some(0), "{doc}");
    for agent in ["claude", "opencode", "pi"] {
        assert!(!skills_of(&f, agent).join("demo-skill").exists(), "{agent}");
    }
}

#[test]
fn skill_edit_is_unsupported() {
    let f = four_agents();
    skill_dir(&skills_of(&f, "claude"), "demo-skill", "text");
    let mut child = Command::new(env!("CARGO_BIN_EXE_nativemem"))
        .env("HOME", f.state.with_file_name("home"))
        .env_remove("NATIVEMEM_OPENCODE_DATA_HOME")
        .env_remove("XDG_DATA_HOME")
        .args(["--claude-home"])
        .arg(&f.claude)
        .arg("--codex-home")
        .arg(&f.codex)
        .arg("--state-dir")
        .arg(&f.state)
        .args([
            "edit",
            "--agent",
            "claude",
            "--id",
            "skill:demo-skill",
            "--json",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(br#"{"name":"demo-skill","description":"d","category":null,"body":"x"}"#)
        .unwrap();
    let out = child.wait_with_output().unwrap();
    let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(doc["code"], "unsupported");
}

#[test]
fn skills_visible_in_the_shared_directory_are_not_copied_and_differences_are_reported() {
    let f = four_agents();
    let shared = f.state.with_file_name("home").join(".agents/skills");
    skill_dir(&shared, "same-skill", "same");
    skill_dir(&shared, "other-skill", "shared text");
    skill_dir(&skills_of(&f, "claude"), "same-skill", "same");
    skill_dir(&skills_of(&f, "claude"), "other-skill", "claude text");

    let (code, doc) = f.json(&["sync", "--apply", "--json"]);
    assert_eq!(code, Some(2), "{doc}");
    assert_eq!(doc["notes"].as_array().unwrap().len(), 2);
    for agent in ["codex", "pi"] {
        assert!(!skills_of(&f, agent).join("same-skill").exists(), "{agent}");
        assert!(
            !skills_of(&f, agent).join("other-skill").exists(),
            "{agent}"
        );
    }
    assert!(skills_of(&f, "opencode").join("same-skill").is_dir());
    assert!(shared.join("other-skill/SKILL.md").is_file());
}

#[test]
fn doctor_probes_the_skills_directory_and_names_shadowed_copies() {
    let f = four_agents();
    let shared = f.state.with_file_name("home").join(".agents/skills");
    skill_dir(&shared, "demo-skill", "shared text");
    skill_dir(&skills_of(&f, "codex"), "demo-skill", "own text");
    let (code, doc) = f.json(&["doctor", "--json"]);
    assert_eq!(code, Some(0), "{doc}");
    for a in doc["agents"].as_array().unwrap() {
        let id = a["id"].as_str().unwrap();
        if a["installed"] == true {
            let ids: Vec<&str> = a["checks"]
                .as_array()
                .unwrap()
                .iter()
                .map(|c| c["id"].as_str().unwrap())
                .collect();
            assert!(
                ids.contains(&format!("{id}.skills_layout").as_str()),
                "{id}"
            );
        }
    }
    let codex = doc["agents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["id"] == "codex")
        .unwrap();
    assert!(codex["notices"][0].as_str().unwrap().contains("demo-skill"));
}

#[test]
fn identical_skills_in_every_agent_are_adopted_without_writes() {
    let f = four_agents();
    for agent in ["claude", "codex", "opencode", "pi"] {
        skill_dir(&skills_of(&f, agent), "demo-skill", "same");
    }
    let (code, doc) = f.json(&["sync", "--apply", "--json"]);
    assert_eq!(code, Some(0), "{doc}");
    assert!(doc["actions"]
        .as_array()
        .unwrap()
        .iter()
        .all(|a| !a["origin"]["native_id"]
            .as_str()
            .unwrap()
            .starts_with("skill:")));
}

#[test]
fn a_skills_directory_that_stops_looking_like_one_pauses_skills_for_that_agent() {
    let f = four_agents();
    for n in ["a", "b", "c"] {
        fs::create_dir_all(skills_of(&f, "claude").join(n)).unwrap();
    }
    skill_dir(&skills_of(&f, "claude"), "demo-skill", "text");
    let (code, doc) = f.json(&["doctor", "--json"]);
    assert_eq!(code, Some(3), "{doc}");
    let claude = &doc["agents"][0];
    assert_eq!(claude["paused"], json!(["skill"]));
    let (_, plan) = f.json(&["sync", "--apply", "--json"]);
    assert!(plan["actions"]
        .as_array()
        .unwrap()
        .iter()
        .all(|a| a["name"] != "demo-skill"));
    assert!(!skills_of(&f, "codex").join("demo-skill").exists());
}
