use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use nativemem_core::adapters::agents_file::AgentsFileAdapter;
use nativemem_core::adapters::claude::ClaudeAdapter;
use nativemem_core::adapters::codex::CodexAdapter;
use nativemem_core::adapters::pi::PiAdapter;
use nativemem_core::adapters::skills::skill_name;
use nativemem_core::compat::{self, AgentReport, CompatState};
use nativemem_core::engine::{self, Action, ApplyOptions, Note};
use nativemem_core::error::{Error, Result};
use nativemem_core::fsx::BackupSession;
use nativemem_core::ledger::Ledger;
use nativemem_core::model::{content_hash, Adapter, Item, ItemKey, Kind, Observed, Scope};
use nativemem_core::session::{is_session_id, SessionOptions};
use serde_json::{json, Value};

mod doctor;
mod mcp;
#[cfg(target_os = "macos")]
mod schedule;

#[derive(Parser)]
#[command(
    name = "nativemem",
    version,
    about = "Keep each agent's native memory in sync"
)]
struct Cli {
    /// Claude Code home [default: $CLAUDE_CONFIG_DIR or ~/.claude]
    #[arg(long, global = true, env = "NATIVEMEM_CLAUDE_HOME")]
    claude_home: Option<PathBuf>,
    /// Codex home [default: $CODEX_HOME or ~/.codex]
    #[arg(long, global = true, env = "NATIVEMEM_CODEX_HOME")]
    codex_home: Option<PathBuf>,
    /// OpenCode home [default: $OPENCODE_CONFIG_DIR or ~/.config/opencode]
    #[arg(long, global = true, env = "NATIVEMEM_OPENCODE_HOME")]
    opencode_home: Option<PathBuf>,
    /// Pi home [default: $PI_CODING_AGENT_DIR or ~/.pi/agent]
    #[arg(long, global = true, env = "NATIVEMEM_PI_HOME")]
    pi_home: Option<PathBuf>,
    /// Ledger and backup directory [default: ~/.nativemem]
    #[arg(long, global = true, env = "NATIVEMEM_STATE_DIR")]
    state_dir: Option<PathBuf>,
    /// Only sync chat sessions updated in the last N days [default: all]
    #[arg(long, global = true, env = "NATIVEMEM_SESSION_DAYS")]
    session_days: Option<u32>,
    /// Leave headless runs and sub-agent sessions out of session sync
    #[arg(long, global = true)]
    exclude_automated_sessions: bool,
    /// Turn chat session sync off
    #[arg(long, global = true)]
    no_sessions: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Show what each agent holds and what cannot be synced
    Status {
        /// Print one JSON document
        #[arg(long)]
        json: bool,
    },
    /// Print the sync plan; with --apply, execute it
    Sync {
        #[arg(long)]
        apply: bool,
        /// Lift the mass-delete guard
        #[arg(long)]
        allow_mass_delete: bool,
        /// Show a macOS notification when an agent's compatibility status changes
        #[arg(long, requires = "apply")]
        notify: bool,
        /// Print one JSON document
        #[arg(long)]
        json: bool,
    },
    /// Check that the agents' on-disk formats still match what the adapters were written for
    Doctor {
        /// Only this agent: claude, codex, opencode or pi
        #[arg(long)]
        agent: Option<String>,
        /// Mark the current observed values of this agent's failing checks as reviewed
        #[arg(long)]
        accept: Option<String>,
        /// Print the prompt that lets a coding agent fix this agent's adapter
        #[arg(long)]
        prompt: Option<String>,
        /// Print one JSON document
        #[arg(long)]
        json: bool,
    },
    /// List every entry found in the agents' native stores
    Items {
        /// Only entries of this kind; sessions are listed only when asked for
        #[arg(long, value_parser = ["memory", "instruction", "session", "skill"])]
        kind: Option<String>,
        /// Print one JSON document
        #[arg(long)]
        json: bool,
    },
    /// Remove one entry from an agent's native store (with backup); does not sync
    Delete {
        /// claude, codex, opencode or pi
        #[arg(long)]
        agent: String,
        /// The entry's native id, as listed by `items`
        #[arg(long, allow_hyphen_values = true)]
        id: String,
        /// Print one JSON document
        #[arg(long)]
        json: bool,
    },
    /// Rewrite one original entry from a JSON document on stdin; does not sync
    Edit {
        /// claude, codex, opencode or pi
        #[arg(long)]
        agent: String,
        /// The entry's native id, as listed by `items`
        #[arg(long, allow_hyphen_values = true)]
        id: String,
        /// Print one JSON document
        #[arg(long)]
        json: bool,
    },
    /// Skills of the agents
    Skill(SkillArgs),
    /// The MCP hub: one local MCP server that holds the real servers and their secrets
    Mcp(mcp::McpArgs),
    /// Run `sync --apply` on a timer through a launchd agent (macOS)
    Schedule(ScheduleArgs),
}

