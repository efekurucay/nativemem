//! Compatibility watch. The agents update themselves while this tool depends on their on-disk
//! formats, so every adapter can describe what it observes (`Adapter::probes`) next to the
//! values it was written against. A failing check pauses the kinds it protects for that agent;
//! the state file remembers versions, reviewed overrides and what the user was last told.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde_json::{json, Map, Value};

use crate::error::{Error, Result};
use crate::fsx::{self, BackupSession};
use crate::model::{Adapter, AgentId, Kind};

const STATE_FILE: &str = "compat.json";
/// How long after a version change the status reads `Updated`.
const UPDATED_WINDOW_MS: i64 = 3 * 86_400_000;
const REPO_PLACEHOLDER: &str = "<path to the nativemem repo>";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ok,
    Updated,
    ActionRequired,
    NotInstalled,
}

impl Status {
    pub fn as_str(&self) -> &'static str {
        match self {
            Status::Ok => "ok",
            Status::Updated => "updated",
            Status::ActionRequired => "action_required",
            Status::NotInstalled => "not_installed",
        }
    }

    pub fn parse(s: &str) -> Option<Status> {
        match s {
            "ok" => Some(Status::Ok),
            "updated" => Some(Status::Updated),
            "action_required" => Some(Status::ActionRequired),
            "not_installed" => Some(Status::NotInstalled),
            _ => None,
        }
    }
}

/// One comparison between what an agent has on disk and what the adapter was written against.
/// `expected` and `observed` are structural descriptors only (key sets, column signatures,
/// type sets, small integers).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub id: String,
    /// The kinds that stop syncing for the agent while this check fails.
    pub feature_kinds: Vec<Kind>,
    pub summary: String,
    pub expected: String,
    pub observed: String,
    pub ok: bool,
    /// The user reviewed this observed value and chose to keep syncing.
    pub accepted: bool,
    /// Repo paths of the adapter files that implement what this check protects.
    pub sources: Vec<String>,
}

impl Check {
    pub fn new(id: &str, kinds: &[Kind], summary: &str, sources: &[&str]) -> Self {
        Check {
            id: id.to_string(),
            feature_kinds: kinds.to_vec(),
            summary: summary.to_string(),
            expected: String::new(),
            observed: String::new(),
            ok: true,
            accepted: false,
            sources: sources.iter().map(|s| s.to_string()).collect(),
        }
    }

