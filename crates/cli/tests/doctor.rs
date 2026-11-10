//! `nativemem doctor`, feature gating and notifications, on fixtures in temp directories. HOME is
//! redirected, every root is explicit and PATH points at an empty directory, so no real agent
//! home or installation is read.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use nativemem_core::testkit::{ClaudeHome, CodexHome, FrontmatterVariant, PiHome};
use serde_json::Value;

const ID: &str = "01a1c682-fd13-7263-8871-4ca47590d7d1";

struct Fx {
    _t: tempfile::TempDir,
    root: PathBuf,
    claude: PathBuf,
    codex: PathBuf,
    pi: PathBuf,
    state: PathBuf,
    claude_session: PathBuf,
}

impl Fx {
    fn new() -> Fx {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().to_path_buf();
        let claude_home = ClaudeHome::new(&root.join("claude"));
        let cwd = Path::new("/w/app");
        let claude_session =
            claude_home.add_session(cwd, "s1", Some("t"), &[("user", "hi"), ("assistant", "yo")]);
        let project = claude_home.add_project(cwd);
        claude_home.add_memory(
            &project,
            "a.md",
            FrontmatterVariant::Nested,
            "alpha",
            "about alpha",
            "user",
            "alpha body",
        );
        let codex_home = CodexHome::new(&root.join("codex"));
        codex_home.write_memory_md(
            "# Task Group: Fix Login\napplies_to: cwd=/w/app; reuse_rule=x\nnotes\n",
        );
        codex_home.write_state_db(5, &[]);
        codex_home.write_version("0.167.0");
        let pi_home = PiHome::new(&root.join("pi"));
        pi_home.add_session(
            Path::new("/w/other"),
            ID,
            Some("pi"),
            &[("user", "q"), ("assistant", "a")],
        );
        fs::create_dir_all(root.join("opencode")).unwrap();
        fs::create_dir_all(root.join("bin")).unwrap();
        Fx {
            claude: root.join("claude"),
            codex: root.join("codex"),
            pi: root.join("pi"),
            state: root.join("state"),
            claude_session,
            root,
            _t: t,
        }
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_nativemem"))
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("CODEX_HOME")
            .env_remove("OPENCODE_CONFIG_DIR")
            .env_remove("PI_CODING_AGENT_DIR")
            .env_remove("NATIVEMEM_CLAUDE_HOME")
            .env_remove("NATIVEMEM_CODEX_HOME")
            .env_remove("NATIVEMEM_OPENCODE_HOME")
            .env_remove("NATIVEMEM_PI_HOME")
            .env_remove("NATIVEMEM_STATE_DIR")
            .env("HOME", self.root.join("home"))
            .env("PATH", self.root.join("bin"))
            .env("NATIVEMEM_REPO", "/Users/x/nativemem")
            .env("NATIVEMEM_NOTIFY_FILE", self.root.join("notifications.txt"))
            .env("NATIVEMEM_CODEX_SESSION_WRITES", "on")
            .arg("--claude-home")
            .arg(&self.claude)
            .arg("--codex-home")
            .arg(&self.codex)
            .arg("--opencode-home")
            .arg(self.root.join("opencode"))
            .arg("--pi-home")
            .arg(&self.pi)
            .arg("--state-dir")
            .arg(&self.state)
            .args(args)
            .output()
            .unwrap()
    }

    fn json(&self, args: &[&str]) -> (Option<i32>, Value) {
        let o = self.run(args);
        let doc = serde_json::from_slice(&o.stdout).unwrap_or_else(|e| panic!("{e}: {}", text(&o)));
        (o.status.code(), doc)
    }

    fn notifications(&self) -> String {
        fs::read_to_string(self.root.join("notifications.txt")).unwrap_or_default()
    }

    fn compat_file(&self) -> Value {
        serde_json::from_str(&fs::read_to_string(self.state.join("compat.json")).unwrap()).unwrap()
    }

