//! Import from four fake agent homes in a temp directory.

use std::fs;
use std::path::Path;

use nativemem_core::fsx::BackupSession;
use nativemem_hub::config::{Auth, HubConfig, Transport};
use nativemem_hub::import::{apply, plan, AgentRoots, ImportPlan, Status};
use nativemem_hub::secrets::{FileStore, SecretStore};

struct Homes {
    _t: tempfile::TempDir,
    claude: std::path::PathBuf,
    codex: std::path::PathBuf,
    opencode: std::path::PathBuf,
    pi: std::path::PathBuf,
    secrets: std::path::PathBuf,
    state: std::path::PathBuf,
}

impl Homes {
    fn roots(&self) -> AgentRoots<'_> {
        AgentRoots {
            claude: &self.claude,
            codex: &self.codex,
            opencode: &self.opencode,
            pi: &self.pi,
        }
    }
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn homes() -> Homes {
    let t = tempfile::tempdir().unwrap();
    let r = t.path().to_path_buf();
    let h = Homes {
        claude: r.join("claude"),
        codex: r.join("codex"),
        opencode: r.join("opencode"),
        pi: r.join("pi"),
        secrets: r.join("secrets"),
        state: r.join("state"),
        _t: t,
    };
    write(
        &h.claude.join(".claude.json"),
        r#"{
          "numStartups": 3,
          "projects": {"/w/app": {"mcpServers": {"project-only": {"command": "p"}}}},
          "mcpServers": {
            "shared": {"type": "stdio", "command": "npx", "args": ["-y", "pkg"], "env": {"API_KEY": "key-one"}},
            "clash": {"command": "claude-cmd"},
            "remote": {"type": "http", "url": "https://mcp.example.com/mcp",
                       "headers": {"Authorization": "Bearer claude-token"}},
            "sse-server": {"type": "sse", "url": "https://old.example.com/sse"},
            "nativemem": {"command": "nativemem", "args": ["mcp", "serve"]}
          }
        }"#,
    );
    write(
        &h.codex.join("config.toml"),
        r#"
model = "x"
[mcp_servers.shared]
command = "npx"
args = ["-y", "pkg"]
env = { API_KEY = "key-one" }
startup_timeout_sec = 30

[mcp_servers.clash]
command = "codex-cmd"

[mcp_servers.fromcodex]
url = "https://codex.example.com/mcp"
http_headers = { "X-Team" = "blue" }
bearer_token_env_var = "CODEX_TOKEN"
"#,
    );
    write(
        &h.opencode.join("opencode.jsonc"),
        r#"{
          // comments and trailing commas are fine here
          "mcp": {
            "oc": {"type": "local", "command": ["node", "srv.js", "--x"],
                   "environment": {"FROM_ENV": "${SOME_VAR}"}, "enabled": false,},
            "shared": {"type": "local", "command": ["npx", "-y", "pkg"], "environment": {"API_KEY": "key-one"}},
            "oauthed": {"type": "remote", "url": "https://o.example.com/mcp", "oauth": {}},
          },
        }"#,
    );
    write(
        &h.pi.join("mcp.json"),
        r#"{"mcpServers": {
            "shared": {"command": "npx", "args": ["-y", "pkg"], "env": {"API_KEY": "key-one"}},
            "pionly": {"command": "/bin/pi-srv", "cwd": "/work", "exposure": "direct"}
        }}"#,
    );
    h
}

fn server<'a>(plan: &'a ImportPlan, name: &str) -> &'a nativemem_hub::import::PlannedServer {
    plan.servers
        .iter()
        .find(|s| s.name == name)
        .unwrap_or_else(|| panic!("no planned server {name}"))
}

