//! `nativemem mcp` on fixtures in temp directories. HOME is redirected, every agent root and the
//! secret store are explicit, so no real agent home, keychain or state directory is touched. The
//! upstream is `nativemem-fake-mcp` from the hub crate.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::OnceLock;

use serde_json::{json, Value};

#[path = "../../hub/tests/common/fake_oauth.rs"]
mod fake_oauth;

/// The fake upstream server. `cargo test -p nativemem-cli` does not build another package's
/// binary, so it is built here when it is missing next to the CLI binary.
fn fake_server() -> &'static Path {
    static PATH: OnceLock<PathBuf> = OnceLock::new();
    PATH.get_or_init(|| {
        let cli = PathBuf::from(env!("CARGO_BIN_EXE_nativemem"));
        let fake = cli.with_file_name(format!(
            "nativemem-fake-mcp{}",
            std::env::consts::EXE_SUFFIX
        ));
        if !fake.exists() {
            let mut build = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
            build.args([
                "build",
                "-p",
                "nativemem-hub",
                "--bin",
                "nativemem-fake-mcp",
            ]);
            if cli.components().any(|c| c.as_os_str() == "release") {
                build.arg("--release");
            }
            assert!(
                build.status().unwrap().success(),
                "cannot build the fake server"
            );
        }
        assert!(fake.exists(), "{} is missing", fake.display());
        fake
    })
}

struct Fx {
    _t: tempfile::TempDir,
    root: PathBuf,
}

impl Fx {
    fn new() -> Fx {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().to_path_buf();
        for d in ["claude", "codex", "opencode", "pi", "home"] {
            fs::create_dir_all(root.join(d)).unwrap();
        }
        Fx { _t: t, root }
    }

    fn state(&self) -> PathBuf {
        self.root.join("state")
    }

    fn secrets(&self) -> PathBuf {
        self.root.join("secrets")
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
            .arg("mcp")
            .arg("--secrets-dir")
            .arg(self.secrets())
            .args(args);
        c
    }

    fn run(&self, args: &[&str], stdin: &str) -> Output {
        let mut child = self
            .command(args)
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
        let o = self.run(args, stdin);
        let out = String::from_utf8_lossy(&o.stdout);
        let doc = serde_json::from_str(&out).unwrap_or_else(|e| {
            panic!(
                "not JSON ({e}): {out}\n{}",
                String::from_utf8_lossy(&o.stderr)
            )
        });
        (o.status.code().unwrap(), doc)
    }

    fn hub_toml(&self) -> String {
        fs::read_to_string(self.state().join("hub.toml")).unwrap_or_default()
    }
}

fn fake_spec(name: &str) -> String {
    json!({ "name": name, "transport": "stdio", "command": fake_server().to_str().unwrap() })
        .to_string()
}

