//! `nativemem doctor` and the compatibility plumbing `status` and `sync` share: probe results,
//! the kinds they pause, warnings and desktop notifications.

use std::borrow::Borrow;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use nativemem_core::compat::{
    self, kinds_text, AgentReport, Check, CompatState, Notification, Status,
};
use nativemem_core::fsx::BackupSession;
use nativemem_core::model::{Adapter, Kind, Observed, ScanReport};
use serde_json::{json, Value};

use crate::{lock_state, print_json, Agents, Blocked, Failure, Outcome, Roots, AGENT_IDS};

/// Exit code of `doctor` when an agent needs action.
const EXIT_ACTION_REQUIRED: u8 = 3;

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// One report per known agent; an agent without a home directory is not installed.
pub fn assess_all(agents: &Agents, state: &mut CompatState) -> Vec<AgentReport> {
    let now = now_ms();
    AGENT_IDS
        .iter()
        .map(|id| match agents.named(id) {
            Ok(Some(adapter)) => compat::assess(adapter, state, now),
            _ => AgentReport::not_installed(id),
        })
        .collect()
}

/// Adds the informational notices: skills whose own copy differs
/// from a copy the agent also reads elsewhere, and sessions other agents hold that Claude Code
/// would delete.
pub fn add_notices(
    agents: &Agents,
    reports: &mut [AgentReport],
    scans: Option<&BTreeMap<String, ScanReport>>,
) {
    for a in agents.all() {
        let shadowed = if let Some(scans) = scans {
            scans
                .get(a.id().as_str())
                .map(|s| s.shadowed_skills.clone())
                .unwrap_or_default()
        } else {
            a.skill_report().map(|r| r.shadowed).unwrap_or_default()
        };
        if let Some(r) = reports.iter_mut().find(|r| r.agent == a.id()) {
            r.notices.extend(shadowed.into_iter().map(|name| {
                format!("skill {name}: the copy in the shared skills directory has other content than this agent's own copy")
            }));
        }
    }
    let notice = if let Some(scans) = scans {
        session_notice(agents, scans.values().flat_map(|s| &s.observed))
    } else {
        session_notice(
            agents,
            agents
                .all()
                .iter()
                .filter(|a| a.id().as_str() != "claude")
                .filter_map(|a| a.scan().ok())
                .flatten(),
        )
    };
    if let Some(notice) = notice {
        if let Some(r) = reports.iter_mut().find(|r| r.agent.as_str() == "claude") {
            r.notices.push(notice);
        }
    }
}

/// A transcript-retention notice needs session observations, not another compatibility assessment.
pub fn session_notice<T: Borrow<Observed>>(
    agents: &Agents,
    observed: impl IntoIterator<Item = T>,
) -> Option<String> {
    let claude = agents.claude.as_ref()?;
    let retention = claude.retention();
    if retention.configured || retention.days == 0 {
        return None;
    }
    let held = observed.into_iter().any(|o| {
        let o = o.borrow();
        o.agent.as_str() != "claude"
            && o.item.kind == Kind::Session
            && o.replica_of.is_none()
            && !claude.accepts(&o.item)
    });
    held.then(|| format!(
        "transcripts older than {} days are deleted by Claude itself; set cleanupPeriodDays in Claude's settings to keep older sessions", retention.days
    ))
}

pub fn blocked_kinds(reports: &[AgentReport]) -> Blocked {
    reports
        .iter()
        .filter(|r| !r.paused_kinds().is_empty())
        .map(|r| (r.agent.to_string(), r.paused_kinds()))
        .collect()
}

/// `codex: sessions paused, run nativemem doctor`, one per affected agent.
pub fn paused_lines(reports: &[AgentReport]) -> Vec<String> {
    reports
        .iter()
        .filter(|r| !r.paused_kinds().is_empty())
        .map(|r| {
            format!(
                "{}: {} paused, run nativemem doctor",
                r.agent,
                kinds_text(&r.paused_kinds())
            )
        })
        .collect()
}

