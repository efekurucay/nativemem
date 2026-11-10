use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::error::{Error, Result};
use crate::model::{AgentId, ItemKey, Kind, Scope};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS items(
    origin TEXT NOT NULL,
    native_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    scope TEXT NOT NULL,
    hash TEXT NOT NULL,
    PRIMARY KEY(origin, native_id)
);
CREATE TABLE IF NOT EXISTS replicas(
    origin TEXT NOT NULL,
    native_id TEXT NOT NULL,
    agent TEXT NOT NULL,
    replica_native_id TEXT NOT NULL,
    path TEXT NOT NULL,
    hash TEXT NOT NULL,
    PRIMARY KEY(origin, native_id, agent),
    FOREIGN KEY(origin, native_id) REFERENCES items(origin, native_id) ON DELETE CASCADE
);
";

/// `hash` is the content hash at the last successful sync.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemRow {
    pub key: ItemKey,
    pub kind: Kind,
    pub scope: Scope,
    pub hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicaRow {
    pub key: ItemKey,
    pub agent: AgentId,
    pub replica_native_id: String,
    pub path: PathBuf,
    pub hash: String,
    /// The adapter's writer revision when the replica was written; rows from before revisions
    /// existed count as 0.
    pub revision: u32,
}

pub struct Ledger {
    conn: Connection,
}

fn scope_to_text(scope: &Scope) -> String {
    match scope {
        Scope::Global => "global".to_string(),
        Scope::Project(p) => format!("project:{}", p.to_string_lossy()),
    }
}

fn scope_from_text(s: &str) -> Result<Scope> {
    if s == "global" {
        Ok(Scope::Global)
    } else if let Some(p) = s.strip_prefix("project:") {
        Ok(Scope::Project(PathBuf::from(p)))
    } else {
        Err(Error::Parse(format!("bad scope in ledger: {s}")))
    }
}

fn item_from_row(r: &Row) -> rusqlite::Result<(String, String, String, String, String)> {
    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
}

fn build_item(
    (origin, native_id, kind, scope, hash): (String, String, String, String, String),
) -> Result<ItemRow> {
    Ok(ItemRow {
        key: ItemKey {
            origin: AgentId(origin),
            native_id,
        },
        kind: Kind::parse(&kind)
            .ok_or_else(|| Error::Parse(format!("bad kind in ledger: {kind}")))?,
        scope: scope_from_text(&scope)?,
        hash,
    })
}

fn replica_from_row(r: &Row) -> rusqlite::Result<ReplicaRow> {
    Ok(ReplicaRow {
        key: ItemKey {
            origin: AgentId(r.get(0)?),
            native_id: r.get(1)?,
        },
        agent: AgentId(r.get(2)?),
        replica_native_id: r.get(3)?,
        path: PathBuf::from(r.get::<_, String>(4)?),
        hash: r.get(5)?,
        revision: r.get(6)?,
    })
}

