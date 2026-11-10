//! `nativemem mcp`: the hub's server, its configuration, its secrets and the import of the
//! agents' own MCP entries.

use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;

use clap::Subcommand;
use nativemem_core::error::Error;
use nativemem_core::fsx::BackupSession;
use nativemem_hub::config::parse_spec_json;
use nativemem_hub::connect::test_server;
use nativemem_hub::import::{self, AgentRoots, Status};
use nativemem_hub::secrets::validate_secret_id;
use nativemem_hub::{FileStore, HubConfig, HubError, KeychainStore, SecretStore, ServerSpec};
use serde_json::{json, Value};

use crate::{print_json, Failure, Outcome, Roots, EXIT_ATTENTION};

#[derive(clap::Args)]
pub struct McpArgs {
    /// Keep secrets in files of this directory instead of the OS keychain (tests)
    #[arg(long, global = true, hide = true)]
    secrets_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: McpCommand,
}

#[derive(Subcommand)]
pub enum McpCommand {
    /// Run the hub: a stdio MCP server that exposes the tools of the configured servers
    Serve,
    /// List the configured servers
    List {
        /// Print one JSON document
        #[arg(long)]
        json: bool,
    },
    /// Add a server from a JSON document on stdin (`name` plus the fields of `hub.toml`)
    Add {
        /// Print one JSON document
        #[arg(long)]
        json: bool,
    },
    /// Replace a server's configuration with a JSON document on stdin
    Edit {
        name: String,
        /// Print one JSON document
        #[arg(long)]
        json: bool,
    },
    /// Remove a server; its secrets are left alone
    Remove {
        name: String,
        /// Print one JSON document
        #[arg(long)]
        json: bool,
    },
    /// Let the hub connect to a server
    Enable {
        name: String,
        /// Print one JSON document
        #[arg(long)]
        json: bool,
    },
    /// Stop the hub from connecting to a server
    Disable {
        name: String,
        /// Print one JSON document
        #[arg(long)]
        json: bool,
    },
    /// Store a secret; the value is read from stdin
    SetSecret {
        id: String,
        /// Print one JSON document
        #[arg(long)]
        json: bool,
    },
    /// Delete a stored secret
    DeleteSecret {
        id: String,
        /// Print one JSON document
        #[arg(long)]
        json: bool,
    },
    /// Connect to a server and list its tools
    Test {
        name: String,
        /// Print one JSON document
        #[arg(long)]
        json: bool,
    },
    /// Read the agents' MCP servers; with --apply, add them to the hub
    Import {
        #[arg(long)]
        apply: bool,
        /// Print one JSON document
        #[arg(long)]
        json: bool,
    },
}

impl McpArgs {
    /// Commands that change `hub.toml` or the secret store run under the state lock.
    pub fn writes(&self) -> bool {
        match &self.command {
            McpCommand::Serve
            | McpCommand::List { .. }
            | McpCommand::Test { .. }
            | McpCommand::Import { apply: false, .. } => false,
            McpCommand::Add { .. }
            | McpCommand::Edit { .. }
            | McpCommand::Remove { .. }
            | McpCommand::Enable { .. }
            | McpCommand::Disable { .. }
            | McpCommand::SetSecret { .. }
            | McpCommand::DeleteSecret { .. }
            | McpCommand::Import { apply: true, .. } => true,
        }
    }

    pub fn json(&self) -> bool {
        match &self.command {
            McpCommand::Serve => false,
            McpCommand::List { json }
            | McpCommand::Add { json }
            | McpCommand::Edit { json, .. }
            | McpCommand::Remove { json, .. }
            | McpCommand::Enable { json, .. }
            | McpCommand::Disable { json, .. }
            | McpCommand::SetSecret { json, .. }
            | McpCommand::DeleteSecret { json, .. }
            | McpCommand::Test { json, .. }
            | McpCommand::Import { json, .. } => *json,
        }
    }
}

impl From<HubError> for Failure {
    fn from(e: HubError) -> Self {
        Failure::Coded {
            code: e.code(),
            message: e.to_string(),
        }
    }
}

fn read_stdin() -> Outcome<String> {
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .map_err(Error::from)?;
    Ok(input)
}

