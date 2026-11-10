//! `nativemem subagent`: the subagent definitions in `<state dir>/subagents.toml` and a way to
//! run one.

use std::ffi::OsString;
use std::io::Read;
use std::path::PathBuf;

use clap::Subcommand;
use nativemem_core::error::Error;
use nativemem_core::fsx::BackupSession;
use nativemem_hub::harness;
use nativemem_hub::runner::Runner;
use nativemem_hub::subagents::{parse_spec_json, validate_name, SubagentConfig, SubagentSpec};
use nativemem_hub::HubError;
use serde_json::{json, Value};

use crate::{print_json, Outcome, Roots, EXIT_ATTENTION};

#[derive(clap::Args)]
pub struct SubagentArgs {
    /// Directories searched for the harness binaries instead of PATH and the usual install directories (tests)
    #[arg(long, global = true, hide = true)]
    search_path: Option<OsString>,
    #[command(subcommand)]
    command: SubagentCommand,
}

#[derive(Subcommand)]
pub enum SubagentCommand {
    /// List the defined subagents
    List {
        /// Print one JSON document
        #[arg(long)]
        json: bool,
    },
    /// Define a subagent from a JSON document on stdin (`name`, `harness`, `model`, `description`, optional `instructions`, `timeout_secs`, `args`)
    Add {
        /// Print one JSON document
        #[arg(long)]
        json: bool,
    },
    /// Replace a subagent's definition with a JSON document on stdin
    Edit {
        name: String,
        /// Print one JSON document
        #[arg(long)]
        json: bool,
    },
    /// Remove a subagent
    Remove {
        name: String,
        /// Print one JSON document
        #[arg(long)]
        json: bool,
    },
    /// Run a subagent once with the prompt in a file, to try it
    Run {
        name: String,
        /// File that holds the prompt; `-` reads stdin
        #[arg(long, allow_hyphen_values = true)]
        prompt_file: PathBuf,
        /// Absolute working directory of the run [default: the current directory]
        #[arg(long)]
        cwd: Option<PathBuf>,
        /// Print one JSON document
        #[arg(long)]
        json: bool,
    },
}

impl SubagentArgs {
    /// Commands that change `subagents.toml` run under the state lock.
    pub fn writes(&self) -> bool {
        matches!(
            self.command,
            SubagentCommand::Add { .. }
                | SubagentCommand::Edit { .. }
                | SubagentCommand::Remove { .. }
        )
    }

    pub fn json(&self) -> bool {
        match &self.command {
            SubagentCommand::List { json }
            | SubagentCommand::Add { json }
            | SubagentCommand::Edit { json, .. }
            | SubagentCommand::Remove { json, .. }
            | SubagentCommand::Run { json, .. } => *json,
        }
    }

    fn search_path(&self) -> OsString {
        self.search_path
            .clone()
            .unwrap_or_else(harness::search_path)
    }
}

fn read_stdin() -> Outcome<String> {
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .map_err(Error::from)?;
    Ok(input)
}

fn spec_json(name: &str, spec: &SubagentSpec) -> Value {
    let mut doc = json!({
        "name": name,
        "harness": spec.harness.as_str(),
        "model": spec.model,
        "description": spec.description,
        "timeout_secs": spec.timeout_secs,
        "args": spec.args,
    });
    if let Some(instructions) = &spec.instructions {
        doc["instructions"] = json!(instructions);
    }
    doc
}

/// The harness binary has to exist for a definition to be useful.
fn require_installed(spec: &SubagentSpec, args: &SubagentArgs) -> Outcome<()> {
    if harness::locate(spec.harness, &args.search_path()).is_none() {
        return Err(HubError::NotInstalled(format!(
            "no {} executable found",
            spec.harness.binary()
        ))
        .into());
    }
    Ok(())
}

fn say(json: bool, doc: Value, line: String) {
    if json {
        print_json(&doc);
    } else {
        println!("{line}");
    }
}

pub fn run(args: &SubagentArgs, roots: &Roots, backups: &BackupSession) -> Outcome<u8> {
    let state = &roots.state;
    let json = args.json();
    match &args.command {
        SubagentCommand::List { .. } => {
            let config = SubagentConfig::load(state)?;
            if json {
                let agents: Vec<Value> = config
                    .agents
                    .iter()
                    .map(|(name, spec)| spec_json(name, spec))
                    .collect();
                print_json(&json!({ "subagents": agents }));
            } else if config.agents.is_empty() {
                println!("no subagents defined");
            } else {
                for (name, spec) in &config.agents {
                    println!(
                        "{name}  {}  {}  {} s  {}",
                        spec.harness.as_str(),
                        spec.model,
                        spec.timeout_secs,
                        spec.description
                    );
                }
            }
        }
        SubagentCommand::Add { .. } => {
            let (name, spec) = parse_spec_json(&read_stdin()?)?;
            let name = name.ok_or_else(|| {
                HubError::Invalid("the subagent spec needs a \"name\" field".into())
            })?;
            validate_name(&name)?;
            let mut config = SubagentConfig::load(state)?;
            if config.agents.contains_key(&name) {
                return Err(HubError::SubagentExists(name).into());
            }
            require_installed(&spec, args)?;
            config.agents.insert(name.clone(), spec);
            config.save(state, backups)?;
            say(json, json!({ "added": name }), format!("added {name}"));
        }
        SubagentCommand::Edit { name, .. } => {
            let (given, spec) = parse_spec_json(&read_stdin()?)?;
            if given.is_some_and(|g| g != *name) {
                return Err(HubError::Invalid(
                    "the name in the spec differs from the subagent being edited".into(),
                )
                .into());
            }
            let mut config = SubagentConfig::load(state)?;
            config.agent(name)?;
            require_installed(&spec, args)?;
            config.agents.insert(name.clone(), spec);
            config.save(state, backups)?;
            say(json, json!({ "edited": name }), format!("edited {name}"));
        }
        SubagentCommand::Remove { name, .. } => {
            let mut config = SubagentConfig::load(state)?;
            config.agent(name)?;
            config.agents.remove(name);
            config.save(state, backups)?;
            say(json, json!({ "removed": name }), format!("removed {name}"));
        }
        SubagentCommand::Run {
            name,
            prompt_file,
            cwd,
            ..
        } => {
            let config = SubagentConfig::load(state)?;
            let spec = config.agent(name)?;
            let prompt = if prompt_file.as_os_str() == "-" {
                read_stdin()?
            } else {
                std::fs::read_to_string(prompt_file).map_err(|e| Error::io(prompt_file, e))?
            };
            if prompt.trim().is_empty() {
                return Err(HubError::Invalid("the prompt is empty".into()).into());
            }
            let runner = Runner::from_env().with_search_path(args.search_path());
            let report = runner.run_blocking(spec, &prompt, cwd.as_deref())?;
            if json {
                print_json(&json!({
                    "ok": report.ok,
                    "text": report.text,
                    "exit_code": report.exit_code,
                    "duration_ms": report.duration_ms,
                    "error": report.error,
                }));
            } else if report.ok {
                println!("{}", report.text);
            } else {
                eprintln!("error: {}", report.error.as_deref().unwrap_or_default());
            }
            return Ok(if report.ok { 0 } else { EXIT_ATTENTION });
        }
    }
    Ok(0)
}
