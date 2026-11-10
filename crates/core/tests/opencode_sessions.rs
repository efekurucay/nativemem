//! OpenCode sessions: reading, the rows the writer adds to a database built from OpenCode's real
//! table structure, the safety rules around that database, and the compatibility probes. Temp
//! directories only.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use nativemem_core::adapters::agents_file::AgentsFileAdapter;
use nativemem_core::adapters::claude::ClaudeAdapter;
use nativemem_core::adapters::opencode_project::sha1_hex;
use nativemem_core::adapters::opencode_sessions::MARKER_KEY;
use nativemem_core::adapters::pi::PiAdapter;
use nativemem_core::adapters::sqlite_guard::KEPT_BACKUPS;
use nativemem_core::engine::{self, Action, ApplyOptions};
use nativemem_core::error::Error;
use nativemem_core::fsx::BackupSession;
use nativemem_core::ledger::Ledger;
use nativemem_core::model::{content_hash, Adapter, AgentId, Item, ItemKey, Kind, Scope};
use nativemem_core::session::{Role, Session, SessionOptions, Turn};
use nativemem_core::testkit::{ClaudeHome, OpencodeData};
use rusqlite::Connection;
use serde_json::{json, Value};

type SessionRow = (
    String,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    String,
    i64,
    i64,
    Option<String>,
    Option<String>,
    i64,
    Option<i64>,
    Option<i64>,
    Option<String>,
);

const BASE: i64 = 1_794_304_800_000;
const CWD: &str = "/w/app";

struct Fx {
    _t: tempfile::TempDir,
    root: PathBuf,
    data: OpencodeData,
    home: PathBuf,
    state: PathBuf,
}

fn fx() -> Fx {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().to_path_buf();
    let data = OpencodeData::new(&root.join("data"));
    data.write_db(&[]);
    let home = root.join("config");
    fs::create_dir_all(&home).unwrap();
    Fx {
        state: root.join("state"),
        root,
        data,
        home,
        _t: t,
    }
}

fn adapter(f: &Fx, stamp: &str) -> AgentsFileAdapter {
    AgentsFileAdapter::new(
        "opencode",
        f.home.clone(),
        f.state.clone(),
        BackupSession::with_stamp(stamp),
    )
    .with_search_path(OsString::new())
    .with_opencode_sessions(f.data.db(), Some(SessionOptions::default()))
}

fn key(id: &str) -> ItemKey {
    ItemKey {
        origin: AgentId::new("claude"),
        native_id: format!("-w-app/{id}.jsonl"),
    }
}

fn session(title: &str, turns: &[(Role, &str)]) -> Session {
    Session {
        id: "orig".into(),
        cwd: PathBuf::from(CWD),
        title: title.into(),
        created_ms: BASE,
        updated_ms: BASE + 60_000,
        turns: turns
            .iter()
            .enumerate()
            .map(|(i, (r, t))| Turn::new(*r, *t, BASE + i as i64 * 20_000))
            .collect(),
    }
}

fn item(s: &Session) -> Item {
    s.to_item()
}

const CHAT: &[(Role, &str)] = &[
    (Role::User, "fix the login"),
    (Role::Assistant, "done"),
    (Role::User, "thanks"),
];

fn replicas(a: &AgentsFileAdapter) -> Vec<nativemem_core::model::Observed> {
    a.scan()
        .unwrap()
        .into_iter()
        .filter(|o| o.item.kind == Kind::Session && o.replica_of.is_some())
        .collect()
}

/// Every row of the four tables as text, so a changed or removed row shows.
fn dump(conn: &Connection) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for table in ["project", "session_v2", "session_message", "event_sequence"] {
        let mut stmt = conn.prepare(&format!("SELECT * FROM {table}")).unwrap();
        let n = stmt.column_count();
        let mut rows = stmt.query([]).unwrap();
        while let Some(r) = rows.next().unwrap() {
            let mut line = format!("{table}|");
            for i in 0..n {
                line.push_str(&format!("{:?}|", r.get_ref(i).unwrap()));
            }
            out.insert(line);
        }
    }
    out
}

fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).unwrap()
}

fn backup_stamps(f: &Fx) -> Vec<String> {
    let mut stamps: Vec<String> = fs::read_dir(f.state.join("backups"))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().join("opencode/opencode.db").is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    stamps.sort();
    stamps
}

fn native(f: &Fx, id: &str, title: Option<&str>, turns: &[(&str, &str)]) {
    f.data
        .add_session(id, Path::new(CWD), title, turns, BASE + 90_000);
}

// ---- reading ----