    /// The outcome of the comparison.
    pub fn found(
        mut self,
        expected: impl Into<String>,
        observed: impl Into<String>,
        ok: bool,
    ) -> Self {
        self.expected = expected.into();
        self.observed = observed.into();
        self.ok = ok;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentReport {
    pub agent: AgentId,
    pub installed: bool,
    pub version: Option<String>,
    pub previous_version: Option<String>,
    pub version_changed_ms: Option<i64>,
    pub checks: Vec<Check>,
    pub status: Status,
}

impl AgentReport {
    pub fn not_installed(agent: &str) -> Self {
        AgentReport {
            agent: AgentId::new(agent),
            installed: false,
            version: None,
            previous_version: None,
            version_changed_ms: None,
            checks: Vec::new(),
            status: Status::NotInstalled,
        }
    }

    pub fn failing(&self) -> Vec<&Check> {
        self.checks.iter().filter(|c| !c.ok).collect()
    }

    /// The kinds that failing checks stop for this agent, sorted.
    pub fn paused_kinds(&self) -> Vec<Kind> {
        let set: BTreeSet<Kind> = self
            .failing()
            .iter()
            .flat_map(|c| c.feature_kinds.iter().copied())
            .collect();
        set.into_iter().collect()
    }

    /// `0.167.0 → 0.170.0` when the version changed, else the current version.
    pub fn version_text(&self) -> Option<String> {
        match (&self.previous_version, &self.version) {
            (Some(old), Some(new)) if old != new => Some(format!("{old} → {new}")),
            (_, Some(v)) => Some(v.clone()),
            _ => None,
        }
    }
}

pub fn display_name(agent: &str) -> String {
    match agent {
        "claude" => "Claude Code",
        "codex" => "Codex",
        "opencode" => "OpenCode",
        "pi" => "Pi",
        other => other,
    }
    .to_string()
}

fn wiki_page(agent: &str) -> &str {
    match agent {
        "claude" => "claude-code",
        other => other,
    }
}

/// `memory, sessions` for the kinds a run leaves alone.
pub fn kinds_text(kinds: &[Kind]) -> String {
    kinds
        .iter()
        .map(|k| match k {
            Kind::Memory => "memory",
            Kind::Instruction => "instructions",
            Kind::Session => "sessions",
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// What is remembered about one agent between runs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentState {
    pub version: Option<String>,
    pub previous_version: Option<String>,
    pub changed_ms: Option<i64>,
    /// Check id to the observed value the user reviewed.
    pub accepted: BTreeMap<String, String>,
    pub notified: Option<Status>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CompatState {
    pub agents: BTreeMap<String, AgentState>,
}

impl CompatState {
    /// An absent or unreadable file is an empty state.
    pub fn load(state_dir: &Path) -> Self {
        std::fs::read_to_string(state_dir.join(STATE_FILE))
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .map(|v| Self::from_json(&v))
            .unwrap_or_default()
    }

    /// Writes the file atomically after backing the previous one up; a state that did not change
    /// leaves the file alone. Returns whether the file was written.
    pub fn save(&self, state_dir: &Path, backups: &BackupSession) -> Result<bool> {
        let path = state_dir.join(STATE_FILE);
        let text = format!(
            "{}\n",
            serde_json::to_string_pretty(&self.to_json()).unwrap()
        );
        match std::fs::read_to_string(&path) {
            Ok(current) if current == text => return Ok(false),
            Ok(_) => {
                fsx::backup(
                    backups,
                    state_dir,
                    &AgentId::new("nativemem"),
                    state_dir,
                    &path,
                )?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(Error::io(&path, e)),
        }
        fsx::atomic_write(&path, text.as_bytes())?;
        Ok(true)
    }

    fn to_json(&self) -> Value {
        let mut agents = Map::new();
        for (id, s) in &self.agents {
            agents.insert(
                id.clone(),
                json!({
                    "version": s.version,
                    "previous_version": s.previous_version,
                    "changed_ms": s.changed_ms,
                    "accepted": s.accepted,
                    "notified": s.notified.map(|n| n.as_str()),
                }),
            );
        }
        json!({ "agents": agents })
    }

    fn from_json(v: &Value) -> Self {
        let text = |o: &Value, k: &str| o.get(k).and_then(Value::as_str).map(str::to_string);
        let mut agents = BTreeMap::new();
        for (id, a) in v
            .get("agents")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            agents.insert(
                id.clone(),
                AgentState {
                    version: text(a, "version"),
                    previous_version: text(a, "previous_version"),
                    changed_ms: a.get("changed_ms").and_then(Value::as_i64),
                    accepted: a
                        .get("accepted")
                        .and_then(Value::as_object)
                        .into_iter()
                        .flatten()
                        .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string())))
                        .collect(),
                    notified: text(a, "notified").as_deref().and_then(Status::parse),
                },
            );
        }
        CompatState { agents }
    }

    /// Records the observed values of the failing checks of `report` as reviewed. Returns the
    /// `(check id, observed)` pairs recorded.
    pub fn accept(&mut self, report: &AgentReport) -> Vec<(String, String)> {
        let state = self.agents.entry(report.agent.to_string()).or_default();
        let mut recorded = Vec::new();
        for c in report.failing() {
            state.accepted.insert(c.id.clone(), c.observed.clone());
            recorded.push((c.id.clone(), c.observed.clone()));
        }
        recorded
    }
}

/// Runs the probes of `adapter` against `state`: tracks the version, applies reviewed overrides
/// and derives the status. Updates `state` in memory only.
pub fn assess(adapter: &dyn Adapter, state: &mut CompatState, now_ms: i64) -> AgentReport {
    let id = adapter.id();
    let remembered = state.agents.entry(id.to_string()).or_default();
    let version = adapter.version();
    if let Some(seen) = &version {
        if remembered.version.as_ref() != Some(seen) {
            if let Some(old) = remembered.version.take() {
                remembered.previous_version = Some(old);
                remembered.changed_ms = Some(now_ms);
            }
            remembered.version = Some(seen.clone());
        }
    }
    let mut checks = adapter.probes();
    for c in &mut checks {
        if !c.ok && remembered.accepted.get(&c.id) == Some(&c.observed) {
            c.ok = true;
            c.accepted = true;
        }
    }
    let changed_recently = remembered
        .changed_ms
        .is_some_and(|t| now_ms - t < UPDATED_WINDOW_MS);
    let status = if checks.iter().any(|c| !c.ok) {
        Status::ActionRequired
    } else if changed_recently {
        Status::Updated
    } else {
        Status::Ok
    };
    AgentReport {
        agent: id,
        installed: true,
        version: remembered.version.clone(),
        previous_version: remembered.previous_version.clone(),
        version_changed_ms: remembered.changed_ms,
        checks,
        status,
    }
}

/// A short desktop notification: the agent and its status, never content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    pub agent: AgentId,
    pub text: String,
}

fn notification_text(report: &AgentReport) -> Option<String> {
    let name = display_name(report.agent.as_str());
    match report.status {
        Status::ActionRequired => Some(format!("{name} needs attention, run nativemem doctor")),
        Status::Updated => Some(match &report.version {
            Some(v) => format!("{name} updated to {v}, all checks pass"),
            None => format!("{name} updated, all checks pass"),
        }),
        Status::Ok | Status::NotInstalled => None,
    }
}

/// The notifications owed for `reports`: one per agent whose status differs from the last one
/// the user was told about and is worth a message. Records every status change in `state`.
pub fn pending_notifications(
    reports: &[AgentReport],
    state: &mut CompatState,
) -> Vec<Notification> {
    let mut out = Vec::new();
    for r in reports.iter().filter(|r| r.installed) {
        let remembered = state.agents.entry(r.agent.to_string()).or_default();
        if remembered.notified.unwrap_or(Status::Ok) == r.status {
            continue;
        }
        remembered.notified = Some(r.status);
        if let Some(text) = notification_text(r) {
            out.push(Notification {
                agent: r.agent.clone(),
                text,
            });
        }
    }
    out
}

/// Where the nativemem repo is: `NATIVEMEM_REPO`, else the directory this binary was built from
/// while it still exists, else a placeholder.
pub fn repo_location() -> String {
    if let Some(dir) = std::env::var_os("NATIVEMEM_REPO").filter(|v| !v.is_empty()) {
        return dir.to_string_lossy().into_owned();
    }
    let built_from = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent);
    match built_from {
        Some(dir) if dir.join("Cargo.toml").is_file() => dir.display().to_string(),
        _ => REPO_PLACEHOLDER.to_string(),
    }
}