/// Shown instead of a literal value: a literal env or header value may be a secret.
const LITERAL: &str = "<literal>";

fn masked(values: &std::collections::BTreeMap<String, String>) -> Value {
    values
        .iter()
        .map(|(k, v)| {
            let shown = if v.starts_with("secret:") { v } else { LITERAL };
            (k.clone(), Value::String(shown.to_string()))
        })
        .collect::<serde_json::Map<_, _>>()
        .into()
}

fn server_json(name: &str, spec: &ServerSpec) -> Value {
    let mut doc = json!({
        "name": name,
        "transport": spec.transport.as_str(),
        "enabled": spec.enabled,
        "auth": spec.auth.as_str(),
        "args": spec.args,
        "env": masked(&spec.env),
        "headers": masked(&spec.headers),
        "secrets": spec.secret_ids(),
    });
    if let Some(command) = &spec.command {
        doc["command"] = json!(command);
    }
    if let Some(cwd) = &spec.cwd {
        doc["cwd"] = json!(cwd);
    }
    if let Some(url) = &spec.url {
        doc["url"] = json!(url);
    }
    doc
}

fn list(config: &HubConfig, json: bool) {
    if json {
        let servers: Vec<Value> = config
            .servers
            .iter()
            .map(|(n, s)| server_json(n, s))
            .collect();
        print_json(&json!({ "servers": servers }));
        return;
    }
    if config.servers.is_empty() {
        println!("no servers configured");
    }
    for (name, spec) in &config.servers {
        println!(
            "{name}  {}  {}  auth {}  secrets: {}",
            spec.transport.as_str(),
            if spec.enabled { "enabled" } else { "disabled" },
            spec.auth.as_str(),
            match spec.secret_ids().join(", ") {
                s if s.is_empty() => "none".to_string(),
                s => s,
            }
        );
    }
}

fn say(json: bool, doc: Value, line: String) {
    if json {
        print_json(&doc);
    } else {
        println!("{line}");
    }
}

pub fn run(args: &McpArgs, roots: &Roots, backups: &BackupSession) -> Outcome<u8> {
    let store: Arc<dyn SecretStore> = match &args.secrets_dir {
        Some(dir) => Arc::new(FileStore::new(dir)),
        None => Arc::new(KeychainStore),
    };
    let json = args.json();
    let state = &roots.state;
    match &args.command {
        McpCommand::Serve => {
            nativemem_hub::serve::run_stdio(HubConfig::load(state)?, store)?;
        }
        McpCommand::List { .. } => list(&HubConfig::load(state)?, json),
        McpCommand::Add { .. } => {
            let (name, spec) = parse_spec_json(&read_stdin()?)?;
            let name = name.ok_or_else(|| {
                HubError::Invalid("the server spec needs a \"name\" field".into())
            })?;
            nativemem_hub::config::validate_name(&name)?;
            let mut config = HubConfig::load(state)?;
            if config.servers.contains_key(&name) {
                return Err(HubError::Exists(name).into());
            }
            config.servers.insert(name.clone(), spec);
            config.save(state, backups)?;
            say(json, json!({ "added": name }), format!("added {name}"));
        }
        McpCommand::Edit { name, .. } => {
            let (given, spec) = parse_spec_json(&read_stdin()?)?;
            if given.is_some_and(|g| g != *name) {
                return Err(HubError::Invalid(
                    "the name in the spec differs from the server being edited".into(),
                )
                .into());
            }
            let mut config = HubConfig::load(state)?;
            *config.server_mut(name)? = spec;
            config.save(state, backups)?;
            say(json, json!({ "edited": name }), format!("edited {name}"));
        }
        McpCommand::Remove { name, .. } => {
            let mut config = HubConfig::load(state)?;
            config.server(name)?;
            config.servers.remove(name);
            config.save(state, backups)?;
            say(json, json!({ "removed": name }), format!("removed {name}"));
        }
        McpCommand::Enable { name, .. } | McpCommand::Disable { name, .. } => {
            let enabled = matches!(args.command, McpCommand::Enable { .. });
            let mut config = HubConfig::load(state)?;
            config.server_mut(name)?.enabled = enabled;
            config.save(state, backups)?;
            say(
                json,
                json!({ "name": name, "enabled": enabled }),
                format!("{name} {}", if enabled { "enabled" } else { "disabled" }),
            );
        }
        McpCommand::SetSecret { id, .. } => {
            validate_secret_id(id)?;
            let input = read_stdin()?;
            let value = input
                .strip_suffix('\n')
                .map(|v| v.strip_suffix('\r').unwrap_or(v))
                .unwrap_or(&input);
            if value.is_empty() {
                return Err(HubError::Invalid("the secret value on stdin is empty".into()).into());
            }
            store.set(id, value)?;
            say(json, json!({ "secret": id }), format!("stored secret {id}"));
        }
        McpCommand::DeleteSecret { id, .. } => {
            validate_secret_id(id)?;
            if !store.delete(id)? {
                return Err(HubError::NotFound(format!("no secret {id:?}")).into());
            }
            say(
                json,
                json!({ "deleted": id }),
                format!("deleted secret {id}"),
            );
        }
        McpCommand::Test { name, .. } => {
            let config = HubConfig::load(state)?;
            let report = test_server(config.server(name)?, store)?;
            say(
                json,
                json!({ "ok": report.ok, "tools": report.tools, "error": report.error }),
                match &report.error {
                    None => format!(
                        "{name}: {} tools: {}",
                        report.tools.len(),
                        report.tools.join(", ")
                    ),
                    Some(e) => format!("{name}: {e}"),
                },
            );
            return Ok(if report.ok { 0 } else { EXIT_ATTENTION });
        }
        McpCommand::Import { apply, .. } => {
            return run_import(roots, backups, &*store, *apply, json)
        }
    }
    Ok(0)
}