fn status_text(r: &AgentReport) -> String {
    match r.status {
        Status::Ok => "ok".into(),
        Status::Updated => format!("updated {}", r.version_text().unwrap_or_default())
            .trim_end()
            .into(),
        Status::ActionRequired => "action required".into(),
        Status::NotInstalled => "not installed".into(),
    }
}

/// One warning per installed agent that is not `Ok`, for `status`.
/// One informational line per notice, for `status`.
pub fn notice_lines(reports: &[AgentReport]) -> Vec<String> {
    reports
        .iter()
        .flat_map(|r| r.notices.iter().map(move |n| format!("{}: {n}", r.agent)))
        .collect()
}

pub fn warning_lines(reports: &[AgentReport]) -> Vec<String> {
    reports
        .iter()
        .filter(|r| matches!(r.status, Status::Updated | Status::ActionRequired))
        .map(|r| {
            let detail = match r.status {
                Status::ActionRequired => format!("{} paused", kinds_text(&r.paused_kinds())),
                _ => status_text(r),
            };
            format!("warning: {}: {detail}, run nativemem doctor", r.agent)
        })
        .collect()
}

/// The additive `compat` member of `status --json`.
pub fn summary_json(reports: &[AgentReport]) -> Value {
    let count = |s: Status| reports.iter().filter(|r| r.status == s).count();
    json!({
        "action_required": count(Status::ActionRequired),
        "updated": count(Status::Updated),
        "agents": reports
            .iter()
            .map(|r| json!({ "id": r.agent.as_str(), "status": r.status.as_str(), "notices": r.notices }))
            .collect::<Vec<_>>(),
    })
}

fn check_json(c: &Check) -> Value {
    json!({
        "id": c.id,
        "feature_kinds": c.feature_kinds.iter().map(Kind::as_str).collect::<Vec<_>>(),
        "summary": c.summary,
        "expected": c.expected,
        "observed": c.observed,
        "ok": c.ok,
        "accepted": c.accepted,
        "sources": c.sources,
    })
}

fn report_json(r: &AgentReport) -> Value {
    let mut doc = json!({
        "id": r.agent.as_str(),
        "installed": r.installed,
        "version": r.version,
        "previous_version": r.previous_version,
        "version_changed_ms": r.version_changed_ms,
        "status": r.status.as_str(),
        "paused": r.paused_kinds().iter().map(Kind::as_str).collect::<Vec<_>>(),
        "checks": r.checks.iter().map(check_json).collect::<Vec<_>>(),
        "notices": r.notices,
    });
    if r.status == Status::ActionRequired {
        doc["prompt"] = json!(compat::prompt(r, &compat::repo_location()));
    }
    doc
}

pub fn document(reports: &[AgentReport]) -> Value {
    json!({ "agents": reports.iter().map(report_json).collect::<Vec<_>>() })
}

fn print_report(r: &AgentReport) {
    let version = r.version.as_deref().unwrap_or("version unknown");
    if !r.installed {
        println!("{}: not installed", r.agent);
        return;
    }
    println!("{}: {version}  {}", r.agent, status_text(r));
    for c in r.checks.iter().filter(|c| c.accepted) {
        println!("  reviewed {}: {}", c.id, c.observed);
    }
    for c in r.failing() {
        println!("  FAIL {}  {}", c.id, c.summary);
        println!("    expected: {}", c.expected);
        println!("    observed: {}", c.observed);
    }
    for n in &r.notices {
        println!("  note: {n}");
    }
    if !r.paused_kinds().is_empty() {
        println!(
            "  paused: {} (nativemem doctor --prompt {} prints a prompt for a coding agent)",
            kinds_text(&r.paused_kinds()),
            r.agent
        );
    }
}

fn known(name: &str) -> Outcome<()> {
    if AGENT_IDS.contains(&name) {
        Ok(())
    } else {
        Err(Failure::UnknownAgent(name.to_string()))
    }
}

pub struct Args<'a> {
    pub agent: Option<&'a str>,
    pub accept: Option<&'a str>,
    pub prompt: Option<&'a str>,
    pub json: bool,
}