    /// Makes Codex's `threads` table differ from what the adapter expects.
    fn drift_codex(&self) {
        fs::remove_file(self.codex.join("state_5.sqlite")).unwrap();
        CodexHome::new(&self.codex).write_state_db(5, &["brand_new INTEGER"]);
    }

    fn drift_claude_line_type(&self) {
        let mut f = fs::OpenOptions::new()
            .append(true)
            .open(&self.claude_session)
            .unwrap();
        writeln!(f, r#"{{"type":"brand-new-line"}}"#).unwrap();
    }

    fn drift_claude_index(&self) {
        let index = self.claude.join("projects/-w-app/memory/MEMORY.md");
        let mut f = fs::OpenOptions::new().append(true).open(index).unwrap();
        writeln!(f, "- [x](x.md) - not the written shape").unwrap();
    }
}

fn text(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn agent<'a>(doc: &'a Value, id: &str) -> &'a Value {
    doc["agents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["id"] == id)
        .unwrap_or_else(|| panic!("no {id} in {doc}"))
}

/// `(agent, op)` of every action that targets an agent, from a `sync --json` document.
fn targets(doc: &Value) -> Vec<(String, String)> {
    doc["actions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| {
            (
                a["agent"].as_str().unwrap().into(),
                a["name"].as_str().unwrap().into(),
            )
        })
        .collect()
}

#[test]
fn doctor_passes_on_the_fixtures_and_writes_only_its_state_file() {
    let f = Fx::new();
    let db = fs::read(f.codex.join("state_5.sqlite")).unwrap();
    let (code, doc) = f.json(&["doctor", "--json"]);
    assert_eq!(code, Some(0), "{doc}");
    let ids: Vec<&str> = doc["agents"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["claude", "codex", "opencode", "pi"]);
    for a in doc["agents"].as_array().unwrap() {
        assert_eq!(a["status"], "ok", "{a}");
        assert!(a.get("prompt").is_none());
        assert_eq!(a["paused"], serde_json::json!([]));
    }
    assert_eq!(agent(&doc, "claude")["checks"].as_array().unwrap().len(), 5);
    assert_eq!(agent(&doc, "codex")["version"], "0.167.0");
    assert_eq!(agent(&doc, "pi")["version"], Value::Null);
    let check = &agent(&doc, "codex")["checks"][1];
    assert_eq!(check["id"], "codex.threads_columns");
    assert_eq!(check["feature_kinds"], serde_json::json!(["session"]));
    for key in [
        "summary", "expected", "observed", "ok", "accepted", "sources",
    ] {
        assert!(check.get(key).is_some(), "{key}");
    }
    assert_eq!(db, fs::read(f.codex.join("state_5.sqlite")).unwrap());
    assert!(f.compat_file()["agents"]["codex"]["version"] == "0.167.0");
    let mut names: Vec<String> = fs::read_dir(&f.state)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names, ["compat.json", "sync.lock"]);

    let o = f.run(&["doctor"]);
    assert_eq!(o.status.code(), Some(0));
    let out = text(&o);
    assert!(out.contains("claude: version unknown  ok"), "{out}");
    assert!(out.contains("codex: 0.167.0  ok"), "{out}");
    assert!(out.contains("pi: version unknown  ok"), "{out}");
}

#[test]
fn doctor_on_an_agent_without_a_home_reports_not_installed() {
    let f = Fx::new();
    fs::remove_dir_all(&f.pi).unwrap();
    let (code, doc) = f.json(&["doctor", "--json", "--agent", "pi"]);
    assert_eq!(code, Some(0));
    assert_eq!(doc["agents"].as_array().unwrap().len(), 1);
    assert_eq!(agent(&doc, "pi")["status"], "not_installed");
    assert_eq!(agent(&doc, "pi")["installed"], false);
    assert!(text(&f.run(&["doctor"])).contains("pi: not installed"));
}

