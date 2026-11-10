//! Registering the hub in the four agents, over fixture homes in temp directories. No real agent
//! home is read or written.

use std::fs;
use std::path::PathBuf;

use nativemem_core::fsx::BackupSession;
use nativemem_hub::import::AgentRoots;
use nativemem_hub::register::{self, AgentOutcome, Backups, Entry};
use serde_json::{json, Value};

struct Fx {
    _t: tempfile::TempDir,
    root: PathBuf,
}

impl Fx {
    fn new() -> Fx {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().to_path_buf();
        for d in ["claude", "codex", "opencode", "pi"] {
            fs::create_dir_all(root.join(d)).unwrap();
        }
        Fx { _t: t, root }
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    fn write(&self, rel: &str, text: &str) {
        let path = self.path(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn read(&self, rel: &str) -> String {
        fs::read_to_string(self.path(rel)).unwrap()
    }

    fn state(&self) -> PathBuf {
        self.root.join("state")
    }

    fn roots<'a>(&'a self, paths: &'a [PathBuf; 4]) -> AgentRoots<'a> {
        AgentRoots {
            claude: &paths[0],
            codex: &paths[1],
            opencode: &paths[2],
            pi: &paths[3],
        }
    }

    fn homes(&self) -> [PathBuf; 4] {
        ["claude", "codex", "opencode", "pi"].map(|d| self.path(d))
    }

    /// Run `f` with the roots and a backup session stamped `stamp`.
    fn with<T>(&self, stamp: &str, f: impl FnOnce(&AgentRoots, &Backups) -> T) -> T {
        let homes = self.homes();
        let state = self.state();
        let session = BackupSession::with_stamp(stamp);
        f(
            &self.roots(&homes),
            &Backups {
                state: &state,
                session: &session,
            },
        )
    }

    fn install(&self, stamp: &str, only: Option<&str>) -> Vec<AgentOutcome> {
        self.with(stamp, |r, b| {
            register::install(r, only, &entry(), b).unwrap()
        })
    }

    fn uninstall(&self, stamp: &str, only: Option<&str>) -> Vec<AgentOutcome> {
        self.with(stamp, |r, b| register::uninstall(r, only, b).unwrap())
    }

    fn status(&self, only: Option<&str>) -> Vec<AgentOutcome> {
        self.with("status", |r, _| {
            register::status(r, only, &entry()).unwrap()
        })
    }

    fn backed_up(&self, stamp: &str) -> bool {
        self.state().join("backups").join(stamp).exists()
    }
}

fn entry() -> Entry {
    Entry {
        command: "/opt/nativemem/bin/nativemem".into(),
        args: vec!["mcp".into(), "serve".into()],
    }
}

fn statuses(outcomes: &[AgentOutcome]) -> Vec<(&'static str, &'static str)> {
    outcomes.iter().map(|o| (o.agent, o.status)).collect()
}

fn only(outcomes: Vec<AgentOutcome>) -> AgentOutcome {
    assert_eq!(outcomes.len(), 1);
    outcomes.into_iter().next().unwrap()
}

const CLAUDE_FILE: &str = "{\n  \"numStartups\": 412,\n  \"ratio\": 0.5,\n  \"note\": \"çay ☕ \\\"quoted\\\"\",\n  \"projects\": {\n    \"/Users/x/app\": {\n      \"allowedTools\": [],\n      \"mcpServers\": {}\n    }\n  },\n  \"mcpServers\": {\n    \"other\": {\n      \"type\": \"stdio\",\n      \"command\": \"tool\",\n      \"args\": [\n        \"-x\"\n      ],\n      \"env\": {\n        \"K\": \"v\"\n      }\n    }\n  }\n}\n";

#[test]
fn claude_gets_the_entry_and_everything_else_stays_byte_for_byte() {
    let fx = Fx::new();
    fx.write("claude/.claude.json", CLAUDE_FILE);

    let done = only(fx.install("one", Some("claude")));
    assert_eq!(done.status, "installed");
    assert_eq!(done.path, Some(fx.path("claude/.claude.json")));
    let doc: Value = serde_json::from_str(&fx.read("claude/.claude.json")).unwrap();
    assert_eq!(
        doc["mcpServers"]["nativemem"],
        json!({
            "type": "stdio",
            "command": "/opt/nativemem/bin/nativemem",
            "args": ["mcp", "serve"],
            "env": {}
        })
    );
    assert_eq!(
        doc["mcpServers"]["other"]["env"]["K"], "v",
        "other servers stay"
    );
    // The previous file is in the backup tree.
    assert_eq!(
        fs::read_to_string(fx.state().join("backups/one/claude/.claude.json")).unwrap(),
        CLAUDE_FILE
    );

    // A second install changes nothing and backs nothing up.
    let installed = fx.read("claude/.claude.json");
    assert_eq!(only(fx.install("two", Some("claude"))).status, "unchanged");
    assert_eq!(fx.read("claude/.claude.json"), installed);
    assert!(!fx.backed_up("two"));

    // Uninstall removes only that entry: the file is the original again.
    assert_eq!(
        only(fx.uninstall("three", Some("claude"))).status,
        "removed"
    );
    assert_eq!(fx.read("claude/.claude.json"), CLAUDE_FILE);
    assert_eq!(only(fx.uninstall("four", Some("claude"))).status, "absent");
    assert!(!fx.backed_up("four"));
}

#[test]
fn claude_config_sits_next_to_a_home_named_dot_claude() {
    let fx = Fx::new();
    fs::create_dir_all(fx.path("home/.claude")).unwrap();
    fx.write("home/.claude.json", "{\n  \"a\": 1\n}\n");
    let homes = [
        fx.path("home/.claude"),
        fx.path("codex"),
        fx.path("opencode"),
        fx.path("pi"),
    ];
    let state = fx.state();
    let session = BackupSession::with_stamp("s");
    let done = only(
        register::install(
            &fx.roots(&homes),
            Some("claude"),
            &entry(),
            &Backups {
                state: &state,
                session: &session,
            },
        )
        .unwrap(),
    );
    assert_eq!(done.path, Some(fx.path("home/.claude.json")));
    assert!(fx.read("home/.claude.json").contains("nativemem"));
    assert!(!fx.path("home/.claude/.claude.json").exists());
}

const CODEX_FILE: &str = "# Codex settings\nmodel = \"gpt-5\"  # the default\n\n[projects.\"/Users/x/app\"]\ntrust_level = \"trusted\"\n\n# servers I added by hand\n[mcp_servers.docs]\nurl = \"https://docs.example.com/mcp\"\n";

#[test]
fn codex_gets_a_table_and_every_other_byte_stays() {
    let fx = Fx::new();
    fx.write("codex/config.toml", CODEX_FILE);

    assert_eq!(only(fx.install("one", Some("codex"))).status, "installed");
    let expected = format!(
        "{CODEX_FILE}\n[mcp_servers.nativemem]\ncommand = \"/opt/nativemem/bin/nativemem\"\nargs = [\"mcp\", \"serve\"]\ntool_timeout_sec = 1800\n"
    );
    assert_eq!(fx.read("codex/config.toml"), expected);
    assert_eq!(
        fs::read_to_string(fx.state().join("backups/one/codex/config.toml")).unwrap(),
        CODEX_FILE
    );

    assert_eq!(only(fx.install("two", Some("codex"))).status, "unchanged");
    assert!(!fx.backed_up("two"));
    assert_eq!(fx.read("codex/config.toml"), expected);

    assert_eq!(only(fx.uninstall("three", Some("codex"))).status, "removed");
    assert_eq!(fx.read("codex/config.toml"), CODEX_FILE);
}

#[test]
fn codex_keeps_a_timeout_the_user_set_and_fixes_a_stale_command() {
    let fx = Fx::new();
    fx.write(
        "codex/config.toml",
        "[mcp_servers.nativemem]\ncommand = \"/old/nativemem\"  # moved\nargs = [\"mcp\", \"serve\"]\ntool_timeout_sec = 60\nstartup_timeout_sec = 30\n",
    );
    assert_eq!(only(fx.status(Some("codex"))).status, "outdated");
    assert_eq!(only(fx.install("one", Some("codex"))).status, "installed");
    let text = fx.read("codex/config.toml");
    assert!(
        text.contains("command = \"/opt/nativemem/bin/nativemem\""),
        "{text}"
    );
    assert!(text.contains("tool_timeout_sec = 60\n"), "{text}");
    assert!(text.contains("startup_timeout_sec = 30\n"), "{text}");
    assert_eq!(only(fx.status(Some("codex"))).status, "registered");
}

#[test]
fn codex_creates_a_missing_config() {
    let fx = Fx::new();
    assert_eq!(only(fx.status(Some("codex"))).status, "missing");
    assert_eq!(only(fx.install("one", Some("codex"))).status, "installed");
    assert_eq!(
        fx.read("codex/config.toml"),
        "[mcp_servers.nativemem]\ncommand = \"/opt/nativemem/bin/nativemem\"\nargs = [\"mcp\", \"serve\"]\ntool_timeout_sec = 1800\n"
    );
    assert!(!fx.backed_up("one"), "there was nothing to back up");
}

const OPENCODE_FILE: &str = "{\n  // appearance\n  \"theme\": \"dark\", // keep\n  \"mcp\": {\n    /* my servers */\n    \"docs\": { \"type\": \"remote\", \"url\": \"https://docs.example.com/mcp\" },\n  },\n  \"instructions\": [\"~/notes.md\"]\n}\n";

#[test]
fn opencode_gets_the_entry_with_comments_and_formatting_intact() {
    let fx = Fx::new();
    fx.write("opencode/opencode.jsonc", OPENCODE_FILE);

    let done = only(fx.install("one", Some("opencode")));
    assert_eq!(done.status, "installed");
    assert_eq!(done.path, Some(fx.path("opencode/opencode.jsonc")));
    let text = fx.read("opencode/opencode.jsonc");
    for kept in [
        "  // appearance\n",
        "  \"theme\": \"dark\", // keep\n",
        "    /* my servers */\n",
        "    \"docs\": { \"type\": \"remote\", \"url\": \"https://docs.example.com/mcp\" },\n",
        "  \"instructions\": [\"~/notes.md\"]\n}\n",
    ] {
        assert!(text.contains(kept), "{kept:?} missing from:\n{text}");
    }
    let doc: Value = serde_json::from_str(&strip(&text)).unwrap();
    assert_eq!(
        doc["mcp"]["nativemem"],
        json!({
            "type": "local",
            "command": ["/opt/nativemem/bin/nativemem", "mcp", "serve"],
            "enabled": true
        })
    );
    assert_eq!(
        fs::read_to_string(fx.state().join("backups/one/opencode/opencode.jsonc")).unwrap(),
        OPENCODE_FILE
    );

    assert_eq!(
        only(fx.install("two", Some("opencode"))).status,
        "unchanged"
    );
    assert!(!fx.backed_up("two"));
    assert_eq!(fx.read("opencode/opencode.jsonc"), text);

    assert_eq!(
        only(fx.uninstall("three", Some("opencode"))).status,
        "removed"
    );
    let after = fx.read("opencode/opencode.jsonc");
    assert!(!after.contains("nativemem"), "{after}");
    for kept in ["// appearance", "// keep", "/* my servers */", "\"docs\""] {
        assert!(after.contains(kept), "{kept:?} missing from:\n{after}");
    }
}

/// Comments and trailing commas out, so serde_json can read what the test wrote.
fn strip(text: &str) -> String {
    let root = jsonc_parser::cst::CstRootNode::parse(text, &Default::default()).unwrap();
    root.value().unwrap().to_serde_value().unwrap().to_string()
}

#[test]
fn opencode_follows_the_servers_layout_and_the_file_that_exists() {
    let fx = Fx::new();
    fx.write(
        "opencode/opencode.json",
        "{\n  \"mcp\": {\n    \"servers\": {\n      \"docs\": { \"type\": \"remote\", \"url\": \"https://d\" }\n    }\n  }\n}\n",
    );
    let done = only(fx.install("one", Some("opencode")));
    assert_eq!(done.path, Some(fx.path("opencode/opencode.json")));
    let doc: Value = serde_json::from_str(&fx.read("opencode/opencode.json")).unwrap();
    assert!(doc["mcp"]["servers"]["nativemem"].is_object());
    assert!(doc["mcp"].get("nativemem").is_none());
    assert_eq!(
        only(fx.install("two", Some("opencode"))).status,
        "unchanged"
    );
    assert_eq!(only(fx.status(Some("opencode"))).status, "registered");
    assert_eq!(
        only(fx.uninstall("three", Some("opencode"))).status,
        "removed"
    );
    assert!(!fx.read("opencode/opencode.json").contains("nativemem"));
}

#[test]
fn opencode_creates_opencode_json_and_uses_jsonc_when_that_is_the_file_present() {
    let fx = Fx::new();
    let done = only(fx.install("one", Some("opencode")));
    assert_eq!(done.path, Some(fx.path("opencode/opencode.json")));
    let doc: Value = serde_json::from_str(&fx.read("opencode/opencode.json")).unwrap();
    assert_eq!(doc["mcp"]["nativemem"]["type"], "local");
    assert!(!fx.backed_up("one"));

    let fx = Fx::new();
    fx.write("opencode/opencode.jsonc", "{\n  // hi\n}\n");
    let done = only(fx.install("one", Some("opencode")));
    assert_eq!(done.path, Some(fx.path("opencode/opencode.jsonc")));
    assert!(!fx.path("opencode/opencode.json").exists());
    assert!(fx.read("opencode/opencode.jsonc").contains("// hi"));
}

#[test]
fn pi_gets_the_entry_in_mcp_json() {
    let fx = Fx::new();
    let done = only(fx.install("one", Some("pi")));
    assert_eq!(done.path, Some(fx.path("pi/mcp.json")));
    let doc: Value = serde_json::from_str(&fx.read("pi/mcp.json")).unwrap();
    assert_eq!(
        doc,
        json!({ "mcpServers": { "nativemem": {
            "command": "/opt/nativemem/bin/nativemem", "args": ["mcp", "serve"] } } })
    );

    let existing = "{\n    \"mcpServers\": {\n        \"radius\": {\n            \"url\": \"https://r\"\n        }\n    }\n}\n";
    fx.write("pi/mcp.json", existing);
    assert_eq!(only(fx.install("two", Some("pi"))).status, "installed");
    let text = fx.read("pi/mcp.json");
    assert!(
        text.starts_with(
            "{\n    \"mcpServers\": {\n        \"radius\": {\n            \"url\": \"https://r\"\n        },\n        \"nativemem\""
        ),
        "{text}"
    );
    assert_eq!(only(fx.install("three", Some("pi"))).status, "unchanged");
    assert_eq!(only(fx.uninstall("four", Some("pi"))).status, "removed");
    assert_eq!(fx.read("pi/mcp.json"), existing);
}

#[test]
fn pi_warns_when_an_extension_replaces_the_built_in_mcp_support() {
    let fx = Fx::new();
    assert!(only(fx.install("one", Some("pi"))).warnings.is_empty());
    fx.write(
        "pi/extensions/other.ts",
        "pi.registerCommand(\"deploy\", { handler() {} });\n",
    );
    assert!(only(fx.status(Some("pi"))).warnings.is_empty());
    fx.write(
        "pi/extensions/mcp-bridge/index.ts",
        "export default (pi) => { pi.registerCommand(\"mcp\", { handler() {} }); };\n",
    );
    let outcome = only(fx.install("two", Some("pi")));
    assert_eq!(outcome.warnings.len(), 1);
    assert!(
        outcome.warnings[0].contains("registers /mcp"),
        "{outcome:?}"
    );
    assert!(outcome.warnings[0].contains("mcp-bridge"), "{outcome:?}");
    assert_eq!(only(fx.status(Some("pi"))).warnings.len(), 1);
}

#[test]
fn a_file_that_does_not_parse_is_never_written() {
    let fx = Fx::new();
    let broken = [
        ("claude/.claude.json", "{ \"mcpServers\": ", "claude"),
        ("codex/config.toml", "[mcp_servers\nbroken", "codex"),
        ("opencode/opencode.json", "{ \"mcp\": [1, 2", "opencode"),
        ("pi/mcp.json", "not json at all", "pi"),
    ];
    for (file, text, _) in &broken {
        fx.write(file, text);
    }
    let outcomes = fx.install("one", None);
    for (outcome, (file, text, agent)) in outcomes.iter().zip(&broken) {
        assert_eq!(outcome.agent, *agent);
        assert_eq!(outcome.status, "error", "{outcome:?}");
        let (code, message) = outcome.error.as_ref().unwrap();
        assert_eq!(*code, "parse", "{outcome:?}");
        assert!(message.contains(file), "{message}");
        assert_eq!(&fx.read(file), text, "{file} must stay as it was");
    }
    assert!(!fx.backed_up("one"));
    let removed = fx.uninstall("two", None);
    assert!(removed.iter().all(|o| o.status == "error"), "{removed:?}");
    let checked = fx.status(None);
    assert!(checked.iter().all(|o| o.status == "error"), "{checked:?}");
}

#[test]
fn a_structure_that_is_not_an_object_is_a_parse_error_too() {
    let fx = Fx::new();
    fx.write("pi/mcp.json", "{\"mcpServers\": [1]}");
    fx.write("codex/config.toml", "mcp_servers = 3\n");
    let outcomes = fx.install("one", None);
    let by = |agent: &str| outcomes.iter().find(|o| o.agent == agent).unwrap();
    for agent in ["pi", "codex"] {
        assert_eq!(by(agent).error.as_ref().unwrap().0, "parse", "{agent}");
    }
    assert_eq!(fx.read("pi/mcp.json"), "{\"mcpServers\": [1]}");
    assert_eq!(fx.read("codex/config.toml"), "mcp_servers = 3\n");
}

#[test]
fn a_missing_agent_home_is_skipped_and_the_rest_installs() {
    let fx = Fx::new();
    fs::remove_dir_all(fx.path("codex")).unwrap();
    fs::remove_dir_all(fx.path("pi")).unwrap();
    let outcomes = fx.install("one", None);
    assert_eq!(
        statuses(&outcomes),
        [
            ("claude", "installed"),
            ("codex", "not_installed"),
            ("opencode", "installed"),
            ("pi", "not_installed"),
        ]
    );
    assert!(!fx.path("codex").exists(), "a missing home is not created");
    assert!(!fx.path("pi").exists());
    assert_eq!(
        statuses(&fx.status(None)),
        [
            ("claude", "registered"),
            ("codex", "not_installed"),
            ("opencode", "registered"),
            ("pi", "not_installed"),
        ]
    );
}

#[test]
fn install_all_then_uninstall_all_and_an_unknown_agent_is_refused() {
    let fx = Fx::new();
    assert_eq!(
        statuses(&fx.install("one", None)),
        [
            ("claude", "installed"),
            ("codex", "installed"),
            ("opencode", "installed"),
            ("pi", "installed"),
        ]
    );
    assert_eq!(
        statuses(&fx.install("two", None)),
        [
            ("claude", "unchanged"),
            ("codex", "unchanged"),
            ("opencode", "unchanged"),
            ("pi", "unchanged"),
        ]
    );
    assert!(!fx.backed_up("two"));
    assert_eq!(
        statuses(&fx.uninstall("three", None)),
        [
            ("claude", "removed"),
            ("codex", "removed"),
            ("opencode", "removed"),
            ("pi", "removed"),
        ]
    );
    assert_eq!(
        statuses(&fx.status(None)),
        [
            ("claude", "missing"),
            ("codex", "missing"),
            ("opencode", "missing"),
            ("pi", "missing"),
        ]
    );
    let error = fx.with("x", |r, b| {
        register::install(r, Some("hermes"), &entry(), b).unwrap_err()
    });
    assert_eq!(error.code(), "invalid");
}

#[test]
fn a_changed_binary_path_is_an_outdated_entry_that_install_corrects() {
    let fx = Fx::new();
    fx.install("one", None);
    let moved = Entry {
        command: "/elsewhere/nativemem".into(),
        args: vec![
            "mcp".into(),
            "serve".into(),
            "--state-dir".into(),
            "/s".into(),
        ],
    };
    let outdated = fx.with("s", |r, _| register::status(r, None, &moved).unwrap());
    assert!(
        outdated.iter().all(|o| o.status == "outdated"),
        "{outdated:?}"
    );
    let installed = fx.with("two", |r, b| register::install(r, None, &moved, b).unwrap());
    assert!(
        installed.iter().all(|o| o.status == "installed"),
        "{installed:?}"
    );
    let current = fx.with("s", |r, _| register::status(r, None, &moved).unwrap());
    assert!(
        current.iter().all(|o| o.status == "registered"),
        "{current:?}"
    );
    let text = fx.read("codex/config.toml");
    assert!(
        text.contains("args = [\"mcp\", \"serve\", \"--state-dir\", \"/s\"]"),
        "{text}"
    );
}