/// What `doctor` starts from: the state as loaded and as the probes updated it.
pub struct Context {
    pub loaded: CompatState,
    pub state: CompatState,
    pub reports: Vec<AgentReport>,
}

/// Writes `state` when it changed. A run that only observes (no `--accept`) skips the write
/// while another run holds the lock.
fn persist(roots: &Roots, backups: &BackupSession, ctx: &Context, locked: bool) -> Outcome<()> {
    if ctx.state == ctx.loaded {
        return Ok(());
    }
    let _guard = if locked {
        None
    } else {
        match lock_state(&roots.state) {
            Ok(file) => Some(file),
            Err(Failure::Coded { code: "busy", .. }) => return Ok(()),
            Err(e) => return Err(e),
        }
    };
    ctx.state.save(&roots.state, backups)?;
    Ok(())
}

/// Returns the exit code: 0, or 3 when an agent needs action.
pub fn run(
    roots: &Roots,
    backups: &BackupSession,
    agents: &Agents,
    mut ctx: Context,
    args: &Args,
) -> Outcome<u8> {
    for name in [args.agent, args.accept, args.prompt].into_iter().flatten() {
        known(name)?;
    }
    let mut accepted: Vec<(String, String)> = Vec::new();
    if let Some(name) = args.accept {
        let report = ctx.reports.iter().find(|r| r.agent.as_str() == name);
        if report.is_none_or(|r| !r.installed) {
            return Err(Failure::Coded {
                code: "not_installed",
                message: format!("{name} has no home directory, nothing to accept"),
            });
        }
        accepted = ctx.state.accept(report.expect("checked above"));
        ctx.reports = assess_all(agents, &mut ctx.state);
    }
    persist(roots, backups, &ctx, args.accept.is_some())?;
    add_notices(agents, &mut ctx.reports, None);

    let shown: Vec<&AgentReport> = ctx
        .reports
        .iter()
        .filter(|r| args.agent.is_none_or(|a| r.agent.as_str() == a))
        .collect();
    let code = if shown.iter().any(|r| r.status == Status::ActionRequired) {
        EXIT_ACTION_REQUIRED
    } else {
        0
    };
    if args.json {
        let mut doc = json!({ "agents": shown.iter().map(|r| report_json(r)).collect::<Vec<_>>() });
        if args.accept.is_some() {
            doc["accepted"] = accepted
                .iter()
                .map(|(check, observed)| {
                    json!({ "agent": args.accept, "check": check, "observed": observed })
                })
                .collect();
        }
        print_json(&doc);
        return Ok(code);
    }
    for (check, observed) in &accepted {
        println!("accepted {check}: {observed}");
    }
    if args.accept.is_some() && accepted.is_empty() {
        println!("nothing to accept for {}", args.accept.unwrap_or_default());
    }
    if let Some(name) = args.prompt {
        match ctx.reports.iter().find(|r| r.agent.as_str() == name) {
            Some(r) if r.status == Status::ActionRequired => {
                println!("{}", compat::prompt(r, &compat::repo_location()));
            }
            _ => println!("{name}: no failing checks, nothing to fix"),
        }
        return Ok(code);
    }
    for r in shown {
        print_report(r);
    }
    Ok(code)
}

/// Shows `notes` as macOS notifications, or appends them to the file named by
/// `NATIVEMEM_NOTIFY_FILE` when that is set. Failures to notify never fail a run.
pub fn deliver(notes: &[Notification]) {
    for n in notes {
        if let Some(file) = std::env::var_os("NATIVEMEM_NOTIFY_FILE").filter(|v| !v.is_empty()) {
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(Path::new(&file))
            {
                let _ = writeln!(f, "{}", n.text);
            }
            continue;
        }
        show(&n.text);
    }
}

#[cfg(target_os = "macos")]
fn show(text: &str) {
    let escaped = text.replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!("display notification \"{escaped}\" with title \"nativemem\"");
    let _ = std::process::Command::new("osascript")
        .args(["-e", &script])
        .output();
}

#[cfg(not(target_os = "macos"))]
fn show(_: &str) {}