#[test]
fn structural_drift_needs_action_and_carries_a_prompt_with_the_failing_check() {
    let f = Fx::new();
    f.drift_claude_line_type();
    f.drift_codex();
    let pi_file = fs::read_dir(f.pi.join("sessions/--w-other--"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let t = fs::read_to_string(&pi_file).unwrap();
    fs::write(&pi_file, t.replace("\"version\":3", "\"version\":4")).unwrap();

    let (code, doc) = f.json(&["doctor", "--json"]);
    assert_eq!(code, Some(3));
    for (id, check, kinds) in [
        ("claude", "claude.line_types", "session"),
        ("codex", "codex.threads_columns", "session"),
        ("pi", "pi.session_header", "session"),
    ] {
        let a = agent(&doc, id);
        assert_eq!(a["status"], "action_required", "{a}");
        assert_eq!(a["paused"], serde_json::json!([kinds]));
        let prompt = a["prompt"].as_str().unwrap();
        assert!(prompt.contains(check), "{prompt}");
        assert!(prompt.contains("/Users/x/nativemem"));
        assert!(prompt.contains(&format!(
            "wiki/agents/{}.md",
            if id == "claude" { "claude-code" } else { id }
        )));
        let failing: Vec<&Value> = a["checks"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|c| c["ok"] == false)
            .collect();
        assert_eq!(failing.len(), 1);
        assert_eq!(failing[0]["id"], check);
    }
    assert_eq!(agent(&doc, "opencode")["status"], "ok");

    let o = f.run(&["doctor", "--agent", "codex"]);
    assert_eq!(o.status.code(), Some(3));
    let out = text(&o);
    assert!(out.contains("FAIL codex.threads_columns"), "{out}");
    assert!(out.contains("added: brand_new"), "{out}");
    assert!(out.contains("paused: sessions"), "{out}");
    assert!(!out.contains("claude"));

    let o = f.run(&["doctor", "--prompt", "codex"]);
    assert_eq!(o.status.code(), Some(3));
    let out = text(&o);
    assert!(
        out.contains("codex.threads_columns") && out.contains("cargo test --workspace"),
        "{out}"
    );
    let o = f.run(&["doctor", "--prompt", "opencode"]);
    assert!(text(&o).contains("nothing to fix"));
}

#[test]
fn sync_pauses_exactly_the_protected_kinds_and_keeps_everything_else_syncing() {
    let f = Fx::new();
    let (_, base) = f.json(&["sync", "--json"]);
    let base = targets(&base);
    for expected in [
        ("codex", "alpha"),
        ("claude", "fix-login"),
        ("pi", "alpha"),
        ("claude", ID),
        ("codex", ID),
        ("pi", "s1"),
        ("codex", "s1"),
    ] {
        assert!(
            base.contains(&(expected.0.into(), expected.1.into())),
            "{expected:?} in {base:?}"
        );
    }

    f.drift_codex();
    let o = f.run(&["sync"]);
    let out = text(&o);
    assert!(
        out.contains("codex: sessions paused, run nativemem doctor"),
        "{out}"
    );
    let (_, doc) = f.json(&["sync", "--json"]);
    let got = targets(&doc);
    assert!(!got.contains(&("codex".into(), ID.into())), "{got:?}");
    assert!(!got.contains(&("codex".into(), "s1".into())), "{got:?}");
    for kept in [
        ("codex", "alpha"),
        ("claude", "fix-login"),
        ("claude", ID),
        ("pi", "s1"),
        ("pi", "alpha"),
    ] {
        assert!(
            got.contains(&(kept.0.into(), kept.1.into())),
            "{kept:?} in {got:?}"
        );
    }
    assert_eq!(f.run(&["sync", "--json"]).status.code(), Some(0));
}

#[test]
fn a_paused_memory_kind_leaves_the_agent_alone_in_both_directions() {
    let f = Fx::new();
    f.drift_claude_index();
    let out = text(&f.run(&["sync"]));
    assert!(
        out.contains("claude: memory paused, run nativemem doctor"),
        "{out}"
    );
    let (_, doc) = f.json(&["sync", "--json"]);
    let got = targets(&doc);
    assert!(
        !got.iter().any(|(a, n)| a == "claude" && n == "fix-login"),
        "{got:?}"
    );
    assert!(
        !got.iter().any(|(_, n)| n == "alpha"),
        "claude's own memory stays put: {got:?}"
    );
    assert!(
        got.contains(&("claude".into(), ID.into())),
        "sessions still reach claude: {got:?}"
    );
    assert!(
        got.contains(&("pi".into(), "s1".into())),
        "other agents keep syncing: {got:?}"
    );

    let before = fs::read_to_string(f.claude.join("projects/-w-app/memory/MEMORY.md")).unwrap();
    assert_eq!(f.run(&["sync", "--apply"]).status.code(), Some(0));
    assert_eq!(
        before,
        fs::read_to_string(f.claude.join("projects/-w-app/memory/MEMORY.md")).unwrap()
    );
    assert!(!f
        .claude
        .join("projects/-w-app/memory/fix-login.md")
        .exists());
}

#[test]
fn accept_records_the_observed_value_and_clears_the_pause() {
    let f = Fx::new();
    f.drift_codex();
    let db = fs::read(f.codex.join("state_5.sqlite")).unwrap();
    assert_eq!(f.run(&["doctor"]).status.code(), Some(3));

    let o = f.run(&["doctor", "--accept", "codex"]);
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    let out = text(&o);
    assert!(
        out.contains("accepted codex.threads_columns: 44 columns"),
        "{out}"
    );
    assert!(out.contains("reviewed codex.threads_columns"), "{out}");
    assert_eq!(db, fs::read(f.codex.join("state_5.sqlite")).unwrap());

    let (code, doc) = f.json(&["doctor", "--json"]);
    assert_eq!(code, Some(0));
    assert_eq!(agent(&doc, "codex")["status"], "ok");
    assert_eq!(agent(&doc, "codex")["checks"][1]["accepted"], true);
    assert!(!text(&f.run(&["sync"])).contains("paused"));
    let accepted = &f.compat_file()["agents"]["codex"]["accepted"];
    assert!(accepted["codex.threads_columns"]
        .as_str()
        .unwrap()
        .contains("brand_new"));

    // A further change is a new observation and needs a new review.
    fs::remove_file(f.codex.join("state_5.sqlite")).unwrap();
    CodexHome::new(&f.codex).write_state_db(5, &["another INTEGER"]);
    assert_eq!(f.run(&["doctor"]).status.code(), Some(3));

    let (code, doc) = f.json(&["doctor", "--accept", "codex", "--json"]);
    assert_eq!(code, Some(0));
    assert_eq!(doc["accepted"][0]["agent"], "codex");
    assert_eq!(doc["accepted"][0]["check"], "codex.threads_columns");
}

#[test]
fn accept_needs_an_installed_known_agent() {
    let f = Fx::new();
    let (code, doc) = f.json(&["doctor", "--accept", "gemini", "--json"]);
    assert_eq!(
        (code, doc["code"].as_str()),
        (Some(1), Some("unknown_agent"))
    );
    fs::remove_dir_all(&f.pi).unwrap();
    let (code, doc) = f.json(&["doctor", "--accept", "pi", "--json"]);
    assert_eq!(
        (code, doc["code"].as_str()),
        (Some(1), Some("not_installed"))
    );
    assert_eq!(f.run(&["doctor", "--accept"]).status.code(), Some(2));
}

#[test]
fn a_version_change_alone_is_updated_and_persists() {
    let f = Fx::new();
    assert_eq!(f.run(&["doctor"]).status.code(), Some(0));
    assert!(!text(&f.run(&["status"])).contains("warning"));

    CodexHome::new(&f.codex).write_version("0.170.0");
    let (code, doc) = f.json(&["doctor", "--json", "--agent", "codex"]);
    assert_eq!(code, Some(0));
    let a = agent(&doc, "codex");
    assert_eq!(
        (
            a["status"].as_str(),
            a["version"].as_str(),
            a["previous_version"].as_str()
        ),
        (Some("updated"), Some("0.170.0"), Some("0.167.0"))
    );
    assert!(a.get("prompt").is_none());
    let saved = &f.compat_file()["agents"]["codex"];
    assert_eq!(saved["version"], "0.170.0");
    assert_eq!(saved["previous_version"], "0.167.0");
    assert!(saved["changed_ms"].as_i64().unwrap() > 0);

    let out = text(&f.run(&["doctor"]));
    assert!(
        out.contains("codex: 0.170.0  updated 0.167.0 → 0.170.0"),
        "{out}"
    );
    let out = text(&f.run(&["status"]));
    assert!(
        out.contains("warning: codex: updated 0.167.0 → 0.170.0, run nativemem doctor"),
        "{out}"
    );
    let (_, status) = f.json(&["status", "--json"]);
    assert_eq!(status["compat"]["updated"], 1);
    assert_eq!(status["compat"]["agents"][1]["status"], "updated");
    assert!(!text(&f.run(&["sync"])).contains("paused"));
}

#[test]
fn status_warns_once_per_agent_that_needs_action() {
    let f = Fx::new();
    f.drift_codex();
    f.drift_claude_line_type();
    let out = text(&f.run(&["status"]));
    assert!(
        out.contains("warning: claude: sessions paused, run nativemem doctor"),
        "{out}"
    );
    assert!(
        out.contains("warning: codex: sessions paused, run nativemem doctor"),
        "{out}"
    );
    assert_eq!(out.matches("warning:").count(), 2);
    let (_, status) = f.json(&["status", "--json"]);
    assert_eq!(status["compat"]["action_required"], 2);
    assert!(!f.state.exists(), "status never writes");
}

#[test]
fn notify_fires_once_per_status_change_and_records_it() {
    let f = Fx::new();
    f.drift_codex();
    assert_eq!(f.run(&["sync", "--apply"]).status.code(), Some(0));
    assert_eq!(f.notifications(), "", "no --notify, no notification");
    assert!(f.compat_file()["agents"]["codex"]["notified"].is_null());

    assert_eq!(
        f.run(&["sync", "--apply", "--notify"]).status.code(),
        Some(0)
    );
    assert_eq!(
        f.notifications(),
        "Codex needs attention, run nativemem doctor\n"
    );
    assert_eq!(
        f.compat_file()["agents"]["codex"]["notified"],
        "action_required"
    );
    f.run(&["sync", "--apply", "--notify"]);
    assert_eq!(f.notifications().lines().count(), 1);

    f.run(&["doctor", "--accept", "codex"]);
    f.run(&["sync", "--apply", "--notify"]);
    assert_eq!(f.notifications().lines().count(), 1);
    assert_eq!(f.compat_file()["agents"]["codex"]["notified"], "ok");

    CodexHome::new(&f.codex).write_version("0.170.0");
    f.run(&["sync", "--apply", "--notify"]);
    assert_eq!(
        f.notifications(),
        "Codex needs attention, run nativemem doctor\nCodex updated to 0.170.0, all checks pass\n"
    );
    assert_eq!(f.compat_file()["agents"]["codex"]["notified"], "updated");
}

#[test]
fn notify_requires_apply() {
    let f = Fx::new();
    assert_eq!(f.run(&["sync", "--notify"]).status.code(), Some(2));
}

fn files_under(dir: &Path) -> std::collections::BTreeMap<PathBuf, Vec<u8>> {
    let mut out = std::collections::BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                let bytes = fs::read(&p).unwrap();
                out.insert(p, bytes);
            }
        }
    }
    out
}