#[test]
fn add_list_edit_enable_disable_remove() {
    let fx = Fx::new();
    let (code, doc) = fx.json(&["add", "--json"], &fake_spec("alpha"));
    assert_eq!((code, &doc), (0, &json!({ "added": "alpha" })));

    let (code, doc) = fx.json(&["add", "--json"], &fake_spec("alpha"));
    assert_eq!(code, 1);
    assert_eq!(doc["code"], "exists", "{doc}");

    let (_, doc) = fx.json(&["add", "--json"], r#"{"transport":"stdio"}"#);
    assert_eq!(doc["code"], "invalid", "{doc}");
    let (_, doc) = fx.json(&["add", "--json"], &fake_spec("nativemem"));
    assert_eq!(doc["code"], "invalid", "{doc}");

    let spec = json!({
        "name": "beta", "transport": "http", "url": "https://mcp.example.com/mcp",
        "headers": { "X-Api-Key": "secret:beta-token", "X-Plain": "plain-header-value" },
        "auth": "oauth",
    });
    let (code, _) = fx.json(&["add", "--json"], &spec.to_string());
    assert_eq!(code, 0);
    let bearer = json!({ "name": "gamma", "transport": "http", "url": "https://mcp.example.com/mcp",
        "headers": { "Authorization": "secret:g" }, "auth": "oauth" });
    let (_, doc) = fx.json(&["add", "--json"], &bearer.to_string());
    assert_eq!(doc["code"], "invalid", "{doc}");

    let (code, doc) = fx.json(&["list", "--json"], "");
    assert_eq!(code, 0);
    let servers = doc["servers"].as_array().unwrap();
    assert_eq!(servers.len(), 2);
    let beta = servers.iter().find(|s| s["name"] == "beta").unwrap();
    assert_eq!(beta["transport"], "http");
    assert_eq!(beta["auth"], "oauth");
    assert_eq!(beta["enabled"], true);
    assert_eq!(beta["secrets"], json!(["beta-token"]));
    assert_eq!(beta["headers"]["X-Api-Key"], "secret:beta-token");
    assert_eq!(beta["auth_state"], "needs_login");
    // A literal value may be a secret: it is never printed.
    assert_eq!(beta["headers"]["X-Plain"], "<literal>");
    assert!(!doc.to_string().contains("plain-header-value"));

    let (code, doc) = fx.json(&["disable", "alpha", "--json"], "");
    assert_eq!(
        (code, &doc),
        (0, &json!({ "name": "alpha", "enabled": false }))
    );
    assert!(fx.hub_toml().contains("enabled = false"));
    let (_, doc) = fx.json(&["enable", "alpha", "--json"], "");
    assert_eq!(doc["enabled"], true);

    let edit = json!({ "transport": "stdio", "command": "/bin/other", "args": ["-x"] });
    let (code, doc) = fx.json(&["edit", "alpha", "--json"], &edit.to_string());
    assert_eq!((code, &doc), (0, &json!({ "edited": "alpha" })));
    let (_, doc) = fx.json(&["list", "--json"], "");
    let alpha = doc["servers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "alpha")
        .unwrap();
    assert_eq!(alpha["command"], "/bin/other");
    let (_, doc) = fx.json(&["edit", "alpha", "--json"], &fake_spec("someone-else"));
    assert_eq!(doc["code"], "invalid", "{doc}");

    let (code, doc) = fx.json(&["remove", "alpha", "--json"], "");
    assert_eq!((code, &doc), (0, &json!({ "removed": "alpha" })));
    let (_, doc) = fx.json(&["remove", "alpha", "--json"], "");
    assert_eq!(doc["code"], "not_found", "{doc}");
    let (_, doc) = fx.json(&["enable", "alpha", "--json"], "");
    assert_eq!(doc["code"], "not_found", "{doc}");

    // Every write backed up the file it replaced.
    let backups = fx.state().join("backups");
    assert!(
        fs::read_dir(&backups)
            .unwrap()
            .any(|e| e.unwrap().path().join("hub/hub.toml").is_file()),
        "no hub.toml backup under {}",
        backups.display()
    );
    // Without --json the output is a line of text.
    let o = fx.run(&["list"], "");
    assert!(String::from_utf8_lossy(&o.stdout).contains("beta  http  enabled  auth oauth"));
}

#[test]
fn secrets_are_stored_listed_by_reference_and_deleted() {
    let fx = Fx::new();
    let value = "tok-0123456789";
    let (code, doc) = fx.json(
        &["set-secret", "api-token", "--json"],
        &format!("{value}\n"),
    );
    assert_eq!((code, &doc), (0, &json!({ "secret": "api-token" })));
    assert!(!doc.to_string().contains(value));
    // The trailing newline of a piped value is not part of the secret.
    let stored = fs::read_to_string(fx.secrets().join("secrets.json")).unwrap();
    assert!(stored.contains(&format!("\"{value}\"")), "{stored}");

    let spec = json!({ "name": "s", "transport": "stdio", "command": "/bin/x",
                       "env": { "TOKEN": "secret:api-token" } });
    fx.json(&["add", "--json"], &spec.to_string());
    let o = fx.run(&["list", "--json"], "");
    let out = format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(out.contains("secret:api-token"));
    assert!(!out.contains(value));
    assert!(!fx.hub_toml().contains(value));

    let (_, doc) = fx.json(&["set-secret", "bad id", "--json"], "x");
    assert_eq!(doc["code"], "invalid", "{doc}");
    let (_, doc) = fx.json(&["set-secret", "empty", "--json"], "\n");
    assert_eq!(doc["code"], "invalid", "{doc}");

    let (code, doc) = fx.json(&["delete-secret", "api-token", "--json"], "");
    assert_eq!((code, &doc), (0, &json!({ "deleted": "api-token" })));
    let (code, doc) = fx.json(&["delete-secret", "api-token", "--json"], "");
    assert_eq!((code, doc["code"].as_str()), (1, Some("not_found")));
}

#[test]
fn test_connects_lists_tools_and_reports_failures() {
    let fx = Fx::new();
    fx.json(&["add", "--json"], &fake_spec("good"));
    let broken = json!({ "name": "broken", "transport": "stdio", "command": "/nonexistent/x" });
    fx.json(&["add", "--json"], &broken.to_string());
    let missing = json!({ "name": "nosecret", "transport": "stdio",
        "command": fake_server().to_str().unwrap(), "env": { "K": "secret:absent" } });
    fx.json(&["add", "--json"], &missing.to_string());

    let (code, doc) = fx.json(&["test", "good", "--json"], "");
    assert_eq!(code, 0);
    assert_eq!(
        doc,
        json!({ "ok": true, "tools": ["echo", "env_matches"], "error": null })
    );

    let (code, doc) = fx.json(&["test", "broken", "--json"], "");
    assert_eq!(code, 2);
    assert_eq!(doc["ok"], false);
    assert!(
        doc["error"].as_str().is_some_and(|e| !e.is_empty()),
        "{doc}"
    );

    let (code, doc) = fx.json(&["test", "nosecret", "--json"], "");
    assert_eq!(code, 2);
    assert!(doc["error"].as_str().unwrap().contains("absent"), "{doc}");

    let (_, doc) = fx.json(&["test", "nope", "--json"], "");
    assert_eq!(doc["code"], "not_found", "{doc}");
}

#[tokio::test]
async fn serve_proxies_the_fake_server_over_a_child_process() {
    use rmcp::model::{CallToolRequestParams, ClientCapabilities, ClientConfig, Implementation};
    use rmcp::transport::TokioChildProcess;
    use rmcp::ServiceExt;

    let fx = Fx::new();
    let value = "env-secret-777";
    fx.json(&["set-secret", "tok", "--json"], value);
    let spec = json!({ "name": "fake", "transport": "stdio",
        "command": fake_server().to_str().unwrap(), "env": { "API_TOKEN": "secret:tok" } });
    fx.json(&["add", "--json"], &spec.to_string());
    let broken = json!({ "name": "broken", "transport": "stdio", "command": "/nonexistent/x" });
    fx.json(&["add", "--json"], &broken.to_string());

    let mut cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_nativemem"));
    let child = fx.command(&["serve"]);
    cmd.args(child.get_args());
    for (k, v) in child.get_envs() {
        match v {
            Some(v) => cmd.env(k, v),
            None => cmd.env_remove(k),
        };
    }
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
        [
            "nativemem_status",
            "subagents_list",
            "fake__echo",
            "fake__env_matches"
        ]
    );

    let text = |r: rmcp::model::CallToolResult| {
        r.content
            .iter()
            .filter_map(|c| c.as_text().map(|t| t.text.clone()))
            .collect::<String>()
    };
    let echoed = client
        .peer()
        .call_tool(
            CallToolRequestParams::new("fake__echo".to_string())
                .with_arguments(json!({ "text": "ping" }).as_object().cloned().unwrap()),
        )
        .await
        .unwrap();
    assert_eq!(text(echoed), "ping");

    let matches = client
        .peer()
        .call_tool(
            CallToolRequestParams::new("fake__env_matches".to_string()).with_arguments(
                json!({ "name": "API_TOKEN", "value": value })
                    .as_object()
                    .cloned()
                    .unwrap(),
            ),
        )
        .await
        .unwrap();
    assert_eq!(text(matches), "true");

    let status = client
        .peer()
        .call_tool(CallToolRequestParams::new("nativemem_status".to_string()))
        .await
        .unwrap();
    let status = text(status);
    assert!(!status.contains(value), "{status}");
    let status: Value = serde_json::from_str(&status).unwrap();
    let state = |n: &str| {
        status["servers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["name"] == n)
            .unwrap()["status"]
            .clone()
    };
    assert_eq!(state("fake"), "connected");
    assert_eq!(state("broken"), "error");
    let _ = client.cancel().await;
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

#[test]
fn import_previews_then_applies_from_the_four_agents() {
    let fx = Fx::new();
    let r = &fx.root;
    write(
        &r.join("claude/.claude.json"),
        r#"{"mcpServers": {
            "shared": {"command": "npx", "args": ["pkg"], "env": {"API_KEY": "claude-key-1"}},
            "clash": {"command": "a"},
            "nativemem": {"command": "nativemem", "args": ["mcp", "serve"]}}}"#,
    );
    write(
        &r.join("codex/config.toml"),
        "[mcp_servers.shared]\ncommand = \"npx\"\nargs = [\"pkg\"]\nenv = { API_KEY = \"claude-key-1\" }\n\n[mcp_servers.clash]\ncommand = \"b\"\n",
    );
    write(
        &r.join("opencode/opencode.json"),
        r#"{"mcp": {"remote": {"type": "remote", "url": "https://x.example.com/mcp",
            "headers": {"Authorization": "Bearer oc-token-2"}}}}"#,
    );
    write(
        &r.join("pi/mcp.json"),
        r#"{"mcpServers": {"shared": {"command": "npx", "args": ["pkg"], "env": {"API_KEY": "claude-key-1"}}}}"#,
    );

    let preview = fx.run(&["import", "--json"], "");
    assert_eq!(preview.status.code(), Some(2), "conflicts need attention");
    let out = String::from_utf8_lossy(&preview.stdout).into_owned();
    let doc: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(doc["applied"], false);
    assert_eq!(
        doc["summary"],
        json!({ "add": 2, "present": 0, "conflicts": 1, "skipped": 0 })
    );
    let shared = doc["servers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "shared")
        .unwrap();
    assert_eq!(shared["agents"], json!(["claude", "codex", "pi"]));
    assert_eq!(shared["secrets"], json!(["shared-API_KEY"]));
    for value in ["claude-key-1", "oc-token-2"] {
        assert!(!out.contains(value), "{value} printed");
    }
    assert!(!fx.state().join("hub.toml").exists());
    assert!(!fx.secrets().exists());

    let text = fx.run(&["import"], "");
    let text = String::from_utf8_lossy(&text.stdout);
    assert!(text.contains("+ shared"), "{text}");
    assert!(text.contains("! clash"), "{text}");
    assert!(
        text.contains("2 to add, 0 already in the hub, 1 conflicts"),
        "{text}"
    );

    let applied = fx.run(&["import", "--apply", "--json"], "");
    assert_eq!(applied.status.code(), Some(2));
    let doc: Value = serde_json::from_slice(&applied.stdout).unwrap();
    assert_eq!(doc["applied"], true);
    let toml = fx.hub_toml();
    assert!(toml.contains("[servers.shared]"), "{toml}");
    assert!(toml.contains("secret:shared-API_KEY"), "{toml}");
    assert!(toml.contains("secret:remote-Authorization"), "{toml}");
    assert!(!toml.contains("clash"), "{toml}");
    for value in ["claude-key-1", "oc-token-2"] {
        assert!(!toml.contains(value), "{value} leaked into hub.toml");
    }
    let stored = fs::read_to_string(fx.secrets().join("secrets.json")).unwrap();
    assert!(stored.contains("claude-key-1") && stored.contains("Bearer oc-token-2"));

    let again: Value = serde_json::from_slice(&fx.run(&["import", "--json"], "").stdout).unwrap();
    assert_eq!(
        again["summary"],
        json!({ "add": 0, "present": 2, "conflicts": 1, "skipped": 0 })
    );
}