#[derive(clap::Args)]
struct SkillArgs {
    #[command(subcommand)]
    command: SkillCommand,
}

#[derive(Subcommand)]
enum SkillCommand {
    /// Print the SKILL.md of one skill
    Show {
        /// claude, codex, opencode or pi
        #[arg(long)]
        agent: String,
        /// The skill's directory name
        #[arg(long, allow_hyphen_values = true)]
        name: String,
        /// Print one JSON document
        #[arg(long)]
        json: bool,
    },
}

#[derive(clap::Args)]
struct ScheduleArgs {
    /// Where the launchd plist is written [default: ~/Library/LaunchAgents]
    #[arg(long, global = true, env = "NATIVEMEM_LAUNCH_AGENTS_DIR")]
    launch_agents_dir: Option<PathBuf>,
    /// Skip every launchctl call
    #[arg(long, global = true, hide = true)]
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    no_launchctl: bool,
    #[command(subcommand)]
    command: ScheduleCommand,
}

#[derive(Subcommand)]
enum ScheduleCommand {
    /// Install or update the scheduled job
    Install {
        /// Minutes between runs
        #[arg(long, default_value_t = 60, value_parser = clap::value_parser!(u32).range(5..))]
        every: u32,
    },
    /// Remove the scheduled job; the state directory is left alone
    Uninstall,
    /// Show whether the job is installed and loaded
    Status {
        /// Print one JSON document
        #[arg(long)]
        json: bool,
    },
}

enum Failure {
    Coded { code: &'static str, message: String },
    Core(Error),
    UnknownAgent(String),
    NotFound { agent: String, id: String },
    Replica,
}

impl From<Error> for Failure {
    fn from(e: Error) -> Self {
        Failure::Core(e)
    }
}

impl Failure {
    fn code(&self) -> &'static str {
        match self {
            Failure::Coded { code, .. } => code,
            Failure::UnknownAgent(_) => "unknown_agent",
            Failure::NotFound { .. } => "not_found",
            Failure::Replica => "unsupported",
            Failure::Core(Error::MassDelete { .. }) => "mass_delete",
            Failure::Core(Error::Unsupported { .. }) => "unsupported",
            Failure::Core(Error::Io { .. }) => "io",
            Failure::Core(Error::Parse(_)) => "parse",
            Failure::Core(Error::Ledger(_)) => "ledger",
        }
    }

    fn message(&self) -> String {
        match self {
            Failure::Coded { message, .. } => message.clone(),
            Failure::UnknownAgent(a) => {
                format!("unknown agent {a:?} (expected claude, codex, opencode or pi)")
            }
            Failure::NotFound { agent, id } => format!("no {agent} entry with id {id:?}"),
            Failure::Replica => {
                "copies are edited through their original, not directly".to_string()
            }
            Failure::Core(e) => e.to_string(),
        }
    }
}

type Outcome<T> = std::result::Result<T, Failure>;

fn print_json(doc: &Value) {
    println!("{doc}");
}

fn scope_json(scope: &Scope) -> Value {
    match scope {
        Scope::Global => json!({ "type": "global" }),
        Scope::Project(p) => json!({ "type": "project", "path": p.display().to_string() }),
    }
}

fn key_json(key: &ItemKey) -> Value {
    json!({ "origin": key.origin.as_str(), "native_id": key.native_id })
}