fn rollouts(codex: &Path) -> usize {
    files_under(&codex.join("sessions"))
        .keys()
        .filter(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("rollout-"))
        })
        .count()
}

#[test]
fn paused_claude_sessions_are_not_read_so_they_do_not_replicate_out() {
    let f = Fx::new();
    f.drift_claude_line_type();
    let (_, doc) = f.json(&["sync", "--json"]);
    let got = targets(&doc);
    assert!(
        !got.iter().any(|(_, n)| n == "s1"),
        "claude sessions stay put: {got:?}"
    );
    assert!(
        !got.iter().any(|(a, n)| a == "claude" && n == ID),
        "{got:?}"
    );
    assert!(got.contains(&("codex".into(), ID.into())), "{got:?}");
    assert!(got.contains(&("codex".into(), "alpha".into())), "{got:?}");
    let items = text(&f.run(&["items", "--kind", "session", "--json"]));
    assert!(items.contains("s1"), "items still reads claude: {items}");
}

#[test]
fn a_paused_kind_leaves_existing_replicas_and_ledger_rows_alone() {
    let f = Fx::new();
    assert_eq!(f.run(&["sync", "--apply"]).status.code(), Some(0));
    f.drift_claude_index();
    let others = |f: &Fx| {
        let mut m = files_under(&f.codex);
        m.extend(files_under(&f.pi));
        m.extend(files_under(&f.root.join("opencode")));
        m
    };
    let before = others(&f);
    let ledger = fs::read(f.state.join("ledger.db")).unwrap();

    let (_, doc) = f.json(&["sync", "--json"]);
    assert_eq!(doc["actions"], serde_json::json!([]), "{doc}");
    assert_eq!(doc["notes"], serde_json::json!([]), "{doc}");
    assert!(text(&f.run(&["sync"])).contains("claude: memory paused"));

    // Deleting the paused agent's original removes nothing anywhere.
    fs::remove_file(f.claude.join("projects/-w-app/memory/a.md")).unwrap();
    let (_, doc) = f.json(&["sync", "--apply", "--json"]);
    assert_eq!(doc["actions"], serde_json::json!([]), "{doc}");
    assert_eq!(before, others(&f));
    assert_eq!(ledger, fs::read(f.state.join("ledger.db")).unwrap());
}

