//! Scheduled sync through a per-user launchd agent.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use nativemem_core::error::Error;
use nativemem_core::fsx::atomic_write;
use serde_json::json;

use crate::{print_json, Failure, Outcome, Roots, ScheduleArgs, ScheduleCommand};

const LABEL: &str = "dev.nativemem.sync";
const LOG_TAIL_LINES: usize = 5;

/// Roots the user chose explicitly; the job has no shell environment, so these are baked in.
pub struct ExplicitRoots {
    pub claude: Option<PathBuf>,
    pub codex: Option<PathBuf>,
    pub opencode: Option<PathBuf>,
    pub pi: Option<PathBuf>,
    pub state: Option<PathBuf>,
}

struct Paths {
    plist: PathBuf,
    bin: PathBuf,
    log: PathBuf,
}

fn paths(args: &ScheduleArgs, roots: &Roots) -> Outcome<Paths> {
    let dir = match &args.launch_agents_dir {
        Some(d) => d.clone(),
        None => std::env::home_dir()
            .ok_or_else(|| Error::Parse("cannot determine home directory".into()))?
            .join("Library/LaunchAgents"),
    };
    Ok(Paths {
        plist: dir.join(format!("{LABEL}.plist")),
        bin: roots.state.join("bin").join("nativemem"),
        log: roots.state.join("sync.log"),
    })
}

fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
    out
}

fn plist(program_arguments: &[String], interval_seconds: u64, log: &Path) -> String {
    let args: String = program_arguments
        .iter()
        .map(|a| format!("        <string>{}</string>\n", xml_escape(a)))
        .collect();
    let log = xml_escape(&log.display().to_string());
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{LABEL}</string>
    <key>ProgramArguments</key>
    <array>
{args}    </array>
    <key>StartInterval</key>
    <integer>{interval_seconds}</integer>
    <key>RunAtLoad</key>
    <false/>
    <key>StandardOutPath</key>
    <string>{log}</string>
    <key>StandardErrorPath</key>
    <string>{log}</string>
    <key>ProcessType</key>
    <string>Background</string>
    <key>LowPriorityIO</key>
    <true/>
</dict>
</plist>
"#
    )
}

fn absolute(p: &Path) -> Outcome<PathBuf> {
    std::path::absolute(p).map_err(|e| Error::from(e).into())
}

fn coded(code: &'static str, message: impl Into<String>) -> Failure {
    Failure::Coded {
        code,
        message: message.into(),
    }
}

/// Copy the running executable so later rebuilds never change a scheduled job.
fn install_binary(dest: &Path) -> Outcome<()> {
    let exe = std::env::current_exe()
        .and_then(|p| p.canonicalize())
        .map_err(Error::from)?;
    let bytes = fs::read(&exe).map_err(Error::from)?;
    let dir = dest.parent().expect("binary path has a parent");
    fs::create_dir_all(dir).map_err(Error::from)?;
    let tmp = dir.join(".nativemem.tmp");
    fs::write(&tmp, bytes).map_err(Error::from)?;
    fs::set_permissions(&tmp, fs::Permissions::from_mode(0o755)).map_err(Error::from)?;
    fs::rename(&tmp, dest).map_err(Error::from)?;
    Ok(())
}

fn user_id() -> Outcome<String> {
    let out = Command::new("id")
        .arg("-u")
        .output()
        .map_err(|e| coded("launchctl", format!("cannot run id -u: {e}")))?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn launchctl(args: &[&str]) -> Outcome<bool> {
    let status = Command::new("launchctl")
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| coded("launchctl", format!("cannot run launchctl: {e}")))?;
    Ok(status.success())
}

fn is_loaded(args: &ScheduleArgs) -> Outcome<bool> {
    if args.no_launchctl {
        return Ok(false);
    }
    launchctl(&["print", &format!("gui/{}/{LABEL}", user_id()?)])
}