/// Below the user's home; Codex and Pi read skills from it besides their own directories.
const SHARED_SKILLS: &str = ".agents/skills";

const AGENT_IDS: [&str; 4] = ["claude", "codex", "opencode", "pi"];

/// The kinds each agent leaves alone because a compatibility check fails.
type Blocked = BTreeMap<String, Vec<Kind>>;

struct Roots {
    claude: PathBuf,
    codex: PathBuf,
    opencode: PathBuf,
    pi: PathBuf,
    state: PathBuf,
    /// The shared skills directory Codex and Pi also read: `<user home>/.agents/skills`.
    shared_skills: Option<PathBuf>,
}

/// The agents that take part in a run: those whose home directory exists.
struct Agents {
    claude: Option<ClaudeAdapter>,
    codex: Option<CodexAdapter>,
    opencode: Option<AgentsFileAdapter>,
    pi: Option<PiAdapter>,
}

impl Agents {
    fn open(
        roots: &Roots,
        backups: &BackupSession,
        sessions: Option<SessionOptions>,
        blocked: &Blocked,
    ) -> Self {
        // Codex session writes are opt-in and, once on, still follow the compatibility probes.
        let codex_session_writes =
            std::env::var("NATIVEMEM_CODEX_SESSION_WRITES").is_ok_and(|v| v == "on");
        let state = || roots.state.clone();
        let present = |dir: &Path| dir.is_dir();
        let paused = |id: &str| blocked.get(id).cloned().unwrap_or_default();
        let shared = || roots.shared_skills.clone().into_iter().collect::<Vec<_>>();
        Agents {
            claude: present(&roots.claude).then(|| {
                let a = ClaudeAdapter::new(roots.claude.clone(), state(), backups.clone())
                    .with_blocked_kinds(&paused("claude"));
                match sessions {
                    Some(o) => a.with_sessions(o),
                    None => a,
                }
            }),
            codex: present(&roots.codex).then(|| {
                let a = CodexAdapter::new(roots.codex.clone(), state(), backups.clone())
                    .with_claude_home(roots.claude.clone())
                    .with_session_writes(codex_session_writes)
                    .with_visible_roots(shared())
                    .with_blocked_kinds(&paused("codex"));
                match sessions {
                    Some(o) => a.with_sessions(o),
                    None => a,
                }
            }),
            opencode: present(&roots.opencode).then(|| {
                AgentsFileAdapter::new("opencode", roots.opencode.clone(), state(), backups.clone())
                    .with_blocked_kinds(&paused("opencode"))
            }),
            pi: present(&roots.pi).then(|| {
                let a = PiAdapter::new(roots.pi.clone(), state(), backups.clone())
                    .with_visible_roots(shared())
                    .with_blocked_kinds(&paused("pi"));
                match sessions {
                    Some(o) => a.with_sessions(o),
                    None => a,
                }
            }),
        }
    }

    fn all(&self) -> Vec<&dyn Adapter> {
        let mut out: Vec<&dyn Adapter> = Vec::new();
        out.extend(self.claude.as_ref().map(|a| a as &dyn Adapter));
        out.extend(self.codex.as_ref().map(|a| a as &dyn Adapter));
        out.extend(self.opencode.as_ref().map(|a| a as &dyn Adapter));
        out.extend(self.pi.as_ref().map(|a| a as &dyn Adapter));
        out
    }

    /// `Err` for a name that is no agent, `Ok(None)` for an agent without a home directory.
    fn named(&self, name: &str) -> Outcome<Option<&dyn Adapter>> {
        Ok(match name {
            "claude" => self.claude.as_ref().map(|a| a as &dyn Adapter),
            "codex" => self.codex.as_ref().map(|a| a as &dyn Adapter),
            "opencode" => self.opencode.as_ref().map(|a| a as &dyn Adapter),
            "pi" => self.pi.as_ref().map(|a| a as &dyn Adapter),
            other => return Err(Failure::UnknownAgent(other.to_string())),
        })
    }
}

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