#[test]
fn accepting_resumes_sync_without_duplicates() {
    let f = Fx::new();
    f.drift_codex();
    assert_eq!(f.run(&["sync", "--apply"]).status.code(), Some(0));
    assert_eq!(rollouts(&f.codex), 0);

    assert_eq!(
        f.run(&["doctor", "--accept", "codex"]).status.code(),
        Some(0)
    );
    assert_eq!(f.run(&["sync", "--apply"]).status.code(), Some(0));
    assert_eq!(rollouts(&f.codex), 2);
    let after = files_under(&f.codex);

    assert_eq!(f.run(&["sync", "--apply"]).status.code(), Some(0));
    assert_eq!(rollouts(&f.codex), 2);
    let (_, doc) = f.json(&["sync", "--json"]);
    assert_eq!(doc["actions"], serde_json::json!([]), "{doc}");
    assert_eq!(after.len(), files_under(&f.codex).len());
}

fn age_pi_session(f: &Fx, days: i64) {
    for (p, _) in files_under(&f.pi.join("sessions")) {
        nativemem_core::testkit::age_days(&p, days);
    }
}

fn claude_replicas(f: &Fx) -> Vec<PathBuf> {
    files_under(&f.claude.join("projects"))
        .into_iter()
        .filter(|(p, b)| {
            p.extension().is_some_and(|e| e == "jsonl")
                && String::from_utf8_lossy(b).contains("nativemem_origin")
        })
        .map(|(p, _)| p)
        .collect()
}