#[test]
fn identical_entries_merge_and_different_ones_conflict() {
    let h = homes();
    let p = plan(&h.roots(), &HubConfig::default());

    let shared = server(&p, "shared");
    assert_eq!(shared.status, Status::Add);
    assert_eq!(shared.agents, ["claude", "codex", "opencode", "pi"]);
    assert_eq!(shared.spec.transport, Transport::Stdio);
    assert_eq!(shared.spec.command.as_deref(), Some("npx"));
    assert_eq!(shared.spec.args, ["-y", "pkg"]);

    let clash = server(&p, "clash");
    assert_eq!(clash.status, Status::Conflict);
    assert_eq!(clash.agents, ["claude", "codex"]);

    // Agent-specific shapes become hub entries.
    let oc = server(&p, "oc");
    assert_eq!(oc.spec.command.as_deref(), Some("node"));
    assert_eq!(oc.spec.args, ["srv.js", "--x"]);
    assert!(!oc.spec.enabled);
    assert_eq!(server(&p, "oauthed").spec.auth, Auth::Oauth);
    assert_eq!(server(&p, "pionly").spec.cwd.as_deref(), Some("/work"));
    assert_eq!(server(&p, "fromcodex").spec.transport, Transport::Http);

    // The hub itself, SSE entries and project-scoped entries are left out.
    assert!(p.servers.iter().all(|s| s.name != "nativemem"));
    assert!(p.servers.iter().all(|s| s.name != "project-only"));
    assert!(p.servers.iter().all(|s| s.name != "sse-server"));
    assert!(p
        .skipped
        .iter()
        .any(|s| s.name == "sse-server" && s.agent == "claude"));
    // Keys the hub cannot carry are named, never their values.
    let warnings = p.warnings.join("\n");
    assert!(warnings.contains("startup_timeout_sec"), "{warnings}");
    assert!(warnings.contains("bearer_token_env_var"), "{warnings}");
    assert!(!warnings.contains("key-one"), "{warnings}");
}

#[test]
fn literal_values_move_to_the_secret_store_and_the_dry_run_writes_nothing() {
    let h = homes();
    let store = FileStore::new(&h.secrets);
    let mut config = HubConfig::default();
    let p = plan(&h.roots(), &config);

    // A dry run only plans.
    assert!(!h.secrets.exists());
    assert!(!h.state.exists());

    let shared = server(&p, "shared");
    assert_eq!(shared.spec.env["API_KEY"], "secret:shared-API_KEY");
    let remote = server(&p, "remote");
    assert_eq!(
        remote.spec.headers["Authorization"],
        "secret:remote-Authorization"
    );
    // An environment reference cannot be evaluated by the hub: the secret is created unset.
    assert_eq!(server(&p, "oc").spec.env["FROM_ENV"], "secret:oc-FROM_ENV");
    assert!(p.warnings.iter().any(|w| w.contains("oc-FROM_ENV")));

    let added = apply(&p, &mut config, &store).unwrap();
    assert_eq!(
        added,
        p.servers.iter().filter(|s| s.status == Status::Add).count()
    );
    assert_eq!(
        store.get("shared-API_KEY").unwrap().as_deref(),
        Some("key-one")
    );
    assert_eq!(
        store.get("remote-Authorization").unwrap().as_deref(),
        Some("Bearer claude-token")
    );
    assert_eq!(
        store.get("fromcodex-X-Team").unwrap().as_deref(),
        Some("blue")
    );
    assert_eq!(store.get("oc-FROM_ENV").unwrap(), None);
    assert!(!config.servers.contains_key("clash"));

    config
        .save(&h.state, &BackupSession::with_stamp("s"))
        .unwrap();
    let saved = fs::read_to_string(h.state.join("hub.toml")).unwrap();
    for value in ["key-one", "claude-token", "blue", "${SOME_VAR}"] {
        assert!(!saved.contains(value), "{value} leaked into hub.toml");
    }
    assert!(saved.contains("secret:shared-API_KEY"), "{saved}");
}

#[test]
fn a_second_import_finds_everything_present_and_a_changed_hub_entry_conflicts() {
    let h = homes();
    let store = FileStore::new(&h.secrets);
    let mut config = HubConfig::default();
    apply(&plan(&h.roots(), &config), &mut config, &store).unwrap();

    let again = plan(&h.roots(), &config);
    assert_eq!(server(&again, "shared").status, Status::Present);
    assert_eq!(server(&again, "clash").status, Status::Conflict);
    assert_eq!(apply(&again, &mut config, &store).unwrap(), 0);

    config.servers.get_mut("pionly").unwrap().args = vec!["--changed".into()];
    let changed = plan(&h.roots(), &config);
    let pionly = server(&changed, "pionly");
    assert_eq!(pionly.status, Status::Conflict);
    assert_eq!(pionly.agents, ["pi", "hub"]);
}