fn resolve(cli: &Cli) -> Result<Roots> {
    let home = || {
        std::env::home_dir().ok_or_else(|| Error::Parse("cannot determine home directory".into()))
    };
    let pick =
        |given: &Option<PathBuf>, fallback_env: Option<&str>, dir: &str| -> Result<PathBuf> {
            if let Some(p) = given {
                return Ok(p.clone());
            }
            if let Some(p) = fallback_env.and_then(env_path) {
                return Ok(p);
            }
            Ok(home()?.join(dir))
        };
    Ok(Roots {
        claude: pick(&cli.claude_home, Some("CLAUDE_CONFIG_DIR"), ".claude")?,
        codex: pick(&cli.codex_home, Some("CODEX_HOME"), ".codex")?,
        opencode: pick(
            &cli.opencode_home,
            Some("OPENCODE_CONFIG_DIR"),
            ".config/opencode",
        )?,
        pi: pick(&cli.pi_home, Some("PI_CODING_AGENT_DIR"), ".pi/agent")?,
        state: pick(&cli.state_dir, None, ".nativemem")?,
        shared_skills: std::env::home_dir().map(|h| h.join(SHARED_SKILLS)),
    })
}

/// Held for the whole write run so the scheduled job and the desktop app never interleave.
fn lock_state(state: &Path) -> Outcome<File> {
    std::fs::create_dir_all(state).map_err(Error::from)?;
    let file = File::options()
        .create(true)
        .append(true)
        .open(state.join("sync.lock"))
        .map_err(Error::from)?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(std::fs::TryLockError::WouldBlock) => Err(Failure::Coded {
            code: "busy",
            message: "another nativemem run is in progress".into(),
        }),
        Err(std::fs::TryLockError::Error(e)) => Err(Error::from(e).into()),
    }
}

fn status(roots: &Roots, present: &Agents, reports: &[AgentReport], json: bool) -> Result<()> {
    let mut agents = Vec::new();
    for a in present.all() {
        let observed = a.scan()?;
        let sessions = observed
            .iter()
            .filter(|o| o.item.kind == Kind::Session)
            .count();
        let skills = observed
            .iter()
            .filter(|o| o.item.kind == Kind::Skill)
            .count();
        let entries = || {
            observed
                .iter()
                .filter(|o| !matches!(o.item.kind, Kind::Session | Kind::Skill))
        };
        let replicas = entries().filter(|o| o.replica_of.is_some()).count();
        let originals = entries().count() - replicas;
        // The prefix is only used by the human output.
        let mut skipped: Vec<(&str, String, String)> = Vec::new();
        if let Some(claude) = present
            .claude
            .as_ref()
            .filter(|_| a.id().as_str() == "claude")
        {
            skipped.extend(
                claude
                    .skipped_projects()?
                    .into_iter()
                    .map(|(l, r)| ("project ", l, r)),
            );
            skipped.extend(
                claude
                    .skipped_files()?
                    .into_iter()
                    .map(|(l, r)| ("file ", l, r)),
            );
        }
        if let Some(codex) = present
            .codex
            .as_ref()
            .filter(|_| a.id().as_str() == "codex")
        {
            skipped.extend(codex.skipped()?.into_iter().map(|(l, r)| ("", l, r)));
        }
        skipped.extend(
            a.skill_report()?
                .skipped
                .into_iter()
                .map(|(l, r)| ("skill ", l, r)),
        );
        let report = match a.id().as_str() {
            "claude" => present.claude.as_ref().map(|c| c.session_report()),
            "codex" => present.codex.as_ref().map(|c| c.session_report()),
            "pi" => present.pi.as_ref().map(|c| c.session_report()),
            _ => None,
        };
        let mut automated = 0;
        if let Some(report) = report {
            let report = report?;
            automated = report.automated;
            skipped.extend(report.skipped.into_iter().map(|(l, r)| ("session ", l, r)));
        }
        if json {
            agents.push(json!({
                "id": a.id().as_str(),
                "originals": originals,
                "replicas": replicas,
                "sessions": sessions,
                "skills": skills,
                "automated_sessions": automated,
                "skipped": skipped
                    .iter()
                    .map(|(_, label, reason)| json!({ "label": label, "reason": reason }))
                    .collect::<Vec<_>>(),
            }));
        } else {
            println!(
                "{}: {originals} originals, {replicas} replicas, {sessions} sessions, {automated} automated, {skills} skills",
                a.id()
            );
            for (kind, label, reason) in &skipped {
                println!("  skipped {kind}{label}: {reason}");
            }
        }
    }
    let items = if roots.state.join("ledger.db").is_file() {
        Ledger::open(&roots.state)?
            .list_items()?
            .iter()
            .filter(|r| r.kind != Kind::Session)
            .count()
    } else {
        0
    };
    if json {
        print_json(&json!({
            "agents": agents,
            "ledger_items": items,
            "compat": doctor::summary_json(reports),
        }));
    } else {
        println!("ledger: {items} items");
        for line in doctor::warning_lines(reports) {
            println!("{line}");
        }
        for line in doctor::notice_lines(reports) {
            println!("{line}");
        }
    }
    Ok(())
}