const NOTICE: &str = "transcripts older than 30 days are deleted by Claude itself";

#[test]
fn a_session_older_than_the_retention_is_planned_for_other_agents_but_not_claude() {
    let f = Fx::new();
    age_pi_session(&f, 40);
    let (_, doc) = f.json(&["sync", "--json"]);
    let got = targets(&doc);
    assert!(
        !got.iter().any(|(a, n)| a == "claude" && n == ID),
        "{got:?}"
    );
    assert!(got.contains(&("codex".into(), ID.into())), "{got:?}");
    assert_eq!(doc["notes"], serde_json::json!([]));
    let plan = text(&f.run(&["sync"]));
    assert!(
        !plan.lines().any(|l| l.contains("claude") && l.contains(ID)),
        "{plan}"
    );

    let (code, doc) = f.json(&["doctor", "--json"]);
    assert_eq!(code, Some(0));
    assert!(agent(&doc, "claude")["notices"][0]
        .as_str()
        .unwrap()
        .contains(NOTICE));
    assert_eq!(agent(&doc, "claude")["status"], "ok");
    assert!(text(&f.run(&["doctor"])).contains(&format!("note: {NOTICE}")));
    let out = text(&f.run(&["status"]));
    assert!(out.contains(&format!("claude: {NOTICE}")), "{out}");
    assert!(
        out.contains("set cleanupPeriodDays in Claude's settings"),
        "{out}"
    );
}