/// A browser that follows the authorization URL, redirects included.
#[cfg(unix)]
fn fake_browser(dir: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let script = dir.join("browser.sh");
    fs::write(&script, "#!/bin/sh\nexec curl -s -L -o /dev/null \"$1\"\n").unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    script
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn login_logout_list_and_test_work_through_the_cli() {
    let fake = fake_oauth::FakeOauth::start().await;
    let fx = Fx::new();
    let spec = json!({ "name": "remote", "transport": "http", "url": fake.mcp_url(),
        "auth": "oauth" });
    fx.json(&["add", "--json"], &spec.to_string());
    fx.json(&["add", "--json"], &fake_spec("local"));

    let auth_states = |doc: &Value| -> Value {
        let by_name = |n: &str| {
            doc["servers"]
                .as_array()
                .unwrap()
                .iter()
                .find(|s| s["name"] == n)
                .unwrap()["auth_state"]
                .clone()
        };
        json!([by_name("remote"), by_name("local")])
    };
    let (_, doc) = fx.json(&["list", "--json"], "");
    assert_eq!(auth_states(&doc), json!(["needs_login", "none"]));
    let (code, doc) = fx.json(&["test", "remote", "--json"], "");
    assert_eq!(code, 2);
    assert_eq!(doc["error"], "Sign in again: nativemem mcp login remote");

    let browser = fake_browser(&fx.root);
    let (code, doc) = fx.json(
        &[
            "login",
            "remote",
            "--browser",
            browser.to_str().unwrap(),
            "--json",
        ],
        "",
    );
    assert_eq!((code, &doc), (0, &json!({ "signed_in": "remote" })));
    assert_eq!(fake.registrations(), 1);
    let (_, doc) = fx.json(&["list", "--json"], "");
    assert_eq!(auth_states(&doc), json!(["signed_in", "none"]));
    let (code, doc) = fx.json(&["test", "remote", "--json"], "");
    assert_eq!(code, 0);
    assert_eq!(doc["tools"], json!(["whoami"]));

    // No token reaches the output or hub.toml.
    let listing = fx.run(&["list", "--json"], "");
    let shown = String::from_utf8_lossy(&listing.stdout).to_string() + &fx.hub_toml();
    for token in fake.bearers() {
        assert!(!shown.contains(&token), "a token leaked");
    }

    let (code, doc) = fx.json(&["logout", "remote", "--json"], "");
    assert_eq!(
        (code, &doc),
        (0, &json!({ "signed_out": "remote", "was_signed_in": true }))
    );
    let (_, doc) = fx.json(&["list", "--json"], "");
    assert_eq!(auth_states(&doc), json!(["needs_login", "none"]));
    let (_, doc) = fx.json(&["logout", "local", "--json"], "");
    assert_eq!(doc["code"], "invalid", "{doc}");
    let (_, doc) = fx.json(&["login", "local", "--json"], "");
    assert_eq!(doc["code"], "invalid", "{doc}");
}

/// The path agents are told to run: this test binary's `nativemem`, resolved through symlinks.
fn registered_command() -> String {
    fs::canonicalize(env!("CARGO_BIN_EXE_nativemem"))
        .unwrap()
        .display()
        .to_string()
}

#[test]
fn install_registers_the_hub_in_every_agent_status_checks_it_and_uninstall_removes_it() {
    let fx = Fx::new();
    let r = &fx.root;
    write(
        &r.join("pi/mcp.json"),
        r#"{"mcpServers":{"radius":{"url":"https://r"}}}"#,
    );
    write(&r.join("codex/config.toml"), "# mine\nmodel = \"gpt\"\n");

    let (code, doc) = fx.json(&["install", "--json"], "");
    assert_eq!(code, 0, "{doc}");
    let args: Vec<String> = [
        "mcp",
        "serve",
        "--claude-home",
        r.join("claude").to_str().unwrap(),
        "--codex-home",
        r.join("codex").to_str().unwrap(),
        "--opencode-home",
        r.join("opencode").to_str().unwrap(),
        "--pi-home",
        r.join("pi").to_str().unwrap(),
        "--state-dir",
        fx.state().to_str().unwrap(),
    ]
    .map(String::from)
    .to_vec();
    assert_eq!(doc["command"], registered_command());
    assert_eq!(doc["args"], json!(args));
    let agents = doc["agents"].as_array().unwrap();
    let statuses: Vec<(&str, &str)> = agents
        .iter()
        .map(|a| (a["agent"].as_str().unwrap(), a["status"].as_str().unwrap()))
        .collect();
    assert_eq!(
        statuses,
        [
            ("claude", "installed"),
            ("codex", "installed"),
            ("opencode", "installed"),
            ("pi", "installed")
        ]
    );
    assert_eq!(agents[3]["path"], r.join("pi/mcp.json").to_str().unwrap());

    // The files hold the entry in each agent's own shape.
    let pi: Value =
        serde_json::from_str(&fs::read_to_string(r.join("pi/mcp.json")).unwrap()).unwrap();
    assert_eq!(
        pi["mcpServers"]["nativemem"]["command"],
        registered_command()
    );
    assert_eq!(pi["mcpServers"]["nativemem"]["args"], json!(args));
    assert_eq!(pi["mcpServers"]["radius"]["url"], "https://r");
    let claude: Value =
        serde_json::from_str(&fs::read_to_string(r.join("claude/.claude.json")).unwrap()).unwrap();
    assert_eq!(claude["mcpServers"]["nativemem"]["type"], "stdio");
    let codex = fs::read_to_string(r.join("codex/config.toml")).unwrap();
    assert!(codex.starts_with("# mine\nmodel = \"gpt\"\n"), "{codex}");
    assert!(codex.contains("tool_timeout_sec = 1800"), "{codex}");
    let opencode: Value =
        serde_json::from_str(&fs::read_to_string(r.join("opencode/opencode.json")).unwrap())
            .unwrap();
    assert_eq!(opencode["mcp"]["nativemem"]["command"][1], "mcp");

    let (code, doc) = fx.json(&["status", "--json"], "");
    assert_eq!(code, 0);
    assert!(doc["agents"]
        .as_array()
        .unwrap()
        .iter()
        .all(|a| a["status"] == "registered"));

    // A second install changes nothing.
    let before = fs::read_to_string(r.join("codex/config.toml")).unwrap();
    let (_, doc) = fx.json(&["install", "--json"], "");
    assert!(doc["agents"]
        .as_array()
        .unwrap()
        .iter()
        .all(|a| a["status"] == "unchanged"));
    assert_eq!(
        fs::read_to_string(r.join("codex/config.toml")).unwrap(),
        before
    );

    // Without --json the result is one line per agent.
    let o = fx.run(&["install", "--agent", "pi"], "");
    assert!(String::from_utf8_lossy(&o.stdout).contains("pi  unchanged"));

    let (code, doc) = fx.json(&["uninstall", "--json"], "");
    assert_eq!(code, 0);
    assert!(doc["agents"]
        .as_array()
        .unwrap()
        .iter()
        .all(|a| a["status"] == "removed"));
    let pi: Value =
        serde_json::from_str(&fs::read_to_string(r.join("pi/mcp.json")).unwrap()).unwrap();
    assert_eq!(pi, json!({"mcpServers": {"radius": {"url": "https://r"}}}));
    assert_eq!(
        fs::read_to_string(r.join("codex/config.toml")).unwrap(),
        "# mine\nmodel = \"gpt\"\n"
    );
    let (_, doc) = fx.json(&["status", "--json"], "");
    assert!(doc["agents"]
        .as_array()
        .unwrap()
        .iter()
        .all(|a| a["status"] == "missing"));

    // The write commands took the state lock and backed up what they replaced.
    assert!(fx.state().join("sync.lock").is_file());
    assert!(fs::read_dir(fx.state().join("backups"))
        .unwrap()
        .next()
        .is_some());
}

#[test]
fn install_without_explicit_roots_registers_the_bare_command() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("home");
    fs::create_dir_all(home.join(".pi/agent")).unwrap();
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
        .env("HOME", &home)
        .args(["mcp", "install", "--agent", "pi", "--json"]);
    let out = c.output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(doc["args"], json!(["mcp", "serve"]));
    let pi: Value =
        serde_json::from_str(&fs::read_to_string(home.join(".pi/agent/mcp.json")).unwrap())
            .unwrap();
    assert_eq!(
        pi["mcpServers"]["nativemem"]["args"],
        json!(["mcp", "serve"])
    );
    // The other agents have no home here and are skipped without being created.
    assert!(!home.join(".claude").exists() && !home.join(".codex").exists());
}

#[test]
fn install_reports_a_file_that_does_not_parse_and_still_installs_the_rest() {
    let fx = Fx::new();
    let r = &fx.root;
    write(&r.join("codex/config.toml"), "[broken\n");
    let (code, doc) = fx.json(&["install", "--json"], "");
    assert_eq!(code, 2, "{doc}");
    let by = |agent: &str| {
        doc["agents"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["agent"] == agent)
            .cloned()
            .unwrap()
    };
    assert_eq!(by("codex")["status"], "error");
    assert_eq!(by("codex")["code"], "parse");
    assert_eq!(by("pi")["status"], "installed");
    assert_eq!(
        fs::read_to_string(r.join("codex/config.toml")).unwrap(),
        "[broken\n"
    );

    let (code, doc) = fx.json(&["install", "--agent", "hermes", "--json"], "");
    assert_eq!(code, 1);
    assert_eq!(doc["code"], "unknown_agent", "{doc}");
}