fn items(roots: &Roots, agents: &Agents, kind: Option<Kind>, json: bool) -> Result<()> {
    let mut observed: Vec<Observed> = Vec::new();
    for a in agents.all() {
        observed.extend(a.scan()?);
    }
    let ledger = if roots.state.join("ledger.db").is_file() {
        Ledger::open(&roots.state)?
    } else {
        Ledger::in_memory()?
    };
    engine::link_skills(&mut observed, &ledger)?;
    observed.retain(|o| match kind {
        Some(k) => o.item.kind == k,
        None => o.item.kind != Kind::Session,
    });
    if json {
        let list: Vec<Value> = observed
            .iter()
            .map(|o| {
                json!({
                    "agent": o.agent.as_str(),
                    "native_id": o.native_id,
                    "kind": o.item.kind.as_str(),
                    "path": o.path.display().to_string(),
                    "scope": scope_json(&o.item.scope),
                    "name": o.item.name,
                    "description": o.item.description,
                    "category": o.item.category,
                    "body": if matches!(o.item.kind, Kind::Session | Kind::Skill) { "" } else { &o.item.body },
                    "replica_of": o.replica_of.as_ref().map(key_json),
                    "hash": content_hash(&o.item),
                })
            })
            .collect();
        print_json(&json!({ "items": list }));
    } else {
        for o in &observed {
            let scope = match &o.item.scope {
                Scope::Global => "global".to_string(),
                Scope::Project(p) => p.display().to_string(),
            };
            println!(
                "{}  {}  {}  {}",
                o.agent,
                o.item.kind.as_str(),
                scope,
                if o.item.kind == Kind::Session {
                    &o.item.description
                } else {
                    &o.item.name
                }
            );
        }
    }
    Ok(())
}

fn delete(
    roots: &Roots,
    backups: &BackupSession,
    agents: &Agents,
    agent: &str,
    id: &str,
    json: bool,
) -> Outcome<()> {
    let adapter = agents.named(agent)?.ok_or_else(|| Failure::NotFound {
        agent: agent.to_string(),
        id: id.to_string(),
    })?;
    refuse_session(id)?;
    adapter.remove(id)?;
    let backup_dir = roots.state.join("backups").join(backups.stamp());
    if json {
        print_json(&json!({
            "deleted": { "agent": agent, "native_id": id },
            "backup_dir": backup_dir.display().to_string(),
        }));
    } else {
        println!("deleted {agent}  {id}");
        println!("backup: {}", backup_dir.display());
    }
    Ok(())
}

/// Sessions are logs the user manages in their own agent; this tool never edits or deletes them.
fn refuse_session(id: &str) -> Outcome<()> {
    if is_session_id(id) {
        return Err(Failure::Coded {
            code: "unsupported",
            message: "chat sessions cannot be deleted or edited from here".into(),
        });
    }
    Ok(())
}

