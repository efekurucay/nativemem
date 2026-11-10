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
        .env("NATIVEMEM_CODEX_SESSION_WRITES", "on")
        .env_remove("NATIVEMEM_OPENCODE_HOME")
        .env_remove("NATIVEMEM_PI_HOME")
        .arg("--claude-home")
        .arg(claude)
        .arg("--codex-home")
        .arg(codex)
        .arg("--opencode-home")
        .arg(state.with_file_name("opencode"))
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
            { "id": "claude", "originals": 1, "replicas": 0, "sessions": 0, "automated_sessions": 0, "skipped": [] },
            { "id": "codex", "originals": 0, "replicas": 0, "sessions": 0, "automated_sessions": 0, "skipped": [] } ],
          "ledger_items": 0 })
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
        .env("NATIVEMEM_CODEX_SESSION_WRITES", "on")
        .env_remove("NATIVEMEM_OPENCODE_HOME")
        .env_remove("NATIVEMEM_PI_HOME")
        .arg("--claude-home")
        .arg(&f.claude)
        .arg("--codex-home")
        .arg(&f.codex)
        .arg("--opencode-home")
        .arg(f.state.with_file_name("opencode"))
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
        assert!(plist.contains("<string>sync</string>\n        <string>--apply</string>"));
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
    }
    assert_eq!(list.iter().filter(|i| i["replica_of"].is_null()).count(), 1);
    let pi = list.iter().find(|i| i["agent"] == "pi").unwrap();
    assert_eq!(pi["replica_of"]["origin"], "claude");
    let human = text(&run(&f.claude, &f.codex, &f.state, &["items"]));
    assert!(!human.contains("session"), "{human}");
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
        .env("NATIVEMEM_CODEX_SESSION_WRITES", "on")
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