#[test]
fn root_sessions_are_read_with_text_and_tool_turns() {
    let f = fx();
    native(
        &f,
        "ses_a",
        Some("Fix login"),
        &[("user", "please fix"), ("assistant", "on it")],
    );
    f.data.add_message(
        "ses_a",
        20,
        "assistant",
        BASE + 5_000,
        &json!({"agent": "build", "model": {"providerID": "p", "id": "m"}, "content": [
            {"type": "tool", "id": "c1", "name": "bash", "state": {"status": "completed",
                "input": {"command": "ls -l"}, "content": [{"type": "text", "text": "a.txt"}],
                "metadata": {"secret": "dropped"}}},
            {"type": "tool", "id": "c2", "name": "read", "state": {"status": "error",
                "input": {"path": "/x"}, "error": "no such file"}},
            {"type": "reasoning", "text": "private thoughts"}],
            "time": {"created": BASE + 5_000}}),
    );
    f.data.add_message(
        "ses_a",
        21,
        "idle",
        BASE + 6_000,
        &json!({"outcome": "succeeded"}),
    );
    f.data.add_message(
        "ses_a",
        22,
        "system",
        BASE + 6_000,
        &json!({"text": "instructions"}),
    );
    let found = adapter(&f, "T").scan().unwrap();
    assert_eq!(found.len(), 1);
    let o = &found[0];
    assert_eq!(o.native_id, "ses_a.jsonl");
    assert_eq!(o.path, f.data.db());
    assert!(o.replica_of.is_none());
    let s = Session::from_body(&o.item.body).unwrap();
    assert_eq!(
        (s.cwd.as_path(), s.title.as_str()),
        (Path::new(CWD), "Fix login")
    );
    assert_eq!((s.created_ms, s.updated_ms), (BASE, BASE + 90_000));
    let texts: Vec<&str> = s.turns.iter().map(|t| t.text.as_str()).collect();
    assert_eq!(texts[..2], ["please fix", "on it"]);
    assert_eq!(
        texts[2],
        "Tool: bash\n```\nls -l\n```\nResult:\n```\na.txt\n```"
    );
    assert_eq!(
        texts[3],
        "Tool: read\n```\n{\"path\":\"/x\"}\n```\nResult (error):\n```\nno such file\n```"
    );
    assert_eq!(texts.len(), 4);
    let all = texts.join("\n");
    assert!(!all.contains("private thoughts") && !all.contains("dropped"));
}

#[test]
fn children_archived_and_userless_sessions_are_not_originals() {
    let f = fx();
    native(
        &f,
        "ses_root",
        None,
        &[("user", "first prompt line"), ("assistant", "ok")],
    );
    native(&f, "ses_child", Some("child"), &[("user", "q")]);
    native(&f, "ses_archived", Some("old"), &[("user", "q")]);
    native(&f, "ses_empty", Some("none"), &[("assistant", "only me")]);
    let conn = f.data.connect();
    conn.execute(
        "UPDATE session_v2 SET parent_id = 'ses_root' WHERE id = 'ses_child'",
        [],
    )
    .unwrap();
    conn.execute(
        "UPDATE session_v2 SET time_archived = 5 WHERE id = 'ses_archived'",
        [],
    )
    .unwrap();
    let a = adapter(&f, "T");
    let found = a.scan().unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].item.description, "first prompt line");
    let report = a.session_report().unwrap();
    assert_eq!(
        report.automated, 1,
        "sub-agent sessions are the only structural marker"
    );
    assert!(report.skipped.is_empty());
}

#[test]
fn the_window_follows_the_last_update() {
    let f = fx();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    let add = |id: &str, age_days: i64| {
        f.data.add_session(
            id,
            Path::new(CWD),
            Some(id),
            &[("user", "q")],
            now - age_days * 86_400_000,
        );
    };
    add("ses_new", 1);
    add("ses_old", 30);
    let windowed = AgentsFileAdapter::new(
        "opencode",
        f.home.clone(),
        f.state.clone(),
        BackupSession::with_stamp("T"),
    )
    .with_opencode_sessions(
        f.data.db(),
        Some(SessionOptions {
            days: Some(14),
            exclude_automated: false,
        }),
    );
    let titles: Vec<String> = windowed
        .scan()
        .unwrap()
        .into_iter()
        .map(|o| o.item.description)
        .collect();
    assert_eq!(titles, ["ses_new"]);
}

#[test]
fn sessions_are_not_read_while_session_sync_is_off() {
    let f = fx();
    native(&f, "ses_a", Some("t"), &[("user", "q")]);
    let off = AgentsFileAdapter::new(
        "opencode",
        f.home.clone(),
        f.state.clone(),
        BackupSession::with_stamp("T"),
    )
    .with_opencode_sessions(f.data.db(), None);
    assert!(off.scan().unwrap().is_empty());
    assert!(!off.supports(Kind::Session, &Scope::Project(PathBuf::from(CWD))));
}

// ---- writing ----