fn skill_show(agents: &Agents, agent: &str, name: &str, json: bool) -> Outcome<()> {
    let not_found = || Failure::NotFound {
        agent: agent.to_string(),
        id: name.to_string(),
    };
    let adapter = agents.named(agent)?.ok_or_else(not_found)?;
    let id = nativemem_core::adapters::skills::native_id(name);
    let skill = find_entry(adapter, &id)?.ok_or_else(not_found)?;
    let file = skill.path.join("SKILL.md");
    let text = String::from_utf8_lossy(&std::fs::read(&file).map_err(|e| Error::io(&file, e))?)
        .into_owned();
    if json {
        print_json(&json!({
            "agent": agent,
            "name": name,
            "path": file.display().to_string(),
            "text": text,
        }));
    } else {
        print!("{text}");
    }
    Ok(())
}

fn parse_edit(input: &str) -> Result<(String, String, Option<String>, String)> {
    let doc: Value = serde_json::from_str(input).map_err(|e| Error::Parse(e.to_string()))?;
    let text = |key: &str| -> Result<String> {
        doc.get(key)
            .and_then(Value::as_str)
            .map(String::from)
            .ok_or_else(|| Error::Parse(format!("missing string field {key:?}")))
    };
    let category = match doc.get("category") {
        None | Some(Value::Null) => None,
        Some(Value::String(c)) => Some(c.clone()),
        Some(_) => return Err(Error::Parse("category must be a string or null".into())),
    };
    Ok((text("name")?, text("description")?, category, text("body")?))
}

fn find_entry(adapter: &dyn Adapter, id: &str) -> Result<Option<Observed>> {
    Ok(adapter.scan()?.into_iter().find(|o| o.native_id == id))
}

fn edit(
    roots: &Roots,
    backups: &BackupSession,
    agents: &Agents,
    agent: &str,
    id: &str,
    json: bool,
) -> Outcome<()> {
    let adapter = agents.named(agent)?.ok_or_else(|| Failure::NotFound {
        agent: agent.to_string(),
        id: id.to_string(),
    })?;
    refuse_session(id)?;
    if skill_name(id).is_some() {
        return Err(Failure::Coded {
            code: "unsupported",
            message: "skills are edited as files in their directory, not from here".into(),
        });
    }
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .map_err(Error::from)?;
    let (name, description, category, body) = parse_edit(&input)?;
    let current = find_entry(adapter, id)?.ok_or_else(|| Failure::NotFound {
        agent: agent.to_string(),
        id: id.to_string(),
    })?;
    if current.replica_of.is_some() {
        return Err(Failure::Replica);
    }
    let (name, description, category) = if current.item.kind == Kind::Instruction {
        if name != current.item.name || description != current.item.description {
            return Err(Failure::Coded {
                code: "invalid",
                message: "the heading cannot be changed from here; edit it in the file itself"
                    .into(),
            });
        }
        (name, description, None)
    } else {
        (name, description, category)
    };
    let item = Item {
        kind: current.item.kind,
        scope: current.item.scope,
        name,
        description,
        category,
        body,
        source_dir: None,
    };
    adapter.write_original(id, &item)?;
    let written = find_entry(adapter, id)?.ok_or_else(|| Failure::NotFound {
        agent: agent.to_string(),
        id: id.to_string(),
    })?;
    let hash = content_hash(&written.item);
    let backup_dir = roots.state.join("backups").join(backups.stamp());
    if json {
        print_json(&json!({
            "edited": { "agent": agent, "native_id": id },
            "hash": hash,
            "backup_dir": backup_dir.display().to_string(),
        }));
    } else {
        println!("edited {agent}  {id}  backup: {}", backup_dir.display());
    }
    Ok(())
}

struct Counts {
    add: usize,
    update: usize,
    remove: usize,
    conflicts: usize,
    skipped: usize,
}

/// Actions that only touch the ledger have no entry-level effect and are not listed.
fn action_json(action: &Action) -> Option<(&'static str, Value)> {
    let (op, agent, name, scope, key) = match action {
        Action::Write {
            key,
            target,
            item,
            update,
        } => (
            if *update { "update" } else { "add" },
            target.as_str(),
            &item.name,
            &item.scope,
            key,
        ),
        Action::WriteBack { key, item, .. } => {
            ("update", key.origin.as_str(), &item.name, &item.scope, key)
        }
        Action::Remove {
            key,
            agent,
            name,
            scope,
            ..
        } => ("remove", agent.as_str(), name, scope, key),
        Action::Adopt { .. } | Action::Forget { .. } | Action::Tombstone { .. } => return None,
    };
    Some((
        op,
        json!({
            "op": op,
            "agent": agent,
            "name": name,
            "scope": scope_json(scope),
            "origin": key_json(key),
        }),
    ))
}

