//! The `threads` rows of replica rollouts, against a fixture database built from Codex's real
//! table structure. Temp directories only.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use nativemem_core::adapters::codex::CodexAdapter;
use nativemem_core::adapters::codex_threads::sync_rows;
use nativemem_core::fsx::BackupSession;
use nativemem_core::model::{Adapter, AgentId, Item, ItemKey, Kind, Scope};
use nativemem_core::session::{parse_iso_ms, Role, Session, SessionOptions, Turn};
use nativemem_core::testkit::CodexHome;
use rusqlite::Connection;

struct Fx {
    _t: tempfile::TempDir,
    home: PathBuf,
    state: PathBuf,
}

fn fx() -> Fx {
    let t = tempfile::tempdir().unwrap();
    let home = t.path().join("codex");
    let state = t.path().join("state");
    CodexHome::new(&home).write_state_db(5, &[]);
    Fx { _t: t, home, state }
}

fn adapter(f: &Fx, stamp: &str) -> CodexAdapter {
    CodexAdapter::new(
        f.home.clone(),
        f.state.clone(),
        BackupSession::with_stamp(stamp),
    )
    .with_sessions(SessionOptions::default())
}

fn key(id: &str) -> ItemKey {
    ItemKey {
        origin: AgentId::new("claude"),
        native_id: format!("-w-app/{id}.jsonl"),
    }
}

fn item(title: &str, first: &str) -> Item {
    let t0 = parse_iso_ms("2026-11-10T10:00:00.000Z").unwrap();
    let session = Session {
        id: "o".into(),
        cwd: PathBuf::from("/w/app"),
        title: title.into(),
        created_ms: t0,
        updated_ms: t0 + 60_000,
        turns: vec![
            Turn::new(Role::User, first, t0),
            Turn::new(Role::Assistant, "answer", t0 + 30_000),
            Turn::new(Role::User, "more", t0 + 60_000),
        ],
    };
    Item {
        kind: Kind::Session,
        scope: Scope::Project("/w/app".into()),
        name: "o".into(),
        description: title.into(),
        category: None,
        body: session.to_body(),
        source_dir: None,
    }
}

fn db(f: &Fx) -> Connection {
    Connection::open(f.home.join("state_5.sqlite")).unwrap()
}

type Row = (String, String, String, String, i64, i64, i64, String);

fn row(f: &Fx, id: &str) -> Option<Row> {
    db(f)
        .query_row(
            "SELECT rollout_path, cwd, title, first_user_message, created_at, updated_at, \
             created_at_ms, source FROM threads WHERE id = ?1",
            [id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                ))
            },
        )
        .ok()
}

fn count(f: &Fx) -> i64 {
    db(f)
        .query_row("SELECT count(*) FROM threads", [], |r| r.get(0))
        .unwrap()
}

fn all_rows_hash(f: &Fx, except: &[&str]) -> String {
    let conn = db(f);
    let mut stmt = conn.prepare("SELECT * FROM threads ORDER BY id").unwrap();
    let n = stmt.column_count();
    let mut text = String::new();
    let mut rows = stmt.query([]).unwrap();
    while let Some(r) = rows.next().unwrap() {
        let id: String = r.get(0).unwrap();
        if except.contains(&id.as_str()) {
            continue;
        }
        for i in 0..n {
            text.push_str(&format!("{:?}|", r.get_ref(i).unwrap()));
        }
        text.push('\n');
    }
    text
}

fn write(a: &CodexAdapter, k: &ItemKey, it: &Item) -> (String, PathBuf) {
    let (_, path) = a.write_replica(k, it).unwrap();
    let id = path
        .file_stem()
        .unwrap()
        .to_string_lossy()
        .rsplit_once("T")
        .map(|(_, r)| r[9..].to_string())
        .unwrap();
    (id, path)
}

fn foreign_row(f: &Fx, id: &str) {
    db(f)
        .execute(
            "INSERT INTO threads (id, rollout_path, created_at, updated_at, source, \
             model_provider, cwd, title, sandbox_policy, approval_mode, preview) \
             VALUES (?1, '/x/r.jsonl', 5, 6, 'cli', 'openai', '/w/other', 'theirs', 'p', 'never', 'q')",
            [id],
        )
        .unwrap();
}

