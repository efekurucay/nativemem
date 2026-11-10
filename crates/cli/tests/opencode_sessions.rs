//! OpenCode sessions through the CLI, on temp roots with `HOME` redirected.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use nativemem_core::testkit::{ClaudeHome, OpencodeData};
use serde_json::{json, Value};

struct Fx {
    _t: tempfile::TempDir,
    root: PathBuf,
    claude: PathBuf,
    data: OpencodeData,
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
    fs::create_dir_all(root.join("opencode")).unwrap();
    let data = OpencodeData::new(&root.join("opencode-data"));
    data.write_db(&[]);
    Fx {
        claude: root.join("claude"),
        state: root.join("state"),
        data,
        root,
        _t: t,
    }
}

impl Fx {
    fn command(&self) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_nativemem"));
        cmd.env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("CODEX_HOME")
            .env_remove("OPENCODE_CONFIG_DIR")
            .env_remove("PI_CODING_AGENT_DIR")
            .env_remove("NATIVEMEM_OPENCODE_HOME")
            .env_remove("NATIVEMEM_OPENCODE_DATA_HOME")
            .env_remove("NATIVEMEM_PI_HOME")
            .env_remove("XDG_DATA_HOME")
            .env("HOME", self.root.join("home"))
            .env("PATH", self.root.join("bin"));
        cmd.arg("--claude-home")
            .arg(&self.claude)
            .arg("--codex-home")
            .arg(self.root.join("codex"))
            .arg("--opencode-home")
            .arg(self.root.join("opencode"))
            .arg("--pi-home")
            .arg(self.root.join("pi"))
            .arg("--state-dir")
            .arg(&self.state);
        cmd
    }

    fn run(&self, args: &[&str]) -> Output {
        let mut cmd = self.command();
        cmd.arg("--opencode-data-home")
            .arg(self.data.root.clone())
            .args(args);
        cmd.output().unwrap()
    }

    fn json(&self, args: &[&str]) -> (Option<i32>, Value) {
        let o = self.run(args);
        let doc = serde_json::from_slice(&o.stdout)
            .unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&o.stdout)));
        (o.status.code(), doc)
    }

    fn sync(&self, apply: bool) -> (Option<i32>, Value) {
        let mut args = vec!["sync", "--json"];
        if apply {
            args.push("--apply");
        }
        self.json(&args)
    }

    /// The agent entry of a `status` or `doctor` document that carries `notices`.
    fn agent<'a>(&self, doc: &'a Value, id: &str) -> &'a Value {
        let agents = if doc["compat"].is_null() {
            &doc["agents"]
        } else {
            &doc["compat"]["agents"]
        };
        agents
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["id"] == id)
            .unwrap_or_else(|| panic!("{doc}"))
    }

    /// The agent entry of a `status` document that carries the session counts.
    fn counts<'a>(&self, doc: &'a Value, id: &str) -> &'a Value {
        doc["agents"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["id"] == id)
            .unwrap_or_else(|| panic!("{doc}"))
    }

    fn claude_sessions(&self) -> usize {
        fs::read_dir(self.claude.join("projects/-w-app"))
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "jsonl"))
            .count()
    }
}

#[test]
fn a_synced_session_reaches_opencode_and_the_second_plan_is_empty() {
    let f = fx();
    let (code, _) = f.sync(true);
    assert_eq!(code, Some(0));
    assert_eq!(f.data.session_ids().len(), 1);
    let (code, plan) = f.sync(false);
    assert_eq!(code, Some(0));
    assert_eq!(plan["actions"], json!([]), "{plan}");
    assert_eq!(plan["notes"], json!([]), "{plan}");
    f.sync(true);
    assert_eq!(f.data.session_ids().len(), 1, "a session id appears once");
    let backups = f.state.join("backups");
    let stamps = fs::read_dir(backups)
        .unwrap()
        .flatten()
        .filter(|e| e.path().join("opencode/opencode.db").is_file())
        .count();
    assert_eq!(stamps, 1, "one backup for the one run that wrote");
}

#[test]
fn opencode_shows_no_session_writes_notice() {
    let f = fx();
    for command in ["status", "doctor"] {
        let (_, doc) = f.json(&[command, "--json"]);
        let opencode = f.agent(&doc, "opencode");
        assert_eq!(opencode["notices"], json!([]), "{command}: {doc}");
        assert!(opencode["status"] == "ok" || opencode["status"] == "updated");
    }
    let o = f.run(&["status"]);
    assert!(!String::from_utf8_lossy(&o.stdout).contains("session writes are off"));
}