#[test]
fn a_configured_retention_is_respected_and_needs_no_notice() {
    let f = Fx::new();
    age_pi_session(&f, 40);
    fs::write(
        f.claude.join("settings.json"),
        r#"{"cleanupPeriodDays": 90}"#,
    )
    .unwrap();
    let (_, doc) = f.json(&["sync", "--json"]);
    assert!(targets(&doc).contains(&("claude".into(), ID.into())));
    let (_, doc) = f.json(&["doctor", "--json"]);
    assert_eq!(agent(&doc, "claude")["notices"], serde_json::json!([]));

    fs::write(
        f.claude.join("settings.json"),
        r#"{"cleanupPeriodDays": 7}"#,
    )
    .unwrap();
    age_pi_session(&f, 10);
    let (_, doc) = f.json(&["sync", "--json"]);
    assert!(!targets(&doc).iter().any(|(a, n)| a == "claude" && n == ID));
    fs::write(f.claude.join("settings.json"), "not json").unwrap();
    age_pi_session(&f, 10);
    let (_, doc) = f.json(&["sync", "--json"]);
    assert!(
        targets(&doc).contains(&("claude".into(), ID.into())),
        "unparsable means the default 30"
    );
}

#[test]
fn a_replica_pruned_by_the_agent_is_not_reported_as_deleted_but_a_young_one_is() {
    let f = Fx::new();
    age_pi_session(&f, 40);
    fs::write(
        f.claude.join("settings.json"),
        r#"{"cleanupPeriodDays": 90}"#,
    )
    .unwrap();
    assert_eq!(f.run(&["sync", "--apply"]).status.code(), Some(0));
    let replicas = claude_replicas(&f);
    assert_eq!(replicas.len(), 1);
    fs::remove_file(&replicas[0]).unwrap();
    fs::remove_file(f.claude.join("settings.json")).unwrap();
    let out = text(&f.run(&["sync"]));
    assert!(!out.contains("replica deleted"), "{out}");
    let applied = text(&f.run(&["sync", "--apply"]));
    assert!(
        !applied.contains("replica deleted") && claude_replicas(&f).is_empty(),
        "{applied}"
    );

    let young = Fx::new();
    assert_eq!(young.run(&["sync", "--apply"]).status.code(), Some(0));
    let replicas = claude_replicas(&young);
    assert_eq!(replicas.len(), 1);
    fs::remove_file(&replicas[0]).unwrap();
    let out = text(&young.run(&["sync"]));
    assert!(out.contains("replica deleted"), "{out}");
}