#[test]
fn a_written_replica_gets_a_row_that_agrees_with_its_rollout() {
    let f = fx();
    let a = adapter(&f, "T1");
    let (id, path) = write(&a, &key("s1"), &item("Fix login", "please fix"));
    assert_eq!(count(&f), 0, "rows follow in finish, not in the write");
    assert!(a.finish().unwrap().is_empty());
    let (rollout, cwd, title, first, created, updated, created_ms, source) =
        row(&f, &id).expect("row");
    assert_eq!(rollout, path.to_string_lossy());
    assert_eq!((cwd.as_str(), title.as_str()), ("/w/app", "Fix login"));
    assert_eq!((first.as_str(), source.as_str()), ("please fix", "vscode"));
    let t0 = parse_iso_ms("2026-11-10T10:00:00.000Z").unwrap() / 1000;
    assert_eq!((created, updated, created_ms), (t0, t0 + 60, t0 * 1000));
    let head = fs::read_to_string(&path).unwrap();
    let meta: serde_json::Value = serde_json::from_str(head.lines().next().unwrap()).unwrap();
    assert_eq!(meta["payload"]["id"], id.as_str());
    assert_eq!(meta["payload"]["cwd"], "/w/app");
    let conn = db(&f);
    let (pre, rec, hist, ver): (String, i64, String, String) = conn
        .query_row(
            "SELECT preview, recency_at, history_mode, cli_version FROM threads WHERE id=?1",
            [&id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(
        (pre.as_str(), rec, hist.as_str()),
        ("please fix", t0 + 60, "legacy")
    );
    assert_eq!(ver, meta["payload"]["cli_version"].as_str().unwrap());
}

#[test]
fn refresh_updates_the_derived_columns_and_keeps_what_codex_owns() {
    let f = fx();
    let (id, _) = write(&adapter(&f, "T1"), &key("s1"), &item("Old", "q"));
    adapter(&f, "T1").finish().unwrap();
    db(&f)
        .execute(
            "UPDATE threads SET archived = 1, is_pinned = 1, name = 'renamed' WHERE id = ?1",
            [&id],
        )
        .unwrap();
    let a = adapter(&f, "T2");
    write(&a, &key("s1"), &item("New", "q2"));
    a.finish().unwrap();
    assert_eq!(count(&f), 1);
    assert_eq!(row(&f, &id).unwrap().2, "New");
    let (arch, pin, name): (i64, i64, String) = db(&f)
        .query_row(
            "SELECT archived, is_pinned, name FROM threads WHERE id=?1",
            [&id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!((arch, pin, name.as_str()), (1, 1, "renamed"));
}

#[test]
fn a_rollout_without_a_row_is_repaired_and_a_resumed_row_is_not_reverted() {
    let f = fx();
    let a = adapter(&f, "T1");
    let (id, _) = write(&a, &key("s1"), &item("T", "q"));
    // The earlier run never wrote the row.
    a.finish().unwrap();
    db(&f).execute("DELETE FROM threads", []).unwrap();
    adapter(&f, "T2").finish().unwrap();
    assert!(row(&f, &id).is_some());
    // Codex resumed the thread: its row is ahead of the rollout and stays.
    db(&f)
        .execute(
            "UPDATE threads SET updated_at = updated_at + 999, title = 'edited' WHERE id=?1",
            [&id],
        )
        .unwrap();
    let before = all_rows_hash(&f, &[]);
    adapter(&f, "T3").finish().unwrap();
    assert_eq!(before, all_rows_hash(&f, &[]));
    assert!(
        !f.state.join("backups/T3").exists(),
        "nothing to write, no backup"
    );
}

#[test]
fn rollouts_written_before_the_database_existed_get_their_rows_later() {
    let f = fx();
    fs::remove_file(f.home.join("state_5.sqlite")).unwrap();
    let a = adapter(&f, "T1");
    let (id, _) = write(&a, &key("s1"), &item("T", "q"));
    assert!(a.finish().unwrap().is_empty());
    CodexHome::new(&f.home).write_state_db(5, &[]);
    adapter(&f, "T2").finish().unwrap();
    assert!(row(&f, &id).is_some());
}

#[test]
fn foreign_rows_and_foreign_rollouts_are_never_touched_and_nothing_is_deleted() {
    let f = fx();
    foreign_row(&f, "foreign-1");
    foreign_row(&f, "foreign-2");
    let codex = CodexHome::new(&f.home);
    codex.add_rollout(
        "01a1c682-fd13-7263-8871-4ca47590d7d1",
        Path::new("/w/app"),
        &[("user", "hi"), ("assistant", "yo")],
        true,
    );
    let theirs = all_rows_hash(&f, &[]);
    let a = adapter(&f, "T1");
    let (id, _) = write(&a, &key("s1"), &item("T", "q"));
    a.finish().unwrap();
    assert_eq!(count(&f), 3, "Codex's imported rollout gets no row");
    assert_eq!(theirs, all_rows_hash(&f, &[&id]));
    let a = adapter(&f, "T2");
    write(&a, &key("s1"), &item("T2", "q"));
    a.finish().unwrap();
    assert_eq!(theirs, all_rows_hash(&f, &[&id]));
    assert_eq!(count(&f), 3);
    let source = include_str!("../src/adapters/codex_threads.rs");
    assert!(!source.to_lowercase().contains("delete from"));
}

#[test]
fn backups_are_online_copies_and_only_five_are_kept() {
    let f = fx();
    for n in 1..=7 {
        let a = adapter(&f, &format!("2026110{n}T000000Z"));
        write(&a, &key("s1"), &item(&format!("t{n}"), "q"));
        a.finish().unwrap();
    }
    let mut kept: Vec<String> = fs::read_dir(f.state.join("backups"))
        .unwrap()
        .flatten()
        .filter(|e| e.path().join("codex/state_5.sqlite").is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    kept.sort();
    assert_eq!(kept.len(), 5);
    assert_eq!(kept[0], "20261103T000000Z");
    let copy = Connection::open(
        f.state
            .join("backups/20261107T000000Z/codex/state_5.sqlite"),
    )
    .unwrap();
    let title: String = copy
        .query_row("SELECT title FROM threads", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        title, "t6",
        "the copy holds the state before that run's write"
    );
    // Other files in the state directory are not backups of the database and stay.
    fs::write(
        f.state.join("backups/20261107T000000Z/codex/AGENTS.md"),
        "x",
    )
    .unwrap();
    let a = adapter(&f, "20261108T000000Z");
    write(&a, &key("s1"), &item("t8", "q"));
    a.finish().unwrap();
    assert!(f
        .state
        .join("backups/20261107T000000Z/codex/AGENTS.md")
        .is_file());
    assert!(!f
        .state
        .join("backups/20261103T000000Z/codex/state_5.sqlite")
        .exists());
}

#[test]
fn a_busy_database_is_skipped_and_repaired_on_the_next_run() {
    let f = fx();
    let a = adapter(&f, "T1");
    let (id, path) = write(&a, &key("s1"), &item("T", "q"));
    let locker = Connection::open(f.home.join("state_5.sqlite")).unwrap();
    locker.execute_batch("BEGIN EXCLUSIVE").unwrap();
    let skipped = sync_rows(
        &f.home,
        &f.state,
        &BackupSession::with_stamp("T1"),
        std::slice::from_ref(&path),
        Duration::from_millis(100),
    )
    .unwrap();
    assert!(skipped.is_some());
    assert!(path.is_file(), "the rollout stays");
    locker.execute_batch("ROLLBACK").unwrap();
    drop(locker);
    assert!(row(&f, &id).is_none());
    adapter(&f, "T2").finish().unwrap();
    assert!(row(&f, &id).is_some());
}

#[test]
fn a_write_ahead_log_database_keeps_its_journal_mode() {
    let f = fx();
    let mode: String = db(&f)
        .query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0))
        .unwrap();
    assert_eq!(mode, "wal");
    let keeper = db(&f);
    keeper
        .query_row("SELECT count(*) FROM threads", [], |r| r.get::<_, i64>(0))
        .unwrap();
    let a = adapter(&f, "T1");
    let (id, _) = write(&a, &key("s1"), &item("T", "q"));
    assert!(a.finish().unwrap().is_empty());
    assert!(row(&f, &id).is_some());
    let mode: String = keeper
        .query_row("PRAGMA journal_mode", [], |r| r.get(0))
        .unwrap();
    assert_eq!(mode, "wal");
}

#[test]
fn schema_drift_or_paused_sessions_write_no_row_and_take_no_backup() {
    let f = fx();
    fs::remove_file(f.home.join("state_5.sqlite")).unwrap();
    CodexHome::new(&f.home).write_state_db(5, &["brand_new INTEGER NOT NULL DEFAULT 0"]);
    let probing = adapter(&f, "T1");
    let failing: Vec<String> = probing
        .probes()
        .into_iter()
        .filter(|c| !c.ok)
        .map(|c| c.id)
        .collect();
    assert_eq!(failing, vec!["codex.threads_columns"]);
    let project = Scope::Project("/w/app".into());
    let paused = adapter(&f, "T1").with_blocked_kinds(&[Kind::Session]);
    assert!(!paused.supports(Kind::Session, &project));
    let (id, _) = write(&probing, &key("s1"), &item("T", "q"));
    paused.finish().unwrap();
    let no_sessions = CodexAdapter::new(
        f.home.clone(),
        f.state.clone(),
        BackupSession::with_stamp("T1"),
    );
    no_sessions.finish().unwrap();
    assert!(row(&f, &id).is_none());
    assert!(!f.state.join("backups").exists());
}
