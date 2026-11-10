//! Codex thread rows through the CLI, on temp roots with `HOME` redirected.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use nativemem_core::testkit::{ClaudeHome, CodexHome};
use serde_json::{json, Value};

struct Fx {
    _t: tempfile::TempDir,
    claude: PathBuf,
    codex: PathBuf,
    state: PathBuf,
}

fn fx() -> Fx {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().to_path_buf();
    ClaudeHome::new(&root.join("claude")).add_session(
        Path::new("/w/app"),
        "s1",
        Some("Fix login"),
        &[("user", "please fix"), ("assistant", "done")],
    );
    CodexHome::new(&root.join("codex")).write_state_db(5, &[]);
    Fx {
        claude: root.join("claude"),
        codex: root.join("codex"),
        state: root.join("state"),
        _t: t,
    }
}

impl Fx {
    fn run(&self, switch: Option<&str>, args: &[&str]) -> Output {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_nativemem"));
        cmd.env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("CODEX_HOME")
            .env_remove("OPENCODE_CONFIG_DIR")
            .env_remove("PI_CODING_AGENT_DIR")
            .env_remove("NATIVEMEM_OPENCODE_HOME")
            .env_remove("NATIVEMEM_OPENCODE_DATA_HOME")
            .env_remove("NATIVEMEM_OPENCODE_SESSION_WRITES")
            .env_remove("XDG_DATA_HOME")
            .env_remove("NATIVEMEM_PI_HOME")
            .env_remove("NATIVEMEM_CODEX_SESSION_WRITES")
            .env("HOME", self.state.with_file_name("home"));
        if let Some(v) = switch {
            cmd.env("NATIVEMEM_CODEX_SESSION_WRITES", v);
        }
        cmd.arg("--claude-home")
            .arg(&self.claude)
            .arg("--codex-home")
            .arg(&self.codex)
            .arg("--opencode-home")
            .arg(self.state.with_file_name("opencode"))
            .arg("--opencode-data-home")
            .arg(self.state.with_file_name("opencode-data"))
            .arg("--pi-home")
            .arg(self.state.with_file_name("pi"))
            .arg("--state-dir")
            .arg(&self.state)
            .args(args)
            .output()
            .unwrap()
    }

    fn sync(&self, switch: Option<&str>, apply: bool) -> (Option<i32>, Value) {
        let mut args = vec!["sync", "--json"];
        if apply {
            args.push("--apply");
        }
        let o = self.run(switch, &args);
        let doc = serde_json::from_slice(&o.stdout)
            .unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&o.stdout)));
        (o.status.code(), doc)
    }

    fn rows(&self) -> Vec<String> {
        CodexHome::new(&self.codex).thread_ids(5)
    }

    fn rollouts(&self) -> usize {
        fn walk(dir: &Path) -> usize {
            fs::read_dir(dir)
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| {
                    let p = e.path();
                    if p.is_dir() {
                        walk(&p)
                    } else {
                        usize::from(p.extension().is_some_and(|x| x == "jsonl"))
                    }
                })
                .sum()
        }
        walk(&self.codex.join("sessions"))
    }
}

#[test]
fn a_synced_session_has_one_row_and_the_second_plan_is_empty() {
    let f = fx();
    let (code, _) = f.sync(Some("on"), true);
    assert_eq!(code, Some(0));
    assert_eq!(f.rollouts(), 1);
    let rows = f.rows();
    assert_eq!(rows.len(), 1);
    let (code, plan) = f.sync(Some("on"), false);
    assert_eq!(code, Some(0));
    assert_eq!(plan["actions"], json!([]), "{plan}");
    assert_eq!(plan["notes"], json!([]), "{plan}");
    f.sync(Some("on"), true);
    assert_eq!(f.rows(), rows, "a thread id appears once");
}

#[test]
fn a_thread_codex_imported_itself_is_adopted_and_gets_no_second_row() {
    let f = fx();
    let id = "01a1c682-fd13-7263-8871-4ca47590d7d1";
    CodexHome::new(&f.codex).add_rollout(
        id,
        Path::new("/w/app"),
        &[("user", "please fix"), ("assistant", "done")],
        true,
    );
    let source = f.claude.join("projects/-w-app/s1.jsonl");
    let record = json!({"source_path": source.to_string_lossy(), "content_sha256": "00",
        "imported_thread_id": id, "imported_at": 1, "source_modified_at": 2,
        "connector_names": [], "title": "t"});
    fs::write(
        f.codex.join("external_agent_session_imports.json"),
        json!({"records": [record], "detected_connector_records": []}).to_string(),
    )
    .unwrap();
    CodexHome::new(&f.codex).insert_thread(5, id, Path::new("/w/app"));
    f.sync(Some("on"), true);
    assert_eq!(f.rows(), vec![id.to_string()]);
    assert_eq!(f.rollouts(), 1);
    let (_, plan) = f.sync(Some("on"), false);
    assert_eq!(plan["actions"], json!([]), "{plan}");
}

#[test]
fn codex_session_writes_are_opt_in_and_the_notice_says_so() {
    let f = fx();
    let (code, plan) = f.sync(None, false);
    assert_eq!((code, &plan["notes"]), (Some(0), &json!([])), "{plan}");
    let (code, _) = f.sync(None, true);
    assert_eq!(code, Some(0));
    assert_eq!((f.rollouts(), f.rows().len()), (0, 0));
    let notice = "session writes are off; set NATIVEMEM_CODEX_SESSION_WRITES=on to let nativemem write sessions into Codex";
    for command in ["status", "doctor"] {
        let o = f.run(None, &[command, "--json"]);
        let doc: Value = serde_json::from_slice(&o.stdout).unwrap();
        let agents = if command == "status" {
            &doc["compat"]["agents"]
        } else {
            &doc["agents"]
        };
        let codex = agents
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["id"] == "codex")
            .unwrap_or_else(|| panic!("{doc}"));
        assert_eq!(codex["notices"], json!([notice]), "{command}: {doc}");
        assert!(codex["status"] == "ok" || codex["status"] == "updated");
    }
    let o = f.run(Some("on"), &["status", "--json"]);
    assert!(!String::from_utf8_lossy(&o.stdout).contains("session writes are off"));
}

#[test]
fn drifted_schema_pauses_codex_sessions() {
    let f = fx();
    fs::remove_file(f.codex.join("state_5.sqlite")).unwrap();
    CodexHome::new(&f.codex).write_state_db(5, &["brand_new INTEGER NOT NULL DEFAULT 0"]);
    f.sync(Some("on"), true);
    assert_eq!((f.rollouts(), f.rows().len()), (0, 0));
}