/// Ledgers created before writer revisions lack the column; their rows read as revision 0.
fn add_revision_column(conn: &Connection) -> Result<()> {
    let has = conn
        .prepare("PRAGMA table_info(replicas)")?
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .iter()
        .any(|c| c == "revision");
    if !has {
        conn.execute(
            "ALTER TABLE replicas ADD COLUMN revision INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }
    Ok(())
}

const REPLICA_COLS: &str = "origin, native_id, agent, replica_native_id, path, hash, revision";

impl Ledger {
    /// Opens (creating if needed) `<state_dir>/ledger.db`.
    pub fn open(state_dir: &Path) -> Result<Ledger> {
        std::fs::create_dir_all(state_dir).map_err(|e| Error::io(state_dir, e))?;
        let conn = Connection::open(state_dir.join("ledger.db"))?;
        conn.pragma_update(None, "foreign_keys", true)?;
        conn.execute_batch(SCHEMA)?;
        add_revision_column(&conn)?;
        Ok(Ledger { conn })
    }

    /// A throwaway ledger for dry runs that must not create files.
    pub fn in_memory() -> Result<Ledger> {
        let conn = Connection::open_in_memory()?;
        conn.pragma_update(None, "foreign_keys", true)?;
        conn.execute_batch(SCHEMA)?;
        add_revision_column(&conn)?;
        Ok(Ledger { conn })
    }

    pub fn get_item(&self, key: &ItemKey) -> Result<Option<ItemRow>> {
        let row = self
            .conn
            .query_row(
                "SELECT origin, native_id, kind, scope, hash FROM items WHERE origin=?1 AND native_id=?2",
                params![key.origin.as_str(), key.native_id],
                item_from_row,
            )
            .optional()?;
        row.map(build_item).transpose()
    }

    pub fn upsert_item(&self, row: &ItemRow) -> Result<()> {
        self.conn.execute(
            "INSERT INTO items(origin, native_id, kind, scope, hash) VALUES(?1,?2,?3,?4,?5)
             ON CONFLICT(origin, native_id) DO UPDATE SET kind=?3, scope=?4, hash=?5",
            params![
                row.key.origin.as_str(),
                row.key.native_id,
                row.kind.as_str(),
                scope_to_text(&row.scope),
                row.hash
            ],
        )?;
        Ok(())
    }

    /// Also removes the item's replicas.
    pub fn delete_item(&self, key: &ItemKey) -> Result<()> {
        self.conn.execute(
            "DELETE FROM items WHERE origin=?1 AND native_id=?2",
            params![key.origin.as_str(), key.native_id],
        )?;
        Ok(())
    }

    pub fn list_items(&self) -> Result<Vec<ItemRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT origin, native_id, kind, scope, hash FROM items ORDER BY origin, native_id",
        )?;
        let rows = stmt
            .query_map([], item_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter().map(build_item).collect()
    }

    pub fn get_replica(&self, key: &ItemKey, agent: &AgentId) -> Result<Option<ReplicaRow>> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {REPLICA_COLS} FROM replicas WHERE origin=?1 AND native_id=?2 AND agent=?3"),
                params![key.origin.as_str(), key.native_id, agent.as_str()],
                replica_from_row,
            )
            .optional()?)
    }

    /// The item row for `row.key` must already exist.
    pub fn upsert_replica(&self, row: &ReplicaRow) -> Result<()> {
        self.conn.execute(
            "INSERT INTO replicas(origin, native_id, agent, replica_native_id, path, hash, revision)
             VALUES(?1,?2,?3,?4,?5,?6,?7)
             ON CONFLICT(origin, native_id, agent)
             DO UPDATE SET replica_native_id=?4, path=?5, hash=?6, revision=?7",
            params![
                row.key.origin.as_str(),
                row.key.native_id,
                row.agent.as_str(),
                row.replica_native_id,
                row.path.to_string_lossy(),
                row.hash,
                row.revision
            ],
        )?;
        Ok(())
    }

    pub fn delete_replica(&self, key: &ItemKey, agent: &AgentId) -> Result<()> {
        self.conn.execute(
            "DELETE FROM replicas WHERE origin=?1 AND native_id=?2 AND agent=?3",
            params![key.origin.as_str(), key.native_id, agent.as_str()],
        )?;
        Ok(())
    }

    pub fn list_replicas(&self, key: &ItemKey) -> Result<Vec<ReplicaRow>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {REPLICA_COLS} FROM replicas WHERE origin=?1 AND native_id=?2 ORDER BY agent"
        ))?;
        let rows = stmt
            .query_map(
                params![key.origin.as_str(), key.native_id],
                replica_from_row,
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn find_by_replica(
        &self,
        agent: &AgentId,
        replica_native_id: &str,
    ) -> Result<Option<ReplicaRow>> {
        Ok(self
            .conn
            .query_row(
                &format!(
                    "SELECT {REPLICA_COLS} FROM replicas WHERE agent=?1 AND replica_native_id=?2"
                ),
                params![agent.as_str(), replica_native_id],
                replica_from_row,
            )
            .optional()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(n: &str) -> ItemKey {
        ItemKey {
            origin: AgentId::new("claude"),
            native_id: n.into(),
        }
    }

    fn item(n: &str, scope: Scope) -> ItemRow {
        ItemRow {
            key: key(n),
            kind: Kind::Memory,
            scope,
            hash: "h1".into(),
        }
    }

    fn replica(n: &str, agent: &str) -> ReplicaRow {
        ReplicaRow {
            key: key(n),
            agent: AgentId::new(agent),
            replica_native_id: format!("{agent}/{n}"),
            path: PathBuf::from(format!("/r/{agent}/{n}.md")),
            hash: "h1".into(),
            revision: 0,
        }
    }

    #[test]
    fn item_crud_and_scope_roundtrip() {
        let t = tempfile::tempdir().unwrap();
        let l = Ledger::open(t.path()).unwrap();
        assert!(t.path().join("ledger.db").exists());
        assert_eq!(l.get_item(&key("a")).unwrap(), None);

        let g = item("a", Scope::Global);
        let p = item("b", Scope::Project(PathBuf::from("/work/proj")));
        l.upsert_item(&g).unwrap();
        l.upsert_item(&p).unwrap();
        assert_eq!(l.get_item(&key("a")).unwrap(), Some(g.clone()));
        assert_eq!(l.get_item(&key("b")).unwrap(), Some(p.clone()));
        assert_eq!(l.list_items().unwrap(), vec![g, p.clone()]);

        let mut updated = p;
        updated.hash = "h2".into();
        l.upsert_item(&updated).unwrap();
        assert_eq!(l.get_item(&key("b")).unwrap().unwrap().hash, "h2");

        l.delete_item(&key("b")).unwrap();
        assert_eq!(l.get_item(&key("b")).unwrap(), None);
    }

    #[test]
    fn replica_crud_find_and_cascade() {
        let t = tempfile::tempdir().unwrap();
        let l = Ledger::open(t.path()).unwrap();
        l.upsert_item(&item("a", Scope::Global)).unwrap();
        let r = replica("a", "codex");
        l.upsert_replica(&r).unwrap();
        assert_eq!(l.get_replica(&key("a"), &r.agent).unwrap(), Some(r.clone()));
        assert_eq!(
            l.find_by_replica(&r.agent, "codex/a").unwrap(),
            Some(r.clone())
        );
        assert_eq!(
            l.find_by_replica(&AgentId::new("other"), "codex/a")
                .unwrap(),
            None
        );

        let mut r2 = r.clone();
        r2.hash = "h9".into();
        l.upsert_replica(&r2).unwrap();
        assert_eq!(l.list_replicas(&key("a")).unwrap(), vec![r2]);

        l.delete_item(&key("a")).unwrap();
        assert!(l.list_replicas(&key("a")).unwrap().is_empty());
        assert_eq!(l.find_by_replica(&r.agent, "codex/a").unwrap(), None);
    }

    #[test]
    fn delete_replica_only() {
        let t = tempfile::tempdir().unwrap();
        let l = Ledger::open(t.path()).unwrap();
        l.upsert_item(&item("a", Scope::Global)).unwrap();
        l.upsert_replica(&replica("a", "codex")).unwrap();
        l.delete_replica(&key("a"), &AgentId::new("codex")).unwrap();
        assert!(l.list_replicas(&key("a")).unwrap().is_empty());
        assert!(l.get_item(&key("a")).unwrap().is_some());
    }

    #[test]
    fn reopen_persists() {
        let t = tempfile::tempdir().unwrap();
        Ledger::open(t.path())
            .unwrap()
            .upsert_item(&item("a", Scope::Global))
            .unwrap();
        assert_eq!(
            Ledger::open(t.path()).unwrap().list_items().unwrap().len(),
            1
        );
    }

    #[test]
    fn ledger_without_revision_column_migrates_and_reads_revision_zero() {
        let t = tempfile::tempdir().unwrap();
        {
            let conn = Connection::open(t.path().join("ledger.db")).unwrap();
            conn.execute_batch(
                "CREATE TABLE items(origin TEXT NOT NULL, native_id TEXT NOT NULL, kind TEXT NOT NULL,
                    scope TEXT NOT NULL, hash TEXT NOT NULL, PRIMARY KEY(origin, native_id));
                 CREATE TABLE replicas(origin TEXT NOT NULL, native_id TEXT NOT NULL, agent TEXT NOT NULL,
                    replica_native_id TEXT NOT NULL, path TEXT NOT NULL, hash TEXT NOT NULL,
                    PRIMARY KEY(origin, native_id, agent),
                    FOREIGN KEY(origin, native_id) REFERENCES items(origin, native_id) ON DELETE CASCADE);
                 INSERT INTO items VALUES('claude','a','session','project:/w','h');
                 INSERT INTO replicas VALUES('claude','a','pi','pi/a','/p/a','h');",
            )
            .unwrap();
        }
        let l = Ledger::open(t.path()).unwrap();
        let r = l
            .get_replica(&key("a"), &AgentId::new("pi"))
            .unwrap()
            .unwrap();
        assert_eq!(r.revision, 0);
        let mut bumped = r;
        bumped.revision = 3;
        l.upsert_replica(&bumped).unwrap();
        drop(l);
        let again = Ledger::open(t.path()).unwrap();
        assert_eq!(
            again
                .get_replica(&key("a"), &AgentId::new("pi"))
                .unwrap()
                .unwrap()
                .revision,
            3
        );
    }
}