/// A prompt for a coding agent working in the nativemem repo at `repo`, to bring the adapter of
/// `report`'s agent back in line with the agent's current formats.
pub fn prompt(report: &AgentReport, repo: &str) -> String {
    let id = report.agent.as_str();
    let name = display_name(id);
    let change = match report.version_text() {
        Some(v) if report.previous_version.is_some() => format!("{name} updated ({v})"),
        Some(v) => format!("{name} is at version {v}"),
        None => format!("{name} changed (its version is unknown)"),
    };
    let mut out = format!(
        "You are working in the nativemem repository at {repo}. {change}, and nativemem's \
         compatibility watch found that formats the {id} adapter depends on no longer match. \
         Syncing of the affected kinds is paused until the adapter is brought up to date.\n\n"
    );
    out.push_str("Failing checks:\n");
    let mut sources: BTreeSet<&str> = BTreeSet::new();
    for c in report.failing() {
        let kinds: Vec<&str> = c.feature_kinds.iter().map(|k| k.as_str()).collect();
        out.push_str(&format!(
            "- {}: {}\n  expected: {}\n  observed: {}\n  pauses: {}\n",
            c.id,
            c.summary,
            c.expected,
            c.observed,
            kinds.join(", ")
        ));
        sources.extend(c.sources.iter().map(String::as_str));
    }
    out.push_str(&format!(
        "\nKinds paused for {id}: {}.\n",
        kinds_text(&report.paused_kinds())
    ));
    out.push_str("\nRead these adapter files first:\n");
    for s in &sources {
        out.push_str(&format!("- {s}\n"));
    }
    out.push_str(&format!(
        "\nThen read wiki/agents/{}.md, wiki/guides/add-an-agent.md and \
         wiki/architecture/overview.md.\n",
        wiki_page(id)
    ));
    out.push_str(
        "\nHard rules:\n\
         - Never run the nativemem binary, the tests or the desktop app against the real agent \
         homes (~/.claude, ~/.codex, ~/.pi, ~/.config/opencode) or ~/.nativemem. Use temp \
         directories and explicit roots; CLI tests redirect HOME.\n\
         - Never put secrets or message text in output, tests or docs.\n\
         - Code, comments and docs describe the current state only; history lives in git.\n\
         - Update the wiki in the same task as the code.\n\
         - Commits carry no AI attribution.\n",
    );
    out.push_str(&format!(
        "\nTask: inspect the new on-disk format read-only, update the expected constants in the \
         probes, the adapter code, the tests and the wiki together, so that the adapter reads and \
         writes the new format correctly. Then run:\n\
         - cargo test --workspace\n\
         - cargo clippy --workspace --all-targets -- -D warnings\n\
         - cargo fmt --all -- --check\n\
         - swift test (in apps/macos)\n\
         Finish by telling the user to run `nativemem doctor --agent {id}` and confirm the \
         checks pass.\n"
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Result as CoreResult;
    use crate::model::{Item, ItemKey, Observed};
    use std::path::PathBuf;

    struct Stub {
        version: Option<String>,
        checks: Vec<Check>,
    }

    impl Adapter for Stub {
        fn id(&self) -> AgentId {
            AgentId::new("codex")
        }
        fn version(&self) -> Option<String> {
            self.version.clone()
        }
        fn probes(&self) -> Vec<Check> {
            self.checks.clone()
        }
        fn scan(&self) -> CoreResult<Vec<Observed>> {
            Ok(Vec::new())
        }
        fn write_replica(&self, _: &ItemKey, _: &Item) -> CoreResult<(String, PathBuf)> {
            unreachable!()
        }
        fn write_original(&self, _: &str, _: &Item) -> CoreResult<()> {
            unreachable!()
        }
        fn remove(&self, _: &str) -> CoreResult<()> {
            unreachable!()
        }
    }

    fn check(ok: bool) -> Check {
        Check::new(
            "codex.x",
            &[Kind::Session],
            "x",
            &["crates/core/src/adapters/codex.rs"],
        )
        .found("a", if ok { "a" } else { "b" }, ok)
    }

    fn stub(version: &str, ok: bool) -> Stub {
        Stub {
            version: Some(version.into()),
            checks: vec![check(ok)],
        }
    }

    const DAY: i64 = 86_400_000;

    #[test]
    fn first_sight_is_ok_a_later_change_is_updated_for_three_days() {
        let mut st = CompatState::default();
        assert_eq!(assess(&stub("1", true), &mut st, 0).status, Status::Ok);
        let r = assess(&stub("2", true), &mut st, DAY);
        assert_eq!(r.status, Status::Updated);
        assert_eq!(r.version_text().as_deref(), Some("1 → 2"));
        assert_eq!(
            assess(&stub("2", true), &mut st, 3 * DAY).status,
            Status::Updated
        );
        assert_eq!(
            assess(&stub("2", true), &mut st, 4 * DAY + 1).status,
            Status::Ok
        );
    }

    #[test]
    fn an_unknown_version_keeps_the_remembered_one() {
        let mut st = CompatState::default();
        assess(&stub("1", true), &mut st, 0);
        let none = Stub {
            version: None,
            checks: vec![],
        };
        let r = assess(&none, &mut st, DAY);
        assert_eq!(r.version.as_deref(), Some("1"));
        assert_eq!(r.status, Status::Ok);
    }

    #[test]
    fn a_failing_check_needs_action_until_its_observed_value_is_accepted() {
        let mut st = CompatState::default();
        let r = assess(&stub("1", false), &mut st, 0);
        assert_eq!(r.status, Status::ActionRequired);
        assert_eq!(r.paused_kinds(), vec![Kind::Session]);
        assert_eq!(
            st.accept(&r),
            vec![("codex.x".to_string(), "b".to_string())]
        );
        let r = assess(&stub("1", false), &mut st, 0);
        assert_eq!(r.status, Status::Ok);
        assert!(r.checks[0].accepted && r.paused_kinds().is_empty());
        let mut other = stub("1", false);
        other.checks[0].observed = "c".into();
        assert_eq!(assess(&other, &mut st, 0).status, Status::ActionRequired);
    }

    #[test]
    fn state_round_trips_through_the_file_with_a_backup_on_change() {
        let t = tempfile::tempdir().unwrap();
        let mut st = CompatState::default();
        let r = assess(&stub("1", false), &mut st, 5);
        st.accept(&r);
        pending_notifications(&[r], &mut st);
        assert!(st.save(t.path(), &BackupSession::with_stamp("T1")).unwrap());
        assert_eq!(CompatState::load(t.path()), st);
        assert!(!st.save(t.path(), &BackupSession::with_stamp("T2")).unwrap());
        assess(&stub("2", true), &mut st, 9);
        assert!(st.save(t.path(), &BackupSession::with_stamp("T3")).unwrap());
        assert!(t.path().join("backups/T3/nativemem/compat.json").is_file());
        assert!(CompatState::load(Path::new("/nonexistent"))
            .agents
            .is_empty());
    }

    #[test]
    fn notifications_fire_once_per_status_change_and_name_no_content() {
        let mut st = CompatState::default();
        let bad = assess(&stub("1", false), &mut st, 0);
        let n = pending_notifications(std::slice::from_ref(&bad), &mut st);
        assert_eq!(n.len(), 1);
        assert_eq!(n[0].text, "Codex needs attention, run nativemem doctor");
        assert!(pending_notifications(&[bad], &mut st).is_empty());
        let updated = assess(&stub("2", true), &mut st, DAY);
        let n = pending_notifications(&[updated], &mut st);
        assert_eq!(n[0].text, "Codex updated to 2, all checks pass");
        let ok = assess(&stub("2", true), &mut st, 9 * DAY);
        assert!(pending_notifications(&[ok], &mut st).is_empty());
        assert_eq!(st.agents["codex"].notified, Some(Status::Ok));
    }

    #[test]
    fn prompt_names_the_check_sources_wiki_pages_and_rules() {
        let mut st = CompatState::default();
        assess(&stub("1", true), &mut st, 0);
        let r = assess(&stub("2", false), &mut st, DAY);
        let p = prompt(&r, "/Users/x/nativemem");
        for needle in [
            "/Users/x/nativemem",
            "Codex updated (1 → 2)",
            "codex.x",
            "expected: a",
            "observed: b",
            "Kinds paused for codex: sessions",
            "crates/core/src/adapters/codex.rs",
            "wiki/agents/codex.md",
            "wiki/guides/add-an-agent.md",
            "wiki/architecture/overview.md",
            "Never run the nativemem binary",
            "no ai attribution",
            "cargo test --workspace",
            "cargo clippy --workspace --all-targets -- -D warnings",
            "cargo fmt --all -- --check",
            "swift test",
            "nativemem doctor --agent codex",
        ] {
            assert!(
                p.to_lowercase().contains(&needle.to_lowercase()),
                "{needle}"
            );
        }
    }

    #[test]
    fn repo_location_prefers_the_environment() {
        // The variable is process-wide, so this test is the only one that touches it.
        std::env::set_var("NATIVEMEM_REPO", "/Users/x/repo");
        assert_eq!(repo_location(), "/Users/x/repo");
        std::env::remove_var("NATIVEMEM_REPO");
        assert!(!repo_location().is_empty());
    }
}