fn install(
    args: &ScheduleArgs,
    roots: &Roots,
    explicit: &ExplicitRoots,
    every: u32,
) -> Outcome<()> {
    let p = paths(args, roots)?;
    install_binary(&p.bin)?;

    let mut program = vec![absolute(&p.bin)?.display().to_string()];
    for (flag, value) in [
        ("--claude-home", &explicit.claude),
        ("--codex-home", &explicit.codex),
        ("--opencode-home", &explicit.opencode),
        ("--pi-home", &explicit.pi),
        ("--state-dir", &explicit.state),
    ] {
        if let Some(v) = value {
            program.push(flag.to_string());
            program.push(absolute(v)?.display().to_string());
        }
    }
    program.push("sync".into());
    program.push("--apply".into());

    if !args.no_launchctl && is_loaded(args)? {
        launchctl(&["bootout", &format!("gui/{}/{LABEL}", user_id()?)])?;
    }
    let doc = plist(&program, u64::from(every) * 60, &absolute(&p.log)?);
    atomic_write(&p.plist, doc.as_bytes())?;
    if !args.no_launchctl {
        let target = format!("gui/{}", user_id()?);
        if !launchctl(&["bootstrap", &target, &p.plist.display().to_string()])? {
            return Err(coded("launchctl", "launchctl bootstrap failed"));
        }
    }
    println!("installed: every {every} minutes");
    println!("plist: {}", p.plist.display());
    println!("binary: {}", p.bin.display());
    Ok(())
}

fn uninstall(args: &ScheduleArgs, roots: &Roots) -> Outcome<()> {
    let p = paths(args, roots)?;
    if !args.no_launchctl && is_loaded(args)? {
        launchctl(&["bootout", &format!("gui/{}/{LABEL}", user_id()?)])?;
    }
    match fs::remove_file(&p.plist) {
        Ok(()) => println!("uninstalled"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => println!("not installed"),
        Err(e) => return Err(Error::from(e).into()),
    }
    Ok(())
}

fn interval_minutes(plist_text: &str) -> Option<u64> {
    let after = plist_text.split("<key>StartInterval</key>").nth(1)?;
    let value = after
        .split("<integer>")
        .nth(1)?
        .split("</integer>")
        .next()?;
    Some(value.trim().parse::<u64>().ok()? / 60)
}

fn status(args: &ScheduleArgs, roots: &Roots, json: bool) -> Outcome<()> {
    let p = paths(args, roots)?;
    let text = fs::read_to_string(&p.plist).ok();
    let installed = text.is_some();
    let interval = text.as_deref().and_then(interval_minutes);
    let loaded = installed && is_loaded(args)?;
    let log_tail: Vec<String> = fs::read_to_string(&p.log)
        .map(|s| {
            let lines: Vec<&str> = s.lines().collect();
            lines[lines.len().saturating_sub(LOG_TAIL_LINES)..]
                .iter()
                .map(|l| l.to_string())
                .collect()
        })
        .unwrap_or_default();
    if json {
        print_json(&json!({
            "installed": installed,
            "loaded": loaded,
            "interval_minutes": interval,
            "log_tail": log_tail,
        }));
    } else if installed {
        println!(
            "installed: every {} minutes, {}",
            interval.map_or("?".to_string(), |m| m.to_string()),
            if loaded { "loaded" } else { "not loaded" }
        );
        for line in &log_tail {
            println!("  {line}");
        }
    } else {
        println!("not installed");
    }
    Ok(())
}

pub fn run(args: &ScheduleArgs, roots: &Roots, explicit: &ExplicitRoots) -> Outcome<()> {
    match &args.command {
        ScheduleCommand::Install { every } => install(args, roots, explicit, *every),
        ScheduleCommand::Uninstall => uninstall(args, roots),
        ScheduleCommand::Status { json } => status(args, roots, *json),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_xml_metacharacters() {
        assert_eq!(xml_escape("a&b<c>\"d'"), "a&amp;b&lt;c&gt;&quot;d&apos;");
    }

    #[test]
    fn interval_round_trips() {
        let doc = plist(&["/x".into()], 1800, Path::new("/l"));
        assert_eq!(interval_minutes(&doc), Some(30));
    }
}