fn note_json(note: &Note) -> (&'static str, Value) {
    let (kind, message) = match note {
        Note::Conflict { key, reason } => (
            "conflict",
            format!("{}  {}: {reason}", key.origin, key.native_id),
        ),
        Note::Skipped {
            agent,
            label,
            reason,
        } => ("skipped", format!("{agent}  {label}: {reason}")),
    };
    (kind, json!({ "kind": kind, "message": message }))
}

fn sync_json(applied: bool, actions: &[Action], notes: &[Note]) -> Value {
    let mut c = Counts {
        add: 0,
        update: 0,
        remove: 0,
        conflicts: 0,
        skipped: 0,
    };
    let mut listed = Vec::new();
    for a in actions {
        if let Some((op, doc)) = action_json(a) {
            match op {
                "add" => c.add += 1,
                "update" => c.update += 1,
                _ => c.remove += 1,
            }
            listed.push(doc);
        }
    }
    let mut notes_out = Vec::new();
    for n in notes {
        let (kind, doc) = note_json(n);
        match kind {
            "conflict" => c.conflicts += 1,
            _ => c.skipped += 1,
        }
        notes_out.push(doc);
    }
    json!({
        "applied": applied,
        "actions": listed,
        "notes": notes_out,
        "summary": {
            "add": c.add,
            "update": c.update,
            "remove": c.remove,
            "conflicts": c.conflicts,
            "skipped": c.skipped,
        },
    })
}

fn sync(
    roots: &Roots,
    agents: &Agents,
    reports: &[AgentReport],
    apply: bool,
    allow_mass_delete: bool,
    json: bool,
) -> Result<bool> {
    let adapters = agents.all();
    if !json {
        for line in doctor::paused_lines(reports) {
            println!("{line}");
        }
    }
    if apply {
        let ledger = Ledger::open(&roots.state)?;
        let plan = engine::plan(&adapters, &ledger)?;
        let report = engine::apply(
            plan,
            &adapters,
            &ledger,
            &ApplyOptions { allow_mass_delete },
        )?;
        if json {
            print_json(&sync_json(true, &report.done, &report.notes));
        } else {
            print!("{report}");
            println!("{}", report.summary());
        }
        return Ok(report.needs_attention());
    }
    let ledger = if roots.state.join("ledger.db").is_file() {
        Ledger::open(&roots.state)?
    } else {
        Ledger::in_memory()?
    };
    let plan = engine::plan(&adapters, &ledger)?;
    if json {
        print_json(&sync_json(false, &plan.actions, &plan.notes));
    } else {
        print!("{plan}");
        println!("{}", plan.summary());
    }
    Ok(plan.needs_attention())
}

#[cfg(target_os = "macos")]
fn run_schedule(cli: &Cli, args: &ScheduleArgs, roots: &Roots) -> Outcome<()> {
    let explicit = schedule::ExplicitRoots {
        claude: cli
            .claude_home
            .clone()
            .or_else(|| env_path("CLAUDE_CONFIG_DIR")),
        codex: cli.codex_home.clone().or_else(|| env_path("CODEX_HOME")),
        opencode: cli
            .opencode_home
            .clone()
            .or_else(|| env_path("OPENCODE_CONFIG_DIR")),
        pi: cli
            .pi_home
            .clone()
            .or_else(|| env_path("PI_CODING_AGENT_DIR")),
        state: cli.state_dir.clone(),
    };
    schedule::run(args, roots, &explicit)
}

#[cfg(not(target_os = "macos"))]
fn run_schedule(_: &Cli, _: &ScheduleArgs, _: &Roots) -> Outcome<()> {
    Err(Failure::Coded {
        code: "unsupported_platform",
        message: "scheduled sync is only available on macOS".into(),
    })
}