#[test]
fn a_written_replica_has_the_rows_opencode_lists_by() {
    let f = fx();
    f.data.add_session(
        "ses_native",
        Path::new("/w/other"),
        Some("theirs"),
        &[("user", "q")],
        BASE + 1,
    );
    f.data.connect()
        .execute(
            "UPDATE session_v2 SET model = '{\"id\":\"glm\",\"providerID\":\"zai\",\"variant\":\"high\"}' WHERE id = 'ses_native'",
            [],
        )
        .unwrap();
    let a = adapter(&f, "T");
    let s = session("Fix login", CHAT);
    let (native_id, path) = a.write_replica(&key("c1"), &item(&s)).unwrap();
    assert_eq!(path, f.data.db());
    let id = native_id.strip_suffix(".jsonl").unwrap();
    assert!(id.starts_with("ses_") && id.len() == 30, "{id}");

    let conn = f.data.connect();
    let project = sha1_hex(b"directory:/w/app");
    let row: SessionRow = conn
        .query_row(
            "SELECT project_id, directory, path, title, parent_id, model, version, time_created, \
             time_updated, time_archived, agent, resume_attempts, time_idle, time_busy, idle_outcome \
             FROM session_v2 WHERE id = ?1",
            [id],
            |r| {
                Ok((
                    r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?,
                    r.get(7)?, r.get(8)?, r.get(9)?, r.get(10)?, r.get(11)?, r.get(12)?, r.get(13)?,
                    r.get(14)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(row.0, project);
    assert_eq!(
        (row.1.as_str(), row.2.as_str(), row.3.as_str()),
        ("/w/app", "", "Fix login")
    );
    assert_eq!((row.4, row.5), (None, None));
    assert_eq!(row.6, "2.2.6");
    assert_eq!((row.7, row.8), (BASE, BASE + 60_000));
    assert_eq!((row.9, row.10.as_deref(), row.11), (None, Some("build"), 0));
    assert_eq!((row.12, row.13), (Some(BASE + 60_000), Some(BASE)));
    assert_eq!(row.14.as_deref(), Some("succeeded"));

    let p: (String, Option<String>, String, i64) = conn
        .query_row(
            "SELECT worktree, vcs, sandboxes, time_active > 0 FROM project WHERE id = ?1",
            [&project],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(
        (p.0.as_str(), p.1, p.2.as_str(), p.3),
        ("/w/app", None, "[]", 1)
    );

    let mut stmt = conn
        .prepare("SELECT id, type, seq, time_created, time_updated, data FROM session_message WHERE session_id = ?1 ORDER BY seq")
        .unwrap();
    let messages: Vec<(String, String, i64, i64, i64, Value)> = stmt
        .query_map([id], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                serde_json::from_str(&r.get::<_, String>(5)?).unwrap(),
            ))
        })
        .unwrap()
        .flatten()
        .collect();
    let kinds: Vec<&str> = messages.iter().map(|m| m.1.as_str()).collect();
    assert_eq!(kinds, ["user", "assistant", "idle", "user", "idle"]);
    assert_eq!(
        messages.iter().map(|m| m.2).collect::<Vec<_>>(),
        [0, 1, 2, 3, 4]
    );
    let ids: BTreeSet<&str> = messages.iter().map(|m| m.0.as_str()).collect();
    assert_eq!(ids.len(), messages.len());
    assert!(messages
        .iter()
        .all(|m| m.0.starts_with("msg_") && m.0.len() == 30));
    assert!(messages
        .windows(2)
        .all(|w| w[0].0 < w[1].0 && w[0].3 <= w[1].3));
    assert!(messages
        .iter()
        .all(|m| m.3 == m.4 && m.5[MARKER_KEY] == "claude:-w-app/c1.jsonl"));
    assert_eq!(messages[0].5["text"], "fix the login");
    assert_eq!(messages[1].5["content"][0]["text"], "done");
    assert_eq!(
        messages[1].5["model"],
        json!({"providerID": "zai", "id": "glm"}),
        "the model a session of this installation names"
    );
    assert_eq!(messages[1].5["agent"], "build");
    assert_eq!(messages.last().unwrap().3, BASE + 60_000);
    let seq: i64 = count(
        &conn,
        &format!("SELECT seq FROM event_sequence WHERE aggregate_id = '{id}'"),
    );
    assert_eq!(seq, 4, "the last sequence number in use");
    let slug: String = conn
        .query_row("SELECT slug FROM session_v2 WHERE id = ?1", [id], |r| {
            r.get(0)
        })
        .unwrap();
    assert!(
        slug.split('-').count() == 2 && slug.chars().all(|c| c.is_ascii_lowercase() || c == '-')
    );
}

#[test]
fn the_picker_and_continue_queries_list_the_replica_newest_first() {
    let f = fx();
    native(&f, "ses_a", Some("older"), &[("user", "q")]);
    let a = adapter(&f, "T");
    let (native_id, _) = a
        .write_replica(&key("c1"), &item(&session("Fix", CHAT)))
        .unwrap();
    let conn = f.data.connect();
    conn.execute(
        "UPDATE session_v2 SET time_updated = ?1 WHERE id = 'ses_a'",
        [BASE],
    )
    .unwrap();
    let newest: String = conn
        .query_row(
            "SELECT id FROM session_v2 WHERE directory = '/w/app' AND parent_id IS NULL \
             AND time_archived IS NULL ORDER BY time_updated DESC, id DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(format!("{newest}.jsonl"), native_id);
    let picker = count(
        &conn,
        &format!(
            "SELECT count(*) FROM session_v2 WHERE project_id = '{}' AND path = '' AND parent_id IS NULL",
            sha1_hex(b"directory:/w/app")
        ),
    );
    assert_eq!(picker, 2);
}

#[test]
fn a_replica_reads_back_like_its_original_and_carries_the_marker() {
    let f = fx();
    let a = adapter(&f, "T");
    let s = session(
        "Fix login",
        &[
            (Role::User, "q"),
            (
                Role::Assistant,
                "Tool: bash\n```\nls\n```\nResult:\n```\nx\n```",
            ),
            (Role::Assistant, "multi\nline  text "),
        ],
    );
    a.write_replica(&key("c1"), &item(&s)).unwrap();
    let found = replicas(&a);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].replica_of, Some(key("c1")));
    assert_eq!(content_hash(&found[0].item), content_hash(&item(&s)));
    assert_eq!(
        Session::from_body(&found[0].item.body).unwrap().updated_ms,
        s.updated_ms
    );
}

#[test]
fn an_existing_project_row_is_reused_and_a_sub_directory_keeps_its_path() {
    let t = tempfile::tempdir().unwrap();
    let repo = t.path().join("repo");
    fs::create_dir_all(repo.join(".git")).unwrap();
    fs::write(
        repo.join(".git/config"),
        "[remote \"origin\"]\n\turl = https://Example.com/acme/app.git\n",
    )
    .unwrap();
    fs::create_dir_all(repo.join("crates/core")).unwrap();
    let f = fx();
    let project = sha1_hex(b"git-remote:example.com/acme/app");
    f.data
        .connect()
        .execute(
            "INSERT INTO project (id, worktree, vcs, name, sandboxes, time_created, time_updated) \
             VALUES (?1, ?2, 'git', 'kept', '[\"x\"]', 7, 8)",
            rusqlite::params![project, repo.to_string_lossy()],
        )
        .unwrap();
    let before = dump(&f.data.connect());
    let a = adapter(&f, "T");
    let mut s = session("Deep", CHAT);
    s.cwd = repo.join("crates/core");
    let (native_id, _) = a.write_replica(&key("c1"), &item(&s)).unwrap();
    let conn = f.data.connect();
    let (pid, dir, path): (String, String, String) = conn
        .query_row(
            "SELECT project_id, directory, path FROM session_v2 WHERE id = ?1",
            [native_id.trim_end_matches(".jsonl")],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(pid, project);
    assert_eq!(dir, s.cwd.to_string_lossy());
    assert_eq!(path, "crates/core");
    assert_eq!(count(&conn, "SELECT count(*) FROM project"), 1);
    assert!(
        before.is_subset(&dump(&conn)),
        "the project row is untouched"
    );
}

#[test]
fn a_new_repository_project_gets_its_own_row() {
    let t = tempfile::tempdir().unwrap();
    let repo = t.path().join("repo");
    fs::create_dir_all(repo.join(".git")).unwrap();
    fs::write(repo.join(".git/opencode"), "abc123\n").unwrap();
    let f = fx();
    let a = adapter(&f, "T");
    let mut s = session("In repo", CHAT);
    s.cwd = repo.clone();
    a.write_replica(&key("c1"), &item(&s)).unwrap();
    let (id, worktree, vcs): (String, String, Option<String>) = f
        .data
        .connect()
        .query_row("SELECT id, worktree, vcs FROM project", [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .unwrap();
    assert_eq!((id.as_str(), vcs.as_deref()), ("abc123", Some("git")));
    assert_eq!(worktree, repo.to_string_lossy());
}

#[test]
fn rows_opencode_made_are_never_touched_and_nothing_is_deleted() {
    let f = fx();
    native(
        &f,
        "ses_a",
        Some("Mine"),
        &[("user", "q"), ("assistant", "a")],
    );
    native(&f, "ses_b", Some("Other"), &[("user", "q2")]);
    let before = dump(&f.data.connect());
    let a = adapter(&f, "T");
    a.write_replica(&key("c1"), &item(&session("One", CHAT)))
        .unwrap();
    a.write_replica(&key("c2"), &item(&session("Two", &CHAT[..2])))
        .unwrap();
    let mid = dump(&f.data.connect());
    assert!(before.is_subset(&mid));
    let longer: Vec<(Role, &str)> = CHAT
        .iter()
        .copied()
        .chain([(Role::Assistant, "more"), (Role::User, "again")])
        .collect();
    let shorter = &CHAT[..1];
    let b = adapter(&f, "T2");
    b.write_replica(&key("c1"), &item(&session("One", &longer)))
        .unwrap();
    b.write_replica(&key("c1"), &item(&session("One", shorter)))
        .unwrap();
    let after = dump(&f.data.connect());
    assert!(
        before.is_subset(&after),
        "native rows are byte for byte the same"
    );
    let conn = f.data.connect();
    assert_eq!(count(&conn, "SELECT count(*) FROM session_v2"), 4);
    assert!(
        count(
            &conn,
            "SELECT count(*) FROM session_message WHERE session_id NOT IN ('ses_a','ses_b')"
        ) >= 8
    );
}

#[test]
fn a_refresh_reuses_the_session_and_leftover_rows_become_idle() {
    let f = fx();
    let a = adapter(&f, "T");
    let longer: Vec<(Role, &str)> = CHAT
        .iter()
        .copied()
        .chain([(Role::Assistant, "more"), (Role::User, "again")])
        .collect();
    let (first, _) = a
        .write_replica(&key("c1"), &item(&session("Old", &longer)))
        .unwrap();
    let conn = f.data.connect();
    let rows_after_long = count(&conn, "SELECT count(*) FROM session_message");
    let (second, _) = a
        .write_replica(&key("c1"), &item(&session("New", CHAT)))
        .unwrap();
    assert_eq!(first, second);
    assert_eq!(count(&conn, "SELECT count(*) FROM session_v2"), 1);
    assert_eq!(
        count(&conn, "SELECT count(*) FROM session_message"),
        rows_after_long
    );
    let title: String = conn
        .query_row("SELECT title FROM session_v2", [], |r| r.get(0))
        .unwrap();
    assert_eq!(title, "New");
    let marked = count(
        &conn,
        &format!(
            "SELECT count(*) FROM session_message WHERE json_extract(data, '$.{MARKER_KEY}') = 'claude:-w-app/c1.jsonl'"
        ),
    );
    assert_eq!(marked, rows_after_long);
    let found = replicas(&a);
    assert_eq!(found.len(), 1);
    assert_eq!(
        content_hash(&found[0].item),
        content_hash(&item(&session("New", CHAT)))
    );
    let seq = count(&conn, "SELECT seq FROM event_sequence");
    assert_eq!(
        seq,
        count(&conn, "SELECT max(seq) FROM session_message"),
        "the sequence never goes below a row in use"
    );
}

#[test]
fn a_replica_with_rows_this_tool_did_not_write_is_left_alone() {
    let f = fx();
    let a = adapter(&f, "T");
    let (native_id, _) = a
        .write_replica(&key("c1"), &item(&session("One", CHAT)))
        .unwrap();
    let id = native_id.trim_end_matches(".jsonl");
    f.data.add_message(
        id,
        50,
        "user",
        BASE + 99_000,
        &json!({"text": "typed in opencode"}),
    );
    let before = dump(&f.data.connect());
    let err = a
        .write_replica(&key("c1"), &item(&session("One", &CHAT[..1])))
        .unwrap_err();
    assert!(matches!(err, Error::Unsupported { .. }), "{err}");
    assert_eq!(dump(&f.data.connect()), before);
}

#[test]
fn ids_never_repeat_across_replicas_and_runs() {
    let f = fx();
    for round in 0..2 {
        let a = adapter(&f, &format!("T{round}"));
        for n in 0..3 {
            a.write_replica(&key(&format!("c{n}")), &item(&session("Same", CHAT)))
                .unwrap();
        }
    }
    let conn = f.data.connect();
    assert_eq!(count(&conn, "SELECT count(*) FROM session_v2"), 3);
    assert_eq!(
        count(&conn, "SELECT count(DISTINCT id) FROM session_message"),
        count(&conn, "SELECT count(*) FROM session_message")
    );
    assert_eq!(count(&conn, "SELECT count(*) FROM event_sequence"), 3);
}

#[test]
fn a_session_whose_project_cannot_be_derived_is_skipped_not_guessed() {
    let t = tempfile::tempdir().unwrap();
    fs::create_dir_all(t.path().join(".hg")).unwrap();
    let f = fx();
    let mut s = session("Hg", CHAT);
    s.cwd = t.path().to_path_buf();
    let err = adapter(&f, "T")
        .write_replica(&key("c1"), &item(&s))
        .unwrap_err();
    assert!(matches!(err, Error::Unsupported { .. }), "{err}");
    assert_eq!(
        count(&f.data.connect(), "SELECT count(*) FROM session_v2"),
        0
    );
    assert!(
        backup_stamps(&f).is_empty()
            || count(&f.data.connect(), "SELECT count(*) FROM project") == 0
    );
}

// ---- the database ----

#[test]
fn a_busy_database_is_skipped_for_the_run_and_repaired_on_the_next() {
    let f = fx();
    native(&f, "ses_a", Some("Mine"), &[("user", "q")]);
    let blocker = f.data.connect();
    blocker.execute_batch("BEGIN IMMEDIATE").unwrap();
    let a = adapter(&f, "T").with_session_busy_timeout(Duration::from_millis(50));
    let started = std::time::Instant::now();
    let err = a
        .write_replica(&key("c1"), &item(&session("One", CHAT)))
        .unwrap_err();
    assert!(matches!(err, Error::Unsupported { .. }), "{err}");
    let err = a
        .write_replica(&key("c2"), &item(&session("Two", CHAT)))
        .unwrap_err();
    assert!(err.to_string().contains("busy"), "{err}");
    assert!(started.elapsed() < Duration::from_secs(2));
    blocker.execute_batch("ROLLBACK").unwrap();
    assert_eq!(
        count(&f.data.connect(), "SELECT count(*) FROM session_v2"),
        1
    );

    let next = adapter(&f, "T2");
    next.write_replica(&key("c1"), &item(&session("One", CHAT)))
        .unwrap();
    assert_eq!(
        count(&f.data.connect(), "SELECT count(*) FROM session_v2"),
        2
    );
}

#[test]
fn the_journal_mode_stays_wal_and_a_wal_database_reads_and_writes() {
    let f = fx();
    let conn = f.data.connect();
    let mode: String = conn
        .query_row("PRAGMA journal_mode = WAL", [], |r| r.get(0))
        .unwrap();
    assert_eq!(mode, "wal");
    native(&f, "ses_a", Some("Mine"), &[("user", "q")]);
    let a = adapter(&f, "T");
    assert_eq!(a.scan().unwrap().len(), 1);
    a.write_replica(&key("c1"), &item(&session("One", CHAT)))
        .unwrap();
    let mode: String = f
        .data
        .connect()
        .query_row("PRAGMA journal_mode", [], |r| r.get(0))
        .unwrap();
    assert_eq!(mode, "wal");
    assert_eq!(replicas(&a).len(), 1);
}

#[test]
fn nothing_is_opened_for_writing_when_there_is_nothing_to_write() {
    let f = fx();
    native(&f, "ses_a", Some("Mine"), &[("user", "q")]);
    let bytes = fs::read(f.data.db()).unwrap();
    let a = adapter(&f, "T");
    a.scan().unwrap();
    a.probes();
    assert_eq!(fs::read(f.data.db()).unwrap(), bytes);
    assert!(!f.state.join("backups").exists());
}

#[test]
fn one_consistent_backup_is_taken_before_the_first_write_of_a_run() {
    let f = fx();
    native(&f, "ses_a", Some("Mine"), &[("user", "q")]);
    let a = adapter(&f, "T1");
    a.write_replica(&key("c1"), &item(&session("One", CHAT)))
        .unwrap();
    a.write_replica(&key("c2"), &item(&session("Two", CHAT)))
        .unwrap();
    let copy = f.state.join("backups/T1/opencode/opencode.db");
    let backup = Connection::open(&copy).unwrap();
    assert_eq!(
        count(&backup, "SELECT count(*) FROM session_v2"),
        1,
        "the copy holds the state before the run's first write"
    );
    assert_eq!(
        count(&f.data.connect(), "SELECT count(*) FROM session_v2"),
        3
    );
}

#[test]
fn only_the_newest_database_backups_are_kept_and_nothing_else_is_pruned() {
    let f = fx();
    fs::create_dir_all(f.state.join("backups/A0/opencode")).unwrap();
    fs::write(f.state.join("backups/A0/opencode/AGENTS.md"), "keep").unwrap();
    fs::create_dir_all(f.state.join("backups/A0/codex")).unwrap();
    fs::write(f.state.join("backups/A0/codex/state_5.sqlite"), "keep").unwrap();
    for n in 1..=KEPT_BACKUPS + 2 {
        let a = adapter(&f, &format!("B{n}"));
        a.write_replica(&key(&format!("c{n}")), &item(&session("S", CHAT)))
            .unwrap();
    }
    let kept = backup_stamps(&f);
    assert_eq!(kept.len(), KEPT_BACKUPS);
    assert_eq!(kept.first().unwrap(), "B3");
    assert_eq!(kept.last().unwrap(), "B7");
    assert!(f.state.join("backups/A0/opencode/AGENTS.md").is_file());
    assert!(f.state.join("backups/A0/codex/state_5.sqlite").is_file());
}

// ---- gating ----

#[test]
fn writes_follow_the_probes_and_the_store() {
    let f = fx();
    native(&f, "ses_a", Some("Mine"), &[("user", "q")]);
    let project = Scope::Project(PathBuf::from(CWD));
    assert!(adapter(&f, "T").supports(Kind::Session, &project));
    let blocked = adapter(&f, "T").with_blocked_kinds(&[Kind::Session]);
    assert!(!blocked.supports(Kind::Session, &project));
    assert!(blocked.scan().unwrap().is_empty());
    assert!(!adapter(&f, "T").supports(Kind::Session, &Scope::Global));
    let none = AgentsFileAdapter::new(
        "opencode",
        f.home.clone(),
        f.state.clone(),
        BackupSession::with_stamp("T"),
    );
    assert!(!none.supports(Kind::Session, &project));
}

fn check<'a>(
    checks: &'a [nativemem_core::compat::Check],
    id: &str,
) -> &'a nativemem_core::compat::Check {
    checks.iter().find(|c| c.id == id).unwrap()
}

#[test]
fn the_schema_probe_passes_on_the_real_structure_and_pauses_on_drift() {
    let f = fx();
    let checks = adapter(&f, "T").probes();
    let schema = check(&checks, "opencode.session_schema");
    assert!(schema.ok, "{}", schema.observed);
    assert_eq!(schema.feature_kinds, [Kind::Session]);
    assert!(check(&checks, "opencode.version").ok);

    let drift = fx();
    drift
        .data
        .connect()
        .execute("ALTER TABLE session_v2 ADD COLUMN extra TEXT", [])
        .unwrap();
    let checks = adapter(&drift, "T").probes();
    let c = check(&checks, "opencode.session_schema");
    assert!(!c.ok);
    assert!(
        c.observed.contains("added: session_v2.extra"),
        "{}",
        c.observed
    );

    let missing = fx();
    missing
        .data
        .connect()
        .execute("DROP TABLE event_sequence", [])
        .unwrap();
    let c = &adapter(&missing, "T").probes();
    let c = check(c, "opencode.session_schema");
    assert!(
        !c.ok && c.observed.contains("event_sequence.seq"),
        "{}",
        c.observed
    );

    let renamed = fx();
    renamed
        .data
        .connect()
        .execute_batch("ALTER TABLE session_message RENAME COLUMN seq TO position")
        .unwrap();
    assert!(!check(&adapter(&renamed, "T").probes(), "opencode.session_schema").ok);
}

#[test]
fn a_database_that_is_not_sqlite_fails_the_schema_probe() {
    let f = fx();
    fs::write(f.data.db(), "not a database at all, just text padding").unwrap();
    let checks = adapter(&f, "T").probes();
    let c = check(&checks, "opencode.session_schema");
    assert!(!c.ok, "{}", c.observed);
}

#[cfg(unix)]
#[test]
fn the_version_probe_reads_the_install_path_and_fails_on_another_major() {
    let f = fx();
    let probe = |version: &str| {
        let bin = f.root.join(format!("bin-{version}"));
        let real = f.root.join(format!("Cellar/opencode/{version}/bin"));
        fs::create_dir_all(&real).unwrap();
        fs::create_dir_all(&bin).unwrap();
        fs::write(real.join("opencode"), "#!/bin/sh\n").unwrap();
        std::os::unix::fs::symlink(real.join("opencode"), bin.join("opencode")).unwrap();
        let a = adapter(&f, "T").with_search_path(bin.into_os_string());
        let checks = a.probes();
        (a.version(), check(&checks, "opencode.version").clone())
    };
    let (v, ok) = probe("2.9.1");
    assert_eq!(v.as_deref(), Some("2.9.1"));
    assert!(ok.ok);
    let (_, newer) = probe("3.0.0");
    assert!(!newer.ok && newer.observed == "3.0.0");
    let (_, older) = probe("1.17.20");
    assert!(!older.ok);
}

#[test]
fn the_installed_version_is_the_version_of_a_written_session() {
    let f = fx();
    let real = f.root.join("Cellar/opencode/2.4.0/bin");
    let bin = f.root.join("bin");
    fs::create_dir_all(&real).unwrap();
    fs::create_dir_all(&bin).unwrap();
    fs::write(real.join("opencode"), "#!/bin/sh\n").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(real.join("opencode"), bin.join("opencode")).unwrap();
    let a = adapter(&f, "T").with_search_path(bin.into_os_string());
    a.write_replica(&key("c1"), &item(&session("One", CHAT)))
        .unwrap();
    let version: String = f
        .data
        .connect()
        .query_row("SELECT version FROM session_v2", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, "2.4.0");
}

// ---- through the engine ----

struct World {
    f: Fx,
    claude: PathBuf,
    pi: PathBuf,
    ledger: Ledger,
}

fn world() -> World {
    let f = fx();
    let claude = f.root.join("claude");
    let pi = f.root.join("pi");
    fs::create_dir_all(&pi).unwrap();
    World {
        claude,
        pi,
        f,
        ledger: Ledger::in_memory().unwrap(),
    }
}

impl World {
    fn adapters(&self) -> (ClaudeAdapter, PiAdapter, AgentsFileAdapter) {
        let stamp = || BackupSession::with_stamp("T");
        (
            ClaudeAdapter::new(self.claude.clone(), self.f.state.clone(), stamp())
                .with_sessions(SessionOptions::default()),
            PiAdapter::new(self.pi.clone(), self.f.state.clone(), stamp())
                .with_sessions(SessionOptions::default()),
            adapter(&self.f, "T"),
        )
    }

    /// Plans, applies, and returns the agents the plan wrote replicas to.
    fn plan_and_apply(&self) -> Vec<String> {
        let (c, p, o) = self.adapters();
        let all: Vec<&dyn Adapter> = vec![&c, &p, &o];
        let plan = engine::plan(&all, &self.ledger).unwrap();
        let targets = plan
            .actions
            .iter()
            .filter_map(|a| match a {
                Action::Write { target, .. } => Some(target.to_string()),
                _ => None,
            })
            .collect();
        engine::apply(
            plan,
            &all,
            &self.ledger,
            &ApplyOptions {
                allow_mass_delete: false,
            },
        )
        .unwrap();
        targets
    }

    fn plan(&self) -> engine::Plan {
        let (c, p, o) = self.adapters();
        let all: Vec<&dyn Adapter> = vec![&c, &p, &o];
        engine::plan(&all, &self.ledger).unwrap()
    }
}

fn writes_to(plan: &engine::Plan, agent: &str) -> usize {
    plan.actions
        .iter()
        .filter(|a| matches!(a, Action::Write { target, .. } if target.as_str() == agent))
        .count()
}

fn wrote(targets: &[String], agent: &str) -> usize {
    targets.iter().filter(|t| t.as_str() == agent).count()
}

#[test]
fn a_claude_session_becomes_an_opencode_replica_and_the_second_plan_is_empty() {
    let w = world();
    ClaudeHome::new(&w.claude).add_session(
        Path::new(CWD),
        "c1",
        Some("Fix login"),
        &[("user", "fix it"), ("assistant", "done")],
    );
    let first = w.plan_and_apply();
    assert_eq!(wrote(&first, "opencode"), 1);
    assert_eq!(wrote(&first, "pi"), 1);
    let second = w.plan();
    assert!(second.is_empty(), "{second}");
    let conn = w.f.data.connect();
    assert_eq!(count(&conn, "SELECT count(*) FROM session_v2"), 1);
    assert_eq!(count(&conn, "SELECT count(*) FROM event_sequence"), 1);
    assert_eq!(count(&conn, "SELECT count(*) FROM project"), 1);
    w.plan_and_apply();
    assert_eq!(
        count(&conn, "SELECT count(*) FROM session_v2"),
        1,
        "no duplicate on a rerun"
    );
}

#[test]
fn an_opencode_session_becomes_a_claude_and_pi_replica() {
    let w = world();
    native(
        &w.f,
        "ses_a",
        Some("Native chat"),
        &[("user", "hello"), ("assistant", "hi")],
    );
    let first = w.plan_and_apply();
    assert_eq!(wrote(&first, "claude"), 1);
    assert_eq!(wrote(&first, "pi"), 1);
    assert_eq!(wrote(&first, "opencode"), 0);
    assert!(w.plan().is_empty());
    assert_eq!(w.f.data.session_ids(), ["ses_a"], "its own row only");
}

#[test]
fn a_paused_session_kind_plans_nothing_for_opencode_and_leaves_the_database_alone() {
    let w = world();
    ClaudeHome::new(&w.claude).add_session(Path::new(CWD), "c1", Some("T"), &[("user", "q")]);
    let (c, p, o) = w.adapters();
    let o = o.with_blocked_kinds(&[Kind::Session]);
    let all: Vec<&dyn Adapter> = vec![&c, &p, &o];
    let plan = engine::plan(&all, &w.ledger).unwrap();
    assert_eq!(writes_to(&plan, "opencode"), 0, "{plan}");
    assert_eq!(writes_to(&plan, "pi"), 1, "{plan}");
    assert!(w.f.data.session_ids().is_empty());
    assert!(!w.f.state.join("backups").exists());
}

#[test]
fn a_lost_ledger_adopts_the_replica_by_its_marker() {
    let w = world();
    ClaudeHome::new(&w.claude).add_session(
        Path::new(CWD),
        "c1",
        Some("T"),
        &[("user", "q"), ("assistant", "a")],
    );
    w.plan_and_apply();
    let rows = dump(&w.f.data.connect());
    let fresh = World {
        ledger: Ledger::in_memory().unwrap(),
        ..w
    };
    let plan = fresh.plan();
    assert_eq!(writes_to(&plan, "opencode"), 0, "{plan}");
    assert!(
        !plan.actions.is_empty(),
        "the replica is adopted rather than written again"
    );
    let (c, p, o) = fresh.adapters();
    let all: Vec<&dyn Adapter> = vec![&c, &p, &o];
    engine::apply(
        plan,
        &all,
        &fresh.ledger,
        &ApplyOptions {
            allow_mass_delete: false,
        },
    )
    .unwrap();
    assert_eq!(dump(&fresh.f.data.connect()), rows);
    assert!(fresh.plan().is_empty());
}

#[test]
fn a_deleted_replica_is_never_written_again_and_a_vanished_original_removes_nothing() {
    let w = world();
    let claude = ClaudeHome::new(&w.claude);
    claude.add_session(
        Path::new(CWD),
        "c1",
        Some("T"),
        &[("user", "q"), ("assistant", "a")],
    );
    w.plan_and_apply();
    let conn = w.f.data.connect();
    conn.execute_batch(
        "DELETE FROM session_message; DELETE FROM event_sequence; DELETE FROM session_v2;",
    )
    .unwrap();
    let plan = w.plan();
    assert_eq!(writes_to(&plan, "opencode"), 0, "{plan}");
    w.plan_and_apply();
    assert_eq!(count(&conn, "SELECT count(*) FROM session_v2"), 0);

    let w2 = world();
    ClaudeHome::new(&w2.claude).add_session(Path::new(CWD), "c1", Some("T"), &[("user", "q")]);
    w2.plan_and_apply();
    fs::remove_dir_all(w2.claude.join("projects")).unwrap();
    let plan = w2.plan();
    assert!(plan.actions.is_empty(), "{plan}");
    assert_eq!(
        count(&w2.f.data.connect(), "SELECT count(*) FROM session_v2"),
        1
    );
}

#[test]
fn a_grown_original_refreshes_an_untouched_replica_but_not_a_continued_one() {
    let w = world();
    let claude = ClaudeHome::new(&w.claude);
    claude.add_session(
        Path::new(CWD),
        "c1",
        Some("T"),
        &[("user", "q"), ("assistant", "a")],
    );
    w.plan_and_apply();
    claude.add_session(
        Path::new(CWD),
        "c1",
        Some("T"),
        &[
            ("user", "q"),
            ("assistant", "a"),
            ("user", "more"),
            ("assistant", "yes"),
        ],
    );
    let plan = w.plan();
    assert_eq!(writes_to(&plan, "opencode"), 1, "{plan}");
    w.plan_and_apply();
    assert!(w.plan().is_empty());
    let conn = w.f.data.connect();
    let user_turns = count(
        &conn,
        "SELECT count(*) FROM session_message WHERE type = 'user' AND json_extract(data, '$.text') = 'more'",
    );
    assert_eq!(user_turns, 1);

    let id: String = conn
        .query_row("SELECT id FROM session_v2", [], |r| r.get(0))
        .unwrap();
    w.f.data.add_message(
        &id,
        99,
        "user",
        BASE + 500_000,
        &json!({"text": "continued in opencode"}),
    );
    claude.add_session(
        Path::new(CWD),
        "c1",
        Some("T"),
        &[
            ("user", "q"),
            ("assistant", "a"),
            ("user", "more"),
            ("assistant", "yes"),
            ("user", "third"),
        ],
    );
    assert_eq!(writes_to(&w.plan(), "opencode"), 0);
}

#[test]
fn an_older_original_keeps_the_newest_time_updated_ordering() {
    let w = world();
    native(&w.f, "ses_new", Some("Recent"), &[("user", "q")]);
    ClaudeHome::new(&w.claude).add_session(
        Path::new(CWD),
        "c1",
        Some("Old chat"),
        &[("user", "q"), ("assistant", "a")],
    );
    w.plan_and_apply();
    let conn = w.f.data.connect();
    let replica_updated: i64 = conn
        .query_row(
            "SELECT time_updated FROM session_v2 WHERE id <> 'ses_new'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let (c, _, _) = w.adapters();
    let original = c
        .scan()
        .unwrap()
        .into_iter()
        .find(|o| o.item.kind == Kind::Session && o.replica_of.is_none())
        .unwrap();
    let s = Session::from_body(&original.item.body).unwrap();
    assert_eq!(replica_updated, s.updated_ms);
}