#[test]
fn opencode_sessions_are_read_as_a_source() {
    let f = fx();
    f.data.add_session(
        "ses_native",
        Path::new("/w/app"),
        Some("From opencode"),
        &[("user", "hello"), ("assistant", "hi")],
        1_794_304_900_000,
    );
    let (_, status) = f.json(&["status", "--json"]);
    assert_eq!(f.counts(&status, "opencode")["sessions"], 1);
    assert_eq!(f.counts(&status, "opencode")["automated_sessions"], 0);
    let before = f.claude_sessions();
    f.sync(true);
    assert_eq!(
        f.claude_sessions(),
        before + 1,
        "the OpenCode session became a Claude replica"
    );
    let ids = f.data.session_ids();
    assert!(ids.contains(&"ses_native".to_string()), "{ids:?}");
    assert_eq!(
        ids.len(),
        2,
        "the Claude session became an OpenCode replica"
    );
    let (_, plan) = f.sync(false);
    assert_eq!(plan["actions"], json!([]), "{plan}");
}

#[test]
fn a_drifted_schema_pauses_opencode_sessions_in_both_directions() {
    let f = fx();
    fs::remove_file(f.data.db()).unwrap();
    f.data.write_db(&["brand_new INTEGER NOT NULL DEFAULT 0"]);
    f.data.add_session(
        "ses_native",
        Path::new("/w/app"),
        Some("From opencode"),
        &[("user", "hello")],
        1_794_304_900_000,
    );
    let (code, doc) = f.json(&["doctor", "--json"]);
    assert_eq!(code, Some(3), "{doc}");
    let opencode = f.agent(&doc, "opencode");
    assert_eq!(opencode["status"], "action_required");
    assert_eq!(opencode["paused"], json!(["session"]));
    let failing: Vec<&Value> = opencode["checks"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["ok"] == false)
        .collect();
    assert_eq!(failing.len(), 1, "{doc}");
    assert_eq!(failing[0]["id"], "opencode.session_schema");
    let before = fs::read(f.data.db()).unwrap();
    let claude_before = f.claude_sessions();
    f.sync(true);
    assert_eq!(fs::read(f.data.db()).unwrap(), before, "nothing written");
    assert_eq!(f.claude_sessions(), claude_before, "nothing read either");
    assert!(
        !f.state.join("backups").exists()
            || !f
                .state
                .join("backups")
                .read_dir()
                .unwrap()
                .flatten()
                .any(|e| e.path().join("opencode").exists())
    );
}

#[test]
fn the_data_home_comes_from_the_flag_the_env_the_xdg_variable_or_the_home_directory() {
    let f = fx();
    f.data.add_session(
        "ses_native",
        Path::new("/w/app"),
        Some("From opencode"),
        &[("user", "hello")],
        1_794_304_900_000,
    );
    let sessions = |mut cmd: Command| -> i64 {
        let o = cmd.args(["status", "--json"]).output().unwrap();
        let doc: Value = serde_json::from_slice(&o.stdout).unwrap();
        f.counts(&doc, "opencode")["sessions"].as_i64().unwrap()
    };
    assert_eq!(sessions(f.command()), 0, "nothing at the default place");

    let mut env = f.command();
    env.env("NATIVEMEM_OPENCODE_DATA_HOME", &f.data.root);
    assert_eq!(sessions(env), 1);

    let xdg = f.root.join("xdg");
    fs::create_dir_all(&xdg).unwrap();
    fs::rename(&f.data.root, xdg.join("opencode")).unwrap();
    let mut xdg_cmd = f.command();
    xdg_cmd.env("XDG_DATA_HOME", &xdg);
    assert_eq!(sessions(xdg_cmd), 1);

    let home_data = f.root.join("home/.local/share");
    fs::create_dir_all(&home_data).unwrap();
    fs::rename(xdg.join("opencode"), home_data.join("opencode")).unwrap();
    assert_eq!(sessions(f.command()), 1);
}

#[test]
fn without_a_database_opencode_takes_no_part_in_sessions_and_shows_no_notice() {
    let f = fx();
    fs::remove_file(f.data.db()).unwrap();
    let (code, _) = f.sync(true);
    assert_eq!(code, Some(0));
    assert!(!f.data.db().exists(), "the database is never created");
    let (_, doc) = f.json(&["doctor", "--json"]);
    assert_eq!(f.agent(&doc, "opencode")["notices"], json!([]));
}

#[test]
fn no_sessions_leaves_the_database_alone() {
    let f = fx();
    let before = fs::read(f.data.db()).unwrap();
    let o = f.run(&["--no-sessions", "sync", "--apply", "--json"]);
    assert_eq!(o.status.code(), Some(0));
    assert_eq!(fs::read(f.data.db()).unwrap(), before);
}

#[test]
fn opencode_sessions_cannot_be_deleted_or_edited_from_here() {
    let f = fx();
    let (code, doc) = f.json(&[
        "delete",
        "--agent",
        "opencode",
        "--id",
        "ses_x.jsonl",
        "--json",
    ]);
    assert_eq!(
        (code, &doc["code"]),
        (Some(1), &json!("unsupported")),
        "{doc}"
    );
}