/// Exit code of runs that finished but need the user's attention.
const EXIT_ATTENTION: u8 = 2;

fn run(cli: &Cli) -> Outcome<u8> {
    let roots = resolve(cli)?;
    let backups = BackupSession::new();
    let sessions = (!cli.no_sessions).then_some(SessionOptions {
        days: cli.session_days,
        exclude_automated: cli.exclude_automated_sessions,
    });
    let _lock = match &cli.command {
        Command::Sync { apply: true, .. }
        | Command::Delete { .. }
        | Command::Edit { .. }
        | Command::Doctor {
            accept: Some(_), ..
        } => Some(lock_state(&roots.state)?),
        Command::Mcp(args) if args.writes() => Some(lock_state(&roots.state)?),
        _ => None,
    };
    let watches_compat = matches!(
        cli.command,
        Command::Status { .. } | Command::Sync { .. } | Command::Doctor { .. }
    );
    let loaded = CompatState::load(&roots.state);
    let mut state = loaded.clone();
    let reports = if watches_compat {
        let probing = Agents::open(&roots, &backups, sessions, &Blocked::new());
        doctor::assess_all(&probing, &mut state)
    } else {
        Vec::new()
    };
    let agents = Agents::open(&roots, &backups, sessions, &doctor::blocked_kinds(&reports));
    let attention = |needs: bool| if needs { EXIT_ATTENTION } else { 0 };
    match &cli.command {
        Command::Status { json } => {
            let mut reports = reports;
            doctor::add_notices(&agents, &mut reports);
            status(&roots, &agents, &reports, *json)
                .map(|()| 0)
                .map_err(Failure::from)
        }
        Command::Sync {
            apply,
            allow_mass_delete,
            notify,
            json,
        } => {
            let needs = sync(&roots, &agents, &reports, *apply, *allow_mass_delete, *json)?;
            if *apply {
                let notes = if *notify {
                    compat::pending_notifications(&reports, &mut state)
                } else {
                    Vec::new()
                };
                if state != loaded {
                    state.save(&roots.state, &backups)?;
                }
                doctor::deliver(&notes);
            }
            Ok(attention(needs))
        }
        Command::Doctor {
            agent,
            accept,
            prompt,
            json,
        } => doctor::run(
            &roots,
            &backups,
            &agents,
            doctor::Context {
                loaded,
                state,
                reports,
            },
            &doctor::Args {
                agent: agent.as_deref(),
                accept: accept.as_deref(),
                prompt: prompt.as_deref(),
                json: *json,
            },
        ),
        Command::Items { kind, json } => items(
            &roots,
            &agents,
            kind.as_deref().and_then(Kind::parse),
            *json,
        )
        .map(|()| 0)
        .map_err(Failure::from),
        Command::Delete { agent, id, json } => {
            delete(&roots, &backups, &agents, agent, id, *json).map(|()| 0)
        }
        Command::Edit { agent, id, json } => {
            edit(&roots, &backups, &agents, agent, id, *json).map(|()| 0)
        }
        Command::Skill(SkillArgs {
            command: SkillCommand::Show { agent, name, json },
        }) => skill_show(&agents, agent, name, *json).map(|()| 0),
        Command::Mcp(args) => mcp::run(args, &roots, &backups),
        Command::Schedule(args) => run_schedule(cli, args, &roots).map(|()| 0),
    }
}

fn wants_json(command: &Command) -> bool {
    match command {
        Command::Status { json }
        | Command::Sync { json, .. }
        | Command::Doctor { json, .. }
        | Command::Items { json, .. }
        | Command::Delete { json, .. }
        | Command::Edit { json, .. }
        | Command::Skill(SkillArgs {
            command: SkillCommand::Show { json, .. },
        })
        | Command::Schedule(ScheduleArgs {
            command: ScheduleCommand::Status { json },
            ..
        }) => *json,
        Command::Mcp(args) => args.json(),
        Command::Schedule(_) => false,
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            if wants_json(&cli.command) {
                print_json(&json!({ "error": e.message(), "code": e.code() }));
            } else {
                eprintln!("error: {}", e.message());
            }
            ExitCode::from(1)
        }
    }
}