fn run_import(
    roots: &Roots,
    backups: &BackupSession,
    store: &dyn SecretStore,
    apply: bool,
    json: bool,
) -> Outcome<u8> {
    let mut config = HubConfig::load(&roots.state)?;
    let plan = import::plan(
        &AgentRoots {
            claude: &roots.claude,
            codex: &roots.codex,
            opencode: &roots.opencode,
            pi: &roots.pi,
        },
        &config,
    );
    let count = |s: Status| plan.servers.iter().filter(|p| p.status == s).count();
    let (add, present, conflicts) = (
        count(Status::Add),
        count(Status::Present),
        count(Status::Conflict),
    );
    if apply {
        import::apply(&plan, &mut config, store)?;
        if add > 0 {
            config.save(&roots.state, backups)?;
        }
    }
    if json {
        let servers: Vec<Value> = plan
            .servers
            .iter()
            .map(|p| {
                json!({
                    "name": p.name,
                    "transport": p.spec.transport.as_str(),
                    "status": p.status.as_str(),
                    "agents": p.agents,
                    "secrets": p.secrets.iter().map(|s| &s.id).collect::<Vec<_>>(),
                })
            })
            .collect();
        let skipped: Vec<Value> = plan
            .skipped
            .iter()
            .map(|s| json!({ "agent": s.agent, "name": s.name, "reason": s.reason }))
            .collect();
        print_json(&json!({
            "applied": apply,
            "servers": servers,
            "skipped": skipped,
            "warnings": plan.warnings,
            "summary": {
                "add": add,
                "present": present,
                "conflicts": conflicts,
                "skipped": plan.skipped.len(),
            },
        }));
    } else {
        for p in &plan.servers {
            let secrets = if p.secrets.is_empty() {
                String::new()
            } else {
                format!(
                    "  secrets: {}",
                    p.secrets
                        .iter()
                        .map(|s| s.id.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            };
            println!(
                "{} {}  {}  ({}){secrets}",
                match p.status {
                    Status::Add => "+",
                    Status::Present => "=",
                    Status::Conflict => "!",
                },
                p.name,
                p.spec.transport.as_str(),
                p.agents.join(", ")
            );
        }
        for s in &plan.skipped {
            println!("skipped {} {}: {}", s.agent, s.name, s.reason);
        }
        for w in &plan.warnings {
            println!("warning: {w}");
        }
        println!(
            "{add} to add, {present} already in the hub, {conflicts} conflicts, {} skipped{}",
            plan.skipped.len(),
            if apply { " (applied)" } else { "" }
        );
    }
    Ok(if conflicts > 0 { EXIT_ATTENTION } else { 0 })
}
