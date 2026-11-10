//! `nativemem subagent` and the subagent tools of `nativemem mcp serve` over temp roots. HOME is
//! redirected and every agent root is explicit. The harnesses are fake shell scripts in a temp
//! directory that is the only place the binaries are searched in, so no real agent is started.

use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

use serde_json::{json, Value};

struct Fx {
    _t: tempfile::TempDir,
    root: PathBuf,
}

impl Fx {
    fn new() -> Fx {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().to_path_buf();
        for d in ["claude", "codex", "opencode", "pi", "home", "bin"] {
            fs::create_dir_all(root.join(d)).unwrap();
        }
        Fx { _t: t, root }
    }

    fn state(&self) -> PathBuf {
        self.root.join("state")
    }

    fn bin(&self) -> PathBuf {
        self.root.join("bin")
    }

    /// A fake harness executable.
    fn harness(&self, name: &str, body: &str) {
        let path = self.bin().join(name);
        fs::write(&path, format!("#!/bin/sh\n{body}")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_nativemem"));
        c.env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("CODEX_HOME")
            .env_remove("OPENCODE_CONFIG_DIR")
            .env_remove("PI_CODING_AGENT_DIR")
            .env_remove("NATIVEMEM_CLAUDE_HOME")
            .env_remove("NATIVEMEM_CODEX_HOME")
            .env_remove("NATIVEMEM_OPENCODE_HOME")
            .env_remove("NATIVEMEM_PI_HOME")
            .env_remove("NATIVEMEM_STATE_DIR")
            .env_remove("NATIVEMEM_SUBAGENT_DEPTH")
            .env("HOME", self.root.join("home"))
            .arg("--claude-home")
            .arg(self.root.join("claude"))
            .arg("--codex-home")
            .arg(self.root.join("codex"))
            .arg("--opencode-home")
            .arg(self.root.join("opencode"))
            .arg("--pi-home")
            .arg(self.root.join("pi"))
            .arg("--state-dir")
            .arg(self.state())
            .arg("subagent")
            .arg("--search-path")
            .arg(self.bin())
            .args(args);
        c
    }

    fn run_with(&self, mut command: Command, stdin: &str) -> Output {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(stdin.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    }

    fn json(&self, args: &[&str], stdin: &str) -> (i32, Value) {
        let o = self.run_with(self.command(args), stdin);
        let out = String::from_utf8_lossy(&o.stdout);
        let doc = serde_json::from_str(&out).unwrap_or_else(|e| {
            panic!(
                "not JSON ({e}): {out}\n{}",
                String::from_utf8_lossy(&o.stderr)
            )
        });
        (o.status.code().unwrap(), doc)
    }

    fn subagents_toml(&self) -> String {
        fs::read_to_string(self.state().join("subagents.toml")).unwrap_or_default()
    }
}

fn spec(name: &str, harness: &str) -> String {
    json!({ "name": name, "harness": harness, "model": "the-model", "description": "Does things" })
        .to_string()
}

#[test]
fn add_list_edit_remove() {
    let fx = Fx::new();
    fx.harness("pi", "echo ok\n");
    fx.harness("claude", "echo ok\n");

    let (code, doc) = fx.json(&["list", "--json"], "");
    assert_eq!((code, doc), (0, json!({ "subagents": [] })));

    let (code, doc) = fx.json(&["add", "--json"], &spec("reviewer", "pi"));
    assert_eq!((code, &doc), (0, &json!({ "added": "reviewer" })));
    let full = json!({
        "name": "scout", "harness": "claude", "model": "sonnet", "description": "Finds code",
        "instructions": "Be terse.", "timeout_secs": 60, "args": ["--allowed-tools", "Read"]
    });
    let (code, _) = fx.json(&["add", "--json"], &full.to_string());
    assert_eq!(code, 0);

    let (code, doc) = fx.json(&["add", "--json"], &spec("reviewer", "pi"));
    assert_eq!(code, 1);
    assert_eq!(doc["code"], "exists", "{doc}");

    let (_, doc) = fx.json(&["list", "--json"], "");
    assert_eq!(
        doc["subagents"],
        json!([
            { "name": "reviewer", "harness": "pi", "model": "the-model",
              "description": "Does things", "timeout_secs": 900, "args": [] },
            { "name": "scout", "harness": "claude", "model": "sonnet",
              "description": "Finds code", "instructions": "Be terse.",
              "timeout_secs": 60, "args": ["--allowed-tools", "Read"] },
        ])
    );
    let text = fx.run_with(fx.command(&["list"]), "");
    let text = String::from_utf8_lossy(&text.stdout).into_owned();
    assert!(
        text.contains("reviewer  pi  the-model  900 s  Does things"),
        "{text}"
    );
    assert!(fx.subagents_toml().contains("[agents.reviewer]"));

    let edited = json!({ "harness": "pi", "model": "other-model", "description": "Changed" });
    let (code, doc) = fx.json(&["edit", "reviewer", "--json"], &edited.to_string());
    assert_eq!((code, &doc), (0, &json!({ "edited": "reviewer" })));
    let (_, doc) = fx.json(&["list", "--json"], "");
    assert_eq!(doc["subagents"][0]["model"], "other-model");
    assert_eq!(doc["subagents"][0]["description"], "Changed");
    // The file before the edit is backed up.
    let backups = fx.state().join("backups");
    let stamp = fs::read_dir(&backups)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let old = fs::read_to_string(stamp.join("hub/subagents.toml")).unwrap();
    assert!(old.contains("the-model"), "{old}");

    let (code, doc) = fx.json(&["edit", "missing", "--json"], &edited.to_string());
    assert_eq!((code, doc["code"].as_str()), (1, Some("not_found")));
    let mismatch = json!({ "name": "scout", "harness": "pi", "model": "m", "description": "d" });
    let (_, doc) = fx.json(&["edit", "reviewer", "--json"], &mismatch.to_string());
    assert_eq!(doc["code"], "invalid", "{doc}");

    let (code, doc) = fx.json(&["remove", "reviewer", "--json"], "");
    assert_eq!((code, &doc), (0, &json!({ "removed": "reviewer" })));
    let (_, doc) = fx.json(&["remove", "reviewer", "--json"], "");
    assert_eq!(doc["code"], "not_found", "{doc}");
    let (_, doc) = fx.json(&["list", "--json"], "");
    assert_eq!(doc["subagents"].as_array().unwrap().len(), 1);
}

#[test]
fn invalid_definitions_are_refused_before_anything_is_written() {
    let fx = Fx::new();
    fx.harness("pi", "echo ok\n");
    let cases = [
        spec("Bad_Name", "pi"),
        spec(&"a".repeat(49), "pi"),
        spec("ok", "hermes"),
        r#"{"harness":"pi","model":"m","description":"d"}"#.to_string(),
        json!({ "name": "a", "harness": "pi", "model": "-x", "description": "d" }).to_string(),
        json!({ "name": "a", "harness": "pi", "model": "m", "description": "d", "timeout_secs": 0 })
            .to_string(),
        json!({ "name": "a", "harness": "pi", "model": "m", "description": "d", "bogus": 1 })
            .to_string(),
        "not json".to_string(),
    ];
    for case in cases {
        let (code, doc) = fx.json(&["add", "--json"], &case);
        assert_eq!(
            (code, doc["code"].as_str()),
            (1, Some("invalid")),
            "{case}: {doc}"
        );
    }
    assert!(!fx.state().join("subagents.toml").exists());
}

#[test]
fn add_and_edit_need_the_harness_binary() {
    let fx = Fx::new();
    fx.harness("pi", "echo ok\n");
    let (code, doc) = fx.json(&["add", "--json"], &spec("a", "codex"));
    assert_eq!(
        (code, doc["code"].as_str()),
        (1, Some("not_installed")),
        "{doc}"
    );
    assert!(!fx.state().join("subagents.toml").exists());

    fx.json(&["add", "--json"], &spec("a", "pi"));
    let (code, doc) = fx.json(
        &["edit", "a", "--json"],
        &json!({ "harness": "opencode", "model": "m", "description": "d" }).to_string(),
    );
    assert_eq!(
        (code, doc["code"].as_str()),
        (1, Some("not_installed")),
        "{doc}"
    );
    assert!(fx.subagents_toml().contains("harness = \"pi\""));
}

#[test]
fn run_prints_the_answer_or_the_failure() {
    let fx = Fx::new();
    fx.harness("pi", "echo \"pi saw: $5\"\n");
    fx.harness("claude", "echo boom >&2\nexit 4\n");
    fx.json(&["add", "--json"], &spec("ok", "pi"));
    fx.json(&["add", "--json"], &spec("bad", "claude"));
    let prompt = fx.root.join("prompt.txt");
    fs::write(&prompt, "from a file").unwrap();
    let prompt = prompt.to_str().unwrap();

    let (code, doc) = fx.json(&["run", "ok", "--prompt-file", prompt, "--json"], "");
    assert_eq!(code, 0, "{doc}");
    assert_eq!(doc["ok"], true);
    assert_eq!(doc["text"], "pi saw: from a file");
    assert_eq!(doc["exit_code"], 0);
    assert_eq!(doc["error"], Value::Null);
    assert!(doc["duration_ms"].is_u64());

    let cwd = fx.root.join("home");
    let cwd = cwd.to_str().unwrap();
    let (code, doc) = fx.json(
        &["run", "ok", "--prompt-file", "-", "--cwd", cwd, "--json"],
        "from stdin",
    );
    assert_eq!((code, &doc["text"]), (0, &json!("pi saw: from stdin")));

    let (code, doc) = fx.json(&["run", "bad", "--prompt-file", prompt, "--json"], "");
    assert_eq!(code, 2);
    assert_eq!(doc["ok"], false);
    assert_eq!(doc["exit_code"], 4);
    assert_eq!(doc["text"], "");
    assert!(doc["error"]
        .as_str()
        .unwrap()
        .starts_with("claude exited with code 4"));
    assert!(doc["error"].as_str().unwrap().ends_with("boom"));

    let (code, doc) = fx.json(&["run", "missing", "--prompt-file", prompt, "--json"], "");
    assert_eq!((code, doc["code"].as_str()), (1, Some("not_found")));
    let (_, doc) = fx.json(&["run", "ok", "--prompt-file", "-", "--json"], "  \n");
    assert_eq!(doc["code"], "invalid", "{doc}");
    let (_, doc) = fx.json(
        &["run", "ok", "--prompt-file", "/nonexistent/p", "--json"],
        "",
    );
    assert_eq!(doc["code"], "io", "{doc}");

    // A run started from inside a subagent is refused.
    let mut nested = fx.command(&["run", "ok", "--prompt-file", prompt, "--json"]);
    nested.env("NATIVEMEM_SUBAGENT_DEPTH", "1");
    let o = fx.run_with(nested, "");
    assert_eq!(o.status.code(), Some(2));
    let doc: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(doc["ok"], false);
    assert!(doc["error"]
        .as_str()
        .unwrap()
        .contains("cannot start subagents"));
}

#[test]
fn writing_commands_are_refused_while_the_state_lock_is_held() {
    let fx = Fx::new();
    fx.harness("pi", "echo ok\n");
    fx.json(&["add", "--json"], &spec("a", "pi"));
    let held = fs::File::create(fx.state().join("sync.lock")).unwrap();
    held.try_lock().unwrap();

    for (args, stdin) in [
        (vec!["add", "--json"], spec("b", "pi")),
        (vec!["edit", "a", "--json"], spec("a", "pi")),
        (vec!["remove", "a", "--json"], String::new()),
    ] {
        let (code, doc) = fx.json(&args, &stdin);
        assert_eq!((code, doc["code"].as_str()), (1, Some("busy")), "{args:?}");
    }
    let (code, doc) = fx.json(&["list", "--json"], "");
    assert_eq!(code, 0);
    assert_eq!(doc["subagents"].as_array().unwrap().len(), 1);
    drop(held);
    let (code, _) = fx.json(&["remove", "a", "--json"], "");
    assert_eq!(code, 0);
}

#[tokio::test]
async fn serve_exposes_the_definitions_as_tools_and_runs_them() {
    use rmcp::model::{CallToolRequestParams, ClientCapabilities, ClientConfig, Implementation};
    use rmcp::transport::TokioChildProcess;
    use rmcp::ServiceExt;

    let fx = Fx::new();
    fx.harness(
        "pi",
        "echo \"pi saw: $5 at $(pwd) depth $NATIVEMEM_SUBAGENT_DEPTH\"\n",
    );
    let definition = json!({
        "name": "helper", "harness": "pi", "model": "pi-model", "description": "Helps out"
    });
    fx.json(&["add", "--json"], &definition.to_string());

    // The hub finds the harness through the PATH it is started with; the fake directory comes
    // first, and the only subagent uses a harness that has a fake.
    let mut cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_nativemem"));
    let probe = fx.command(&["list"]);
    let mut args: Vec<_> = probe.get_args().map(|a| a.to_owned()).collect();
    let at = args.iter().position(|a| a == "subagent").unwrap();
    args.truncate(at);
    args.extend(["mcp".into(), "serve".into()]);
    cmd.args(&args);
    for (k, v) in probe.get_envs() {
        match v {
            Some(v) => cmd.env(k, v),
            None => cmd.env_remove(k),
        };
    }
    cmd.env("PATH", format!("{}:/usr/bin:/bin", fx.bin().display()));
    let client = ClientConfig::new(
        ClientCapabilities::default(),
        Implementation::new("test-client", "0"),
    )
    .serve(TokioChildProcess::new(cmd).unwrap())
    .await
    .expect("the hub answers initialize");

    let tools = client.peer().list_all_tools().await.unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    assert_eq!(
        names,
        ["nativemem_status", "subagents_list", "subagent_helper"]
    );
    let helper = tools.iter().find(|t| t.name == "subagent_helper").unwrap();
    assert_eq!(
        helper.description.as_deref(),
        Some("Helps out (harness pi, model pi-model)")
    );

    let cwd = fx.root.join("home").canonicalize().unwrap();
    let result = client
        .peer()
        .call_tool(
            CallToolRequestParams::new("subagent_helper".to_string()).with_arguments(
                json!({ "prompt": "go", "cwd": cwd.to_str().unwrap() })
                    .as_object()
                    .cloned()
                    .unwrap(),
            ),
        )
        .await
        .unwrap();
    let text: String = result
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
        .collect();
    assert_ne!(result.is_error, Some(true), "{text}");
    assert_eq!(text, format!("pi saw: go at {} depth 1", cwd.display()));
}
