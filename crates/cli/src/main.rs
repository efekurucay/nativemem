use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use nativemem_core::adapters::claude::ClaudeAdapter;
use nativemem_core::adapters::codex::CodexAdapter;
use nativemem_core::engine::{self, ApplyOptions};
use nativemem_core::error::{Error, Result};
use nativemem_core::fsx::BackupSession;
use nativemem_core::ledger::Ledger;
use nativemem_core::model::Adapter;

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
    /// Ledger and backup directory [default: ~/.nativemem]
    #[arg(long, global = true, env = "NATIVEMEM_STATE_DIR")]
    state_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Show what each agent holds and what cannot be synced
    Status,
    /// Print the sync plan; with --apply, execute it
    Sync {
        #[arg(long)]
        apply: bool,
        /// Lift the mass-delete guard
        #[arg(long)]
        allow_mass_delete: bool,
    },
}

struct Roots {
    claude: PathBuf,
    codex: PathBuf,
    state: PathBuf,
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
        state: pick(&cli.state_dir, None, ".nativemem")?,
    })
}

fn status(roots: &Roots, claude: &ClaudeAdapter, codex: &CodexAdapter) -> Result<()> {
    let adapters: [&dyn Adapter; 2] = [claude, codex];
    for a in adapters {
        let observed = a.scan()?;
        let replicas = observed.iter().filter(|o| o.replica_of.is_some()).count();
        println!(
            "{}: {} originals, {} replicas",
            a.id(),
            observed.len() - replicas,
            replicas
        );
        if a.id().as_str() == "claude" {
            for (dir, reason) in claude.skipped_projects()? {
                println!("  skipped project {dir}: {reason}");
            }
            for (id, reason) in claude.skipped_files()? {
                println!("  skipped file {id}: {reason}");
            }
        } else {
            for (label, reason) in codex.skipped()? {
                println!("  skipped {label}: {reason}");
            }
        }
    }
    let items = if roots.state.join("ledger.db").is_file() {
        Ledger::open(&roots.state)?.list_items()?.len()
    } else {
        0
    };
    println!("ledger: {items} items");
    Ok(())
}

fn sync(
    roots: &Roots,
    claude: &ClaudeAdapter,
    codex: &CodexAdapter,
    apply: bool,
    allow_mass_delete: bool,
) -> Result<bool> {
    let adapters: [&dyn Adapter; 2] = [claude, codex];
    if apply {
        let ledger = Ledger::open(&roots.state)?;
        let plan = engine::plan(&adapters, &ledger)?;
        let report = engine::apply(
            plan,
            &adapters,
            &ledger,
            &ApplyOptions { allow_mass_delete },
        )?;
        print!("{report}");
        println!("{}", report.summary());
        return Ok(report.needs_attention());
    }
    let ledger = if roots.state.join("ledger.db").is_file() {
        Ledger::open(&roots.state)?
    } else {
        Ledger::in_memory()?
    };
    let plan = engine::plan(&adapters, &ledger)?;
    print!("{plan}");
    println!("{}", plan.summary());
    Ok(plan.needs_attention())
}

fn run(cli: &Cli) -> Result<bool> {
    let roots = resolve(cli)?;
    let backups = BackupSession::new();
    let claude = ClaudeAdapter::new(roots.claude.clone(), roots.state.clone(), backups.clone());
    let codex = CodexAdapter::new(roots.codex.clone(), roots.state.clone(), backups);
    match &cli.command {
        Command::Status => status(&roots, &claude, &codex).map(|()| false),
        Command::Sync {
            apply,
            allow_mass_delete,
        } => sync(&roots, &claude, &codex, *apply, *allow_mass_delete),
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(false) => ExitCode::SUCCESS,
        Ok(true) => ExitCode::from(2),
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(1)
        }
    }
}
