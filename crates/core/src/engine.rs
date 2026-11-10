use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::error::{Error, Result};
use crate::ledger::{ItemRow, Ledger, ReplicaRow};
use crate::model::{content_hash, Adapter, AgentId, Item, ItemKey, Kind, Observed, Scope};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Record a marked entry that the ledger does not know yet; writes nothing.
    Adopt {
        item: ItemRow,
        replica: ReplicaRow,
        name: String,
    },
    /// Create or update the replica of an original in `target`.
    Write {
        key: ItemKey,
        target: AgentId,
        item: Item,
        update: bool,
    },
    /// An edit made on `from`'s replica: update the original, then the other replicas.
    WriteBack {
        key: ItemKey,
        from: AgentId,
        item: Item,
        targets: Vec<AgentId>,
    },
    Remove {
        key: ItemKey,
        agent: AgentId,
        native_id: String,
        scope: Scope,
        name: String,
        original: bool,
    },
    /// Drop the ledger rows of an item whose entries are gone.
    Forget { key: ItemKey },
    /// A session replica the user deleted: keep an empty-hash ledger row so it is never recreated.
    Tombstone {
        replica: ReplicaRow,
        scope: Scope,
        name: String,
    },
}

impl Action {
    fn key(&self) -> &ItemKey {
        match self {
            Action::Adopt { item, .. } => &item.key,
            Action::Write { key, .. }
            | Action::WriteBack { key, .. }
            | Action::Remove { key, .. }
            | Action::Forget { key } => key,
            Action::Tombstone { replica, .. } => &replica.key,
        }
    }

    fn rank(&self) -> u8 {
        match self {
            Action::Adopt { .. } | Action::Tombstone { .. } => 0,
            Action::Write { .. } | Action::WriteBack { .. } => 1,
            Action::Remove { .. } => 2,
            Action::Forget { .. } => 3,
        }
    }

    fn label(&self) -> String {
        match self {
            Action::Adopt { name, .. }
            | Action::Remove { name, .. }
            | Action::Tombstone { name, .. } => name.clone(),
            Action::Write { item, .. } | Action::WriteBack { item, .. } => item_label(item),
            Action::Forget { key } => key.native_id.clone(),
        }
    }
}

fn scope_label(scope: &Scope) -> String {
    match scope {
        Scope::Global => "global".to_string(),
        Scope::Project(p) => p.display().to_string(),
    }
}

/// A session is shown by its title, every other item by its name.
fn item_label(item: &Item) -> String {
    match item.kind {
        Kind::Session => format!("session \"{}\"", item.description),
        _ => item.name.clone(),
    }
}

/// Memory entries are listed with three spaces before the origin, sessions with two.
fn from_gap(kind: Kind) -> &'static str {
    if kind == Kind::Session {
        "  "
    } else {
        "   "
    }
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Action::Adopt {
                item,
                replica,
                name,
            } => write!(
                f,
                "= {}  {}  {}   (adopt replica of {})",
                replica.agent,
                scope_label(&item.scope),
                name,
                item.key.origin
            ),
            Action::Write {
                key,
                target,
                item,
                update,
            } => write!(
                f,
                "{} {}  {}  {}{}(from {})",
                if *update { '~' } else { '+' },
                target,
                scope_label(&item.scope),
                item_label(item),
                from_gap(item.kind),
                key.origin
            ),
            Action::WriteBack {
                key, from, item, ..
            } => write!(
                f,
                "~ {}  {}  {}   (from {})",
                key.origin,
                scope_label(&item.scope),
                item.name,
                from
            ),
            Action::Remove {
                agent, scope, name, ..
            } => write!(f, "- {}  {}  {}", agent, scope_label(scope), name),
            Action::Forget { key } => {
                write!(f, "- ledger  {}:{}", key.origin, key.native_id)
            }
            Action::Tombstone {
                replica,
                scope,
                name,
            } => write!(
                f,
                "x {}  {}  {}  (replica deleted; not recreated)",
                replica.agent,
                scope_label(scope),
                name
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Note {
    Conflict {
        key: ItemKey,
        reason: String,
    },
    Skipped {
        agent: AgentId,
        label: String,
        reason: String,
    },
}

impl fmt::Display for Note {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Note::Conflict { key, reason } => {
                write!(
                    f,
                    "! {}  {}   conflict: {reason}",
                    key.origin, key.native_id
                )
            }
            Note::Skipped {
                agent,
                label,
                reason,
            } => write!(f, "? {agent}  {label}   {reason}"),
        }
    }
}

struct Counts {
    add: usize,
    update: usize,
    remove: usize,
    conflicts: usize,
    skipped: usize,
}

fn count(actions: &[Action], notes: &[Note]) -> Counts {
    let mut c = Counts {
        add: 0,
        update: 0,
        remove: 0,
        conflicts: 0,
        skipped: 0,
    };
    for a in actions {
        match a {
            Action::Write { update: false, .. } => c.add += 1,
            Action::Write { update: true, .. } | Action::WriteBack { .. } => c.update += 1,
            Action::Remove { .. } => c.remove += 1,
            Action::Adopt { .. } | Action::Forget { .. } | Action::Tombstone { .. } => {}
        }
    }
    for n in notes {
        match n {
            Note::Conflict { .. } => c.conflicts += 1,
            Note::Skipped { .. } => c.skipped += 1,
        }
    }
    c
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Plan {
    pub actions: Vec<Action>,
    pub notes: Vec<Note>,
}

impl Plan {
    pub fn is_empty(&self) -> bool {
        self.actions.is_empty() && self.notes.is_empty()
    }

    pub fn summary(&self) -> String {
        let c = count(&self.actions, &self.notes);
        format!(
            "{} to add, {} to update, {} to remove, {} conflicts, {} skipped",
            c.add, c.update, c.remove, c.conflicts, c.skipped
        )
    }

    /// True when a conflict or skipped entry needs the user's attention.
    pub fn needs_attention(&self) -> bool {
        !self.notes.is_empty()
    }
}

impl fmt::Display for Plan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for a in &self.actions {
            writeln!(f, "{a}")?;
        }
        for n in &self.notes {
            writeln!(f, "{n}")?;
        }
        Ok(())
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Report {
    pub done: Vec<Action>,
    pub notes: Vec<Note>,
}

impl Report {
    pub fn summary(&self) -> String {
        let c = count(&self.done, &self.notes);
        format!(
            "{} added, {} updated, {} removed, {} conflicts, {} skipped",
            c.add, c.update, c.remove, c.conflicts, c.skipped
        )
    }

    pub fn needs_attention(&self) -> bool {
        !self.notes.is_empty()
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for a in &self.done {
            writeln!(f, "{a}")?;
        }
        for n in &self.notes {
            writeln!(f, "{n}")?;
        }
        Ok(())
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct ApplyOptions {
    pub allow_mass_delete: bool,
}

type Replicas<'a> = BTreeMap<(ItemKey, AgentId), &'a Observed>;

struct Side<'a> {
    agent: &'a AgentId,
    row: Option<ReplicaRow>,
    obs: Option<&'a Observed>,
}

impl Side<'_> {
    fn changed(&self) -> bool {
        match (&self.row, self.obs) {
            (Some(r), Some(o)) => content_hash(&o.item) != r.hash,
            _ => false,
        }
    }
}

fn scan_all(adapters: &[&dyn Adapter]) -> Result<Vec<Observed>> {
    let mut observed = Vec::new();
    for a in adapters {
        observed.extend(a.scan()?);
    }
    Ok(observed)
}

fn index(observed: &[Observed]) -> (BTreeMap<ItemKey, &Observed>, Replicas<'_>) {
    let mut originals = BTreeMap::new();
    let mut replicas = BTreeMap::new();
    for o in observed {
        match &o.replica_of {
            None => {
                let key = ItemKey {
                    origin: o.agent.clone(),
                    native_id: o.native_id.clone(),
                };
                originals.insert(key, o);
            }
            Some(key) => {
                replicas.insert((key.clone(), o.agent.clone()), o);
            }
        }
    }
    (originals, replicas)
}

pub fn plan(adapters: &[&dyn Adapter], ledger: &Ledger) -> Result<Plan> {
    let observed = scan_all(adapters)?;
    let (originals, replicas) = index(&observed);

    let mut keys: BTreeSet<ItemKey> = originals.keys().cloned().collect();
    keys.extend(replicas.keys().map(|(k, _)| k.clone()));
    keys.extend(ledger.list_items()?.into_iter().map(|r| r.key));

    let mut plan = Plan::default();
    for key in keys {
        if adapters.iter().any(|a| a.id() == key.origin) {
            plan_key(adapters, &originals, &replicas, ledger, &key, &mut plan)?;
        }
    }
    Ok(plan)
}

fn plan_key(
    adapters: &[&dyn Adapter],
    originals: &BTreeMap<ItemKey, &Observed>,
    replicas: &Replicas<'_>,
    ledger: &Ledger,
    key: &ItemKey,
    plan: &mut Plan,
) -> Result<()> {
    let item_row = ledger.get_item(key)?;
    let Some((kind, scope)) = originals
        .get(key)
        .map(|o| (o.item.kind, o.item.scope.clone()))
        .or_else(|| item_row.as_ref().map(|r| (r.kind, r.scope.clone())))
        .or_else(|| {
            replicas
                .iter()
                .find(|((k, _), _)| k == key)
                .map(|(_, o)| (o.item.kind, o.item.scope.clone()))
        })
    else {
        return Ok(());
    };
    if kind == Kind::Session {
        return plan_session(adapters, originals, replicas, ledger, key, &scope, plan);
    }
    // An origin that cannot store this item (its kind is paused) is left alone: nothing is
    // written back to it or removed from it.
    if adapters
        .iter()
        .any(|a| a.id() == key.origin && !a.supports(kind, &scope))
    {
        return Ok(());
    }
    // A paused agent that already holds a replica freezes the item for every agent: its copy
    // cannot be read, so nothing may be updated, removed or forgotten until it resumes.
    for a in adapters.iter().filter(|a| !a.supports(kind, &scope)) {
        if a.id() != key.origin && ledger.get_replica(key, &a.id())?.is_some() {
            return Ok(());
        }
    }
    // Agents that cannot store this item are not part of it at all.
    let ids: Vec<AgentId> = adapters
        .iter()
        .filter(|a| a.supports(kind, &scope))
        .map(|a| a.id())
        .collect();
    let mut sides = Vec::new();
    for agent in ids.iter().filter(|a| **a != key.origin) {
        sides.push(Side {
            agent,
            row: ledger.get_replica(key, agent)?,
            obs: replicas.get(&(key.clone(), agent.clone())).copied(),
        });
    }
    let conflict = |plan: &mut Plan, reason: &str| {
        plan.notes.push(Note::Conflict {
            key: key.clone(),
            reason: reason.to_string(),
        })
    };

    let Some(orig) = originals.get(key).copied() else {
        for s in &sides {
            if let (None, Some(o)) = (&s.row, s.obs) {
                plan.notes.push(Note::Skipped {
                    agent: o.agent.clone(),
                    label: o.native_id.clone(),
                    reason: format!(
                        "orphan replica: original {}:{} not found",
                        key.origin, key.native_id
                    ),
                });
            }
        }
        if item_row.is_none() {
            return Ok(());
        }
        let tracked: Vec<&Side> = sides.iter().filter(|s| s.row.is_some()).collect();
        if tracked.iter().any(|s| s.changed()) {
            conflict(plan, "original missing, replica edited");
            return Ok(());
        }
        for s in tracked {
            if let Some(o) = s.obs {
                plan.actions.push(remove_action(key, o, false));
            }
        }
        plan.actions.push(Action::Forget { key: key.clone() });
        return Ok(());
    };

    let cur = content_hash(&orig.item);
    let base = item_row.map_or_else(|| cur.clone(), |r| r.hash);
    let orig_changed = cur != base;

    let mut missing = false;
    let mut changed: Vec<&Side> = Vec::new();
    let mut present: Vec<&Side> = Vec::new();
    let mut absent: Vec<&AgentId> = Vec::new();
    for s in &sides {
        match (&s.row, s.obs) {
            (Some(_), Some(_)) if s.changed() => changed.push(s),
            (Some(_), Some(_)) => present.push(s),
            (Some(_), None) => missing = true,
            (None, Some(o)) => {
                plan.actions.push(Action::Adopt {
                    item: ItemRow {
                        key: key.clone(),
                        kind: orig.item.kind,
                        scope: orig.item.scope.clone(),
                        hash: base.clone(),
                    },
                    replica: ReplicaRow {
                        key: key.clone(),
                        agent: o.agent.clone(),
                        replica_native_id: o.native_id.clone(),
                        path: o.path.clone(),
                        hash: content_hash(&o.item),
                        revision: adapters
                            .iter()
                            .find(|a| a.id() == o.agent)
                            .map_or(0, |a| a.writer_revision(o.item.kind)),
                    },
                    name: o.item.name.clone(),
                });
                present.push(s);
            }
            (None, None) => absent.push(s.agent),
        }
    }

    if missing {
        if orig_changed || !changed.is_empty() {
            conflict(plan, "replica deleted, other side edited");
            return Ok(());
        }
        plan.actions.push(remove_action(key, orig, true));
        for s in &present {
            if let Some(o) = s.obs {
                plan.actions.push(remove_action(key, o, false));
            }
        }
        plan.actions.push(Action::Forget { key: key.clone() });
        return Ok(());
    }
    if changed.len() > 1 || (changed.len() == 1 && orig_changed) {
        conflict(plan, "edited on both sides");
        return Ok(());
    }
    if let Some(c) = changed.first() {
        // The editing side is also a target: re-running its `write_replica` is
        // idempotent for the file content (write_if_changed skips) but regenerates
        // that agent's index so a renamed replica does not leave a stale entry.
        let mut targets: Vec<AgentId> = present
            .iter()
            .map(|s| s.agent.clone())
            .chain(absent.iter().map(|a| (*a).clone()))
            .collect();
        targets.push(c.agent.clone());
        plan.actions.push(Action::WriteBack {
            key: key.clone(),
            from: c.agent.clone(),
            item: c.obs.map_or_else(
                || orig.item.clone(),
                |o| match o.item.kind {
                    // The heading is the identity of an instruction section; only the body travels.
                    Kind::Instruction => Item {
                        name: orig.item.name.clone(),
                        description: orig.item.description.clone(),
                        ..o.item.clone()
                    },
                    Kind::Memory | Kind::Session => o.item.clone(),
                },
            ),
            targets,
        });
        return Ok(());
    }
    if orig_changed {
        for s in &present {
            plan.actions.push(Action::Write {
                key: key.clone(),
                target: s.agent.clone(),
                item: orig.item.clone(),
                update: true,
            });
        }
    }
    for agent in absent {
        plan.actions.push(Action::Write {
            key: key.clone(),
            target: agent.clone(),
            item: orig.item.clone(),
            update: false,
        });
    }
    Ok(())
}

/// Snapshot policy: a replica is created once per target agent and refreshed only while it is
/// unmodified and the original has changed. Nothing ever flows back to the original, and a
/// missing original or a replica the user deleted never causes a removal or a recreation.
fn plan_session(
    adapters: &[&dyn Adapter],
    originals: &BTreeMap<ItemKey, &Observed>,
    replicas: &Replicas<'_>,
    ledger: &Ledger,
    key: &ItemKey,
    scope: &Scope,
    plan: &mut Plan,
) -> Result<()> {
    let Some(orig) = originals.get(key).copied() else {
        return Ok(());
    };
    let cur = content_hash(&orig.item);
    let grown = ledger.get_item(key)?.is_some_and(|row| row.hash != cur);
    let label = item_label(&orig.item);
    for adapter in adapters {
        let agent = adapter.id();
        if agent == key.origin || !adapter.supports(Kind::Session, scope) {
            continue;
        }
        let obs = replicas.get(&(key.clone(), agent.clone())).copied();
        match (ledger.get_replica(key, &agent)?, obs) {
            (None, None) => plan.actions.push(Action::Write {
                key: key.clone(),
                target: agent,
                item: orig.item.clone(),
                update: false,
            }),
            (None, Some(o)) => plan.actions.push(Action::Adopt {
                item: ItemRow {
                    key: key.clone(),
                    kind: Kind::Session,
                    scope: scope.clone(),
                    hash: cur.clone(),
                },
                replica: ReplicaRow {
                    key: key.clone(),
                    agent,
                    replica_native_id: o.native_id.clone(),
                    path: o.path.clone(),
                    // An empty hash freezes a replica another program wrote.
                    hash: if adapter.is_foreign_replica(o) {
                        String::new()
                    } else {
                        content_hash(&o.item)
                    },
                    revision: adapter.writer_revision(Kind::Session),
                },
                name: label.clone(),
            }),
            (Some(row), _) if row.hash.is_empty() => {}
            // A file that exists but was not scanned (outside the window, too large) is not deleted.
            (Some(mut row), None) if !row.path.exists() => {
                row.hash = String::new();
                plan.actions.push(Action::Tombstone {
                    replica: row,
                    scope: scope.clone(),
                    name: label.clone(),
                });
            }
            (Some(_), None) => {}
            (Some(row), Some(o)) => {
                let outdated = row.revision < adapter.writer_revision(Kind::Session);
                if (grown || outdated) && content_hash(&o.item) == row.hash {
                    plan.actions.push(Action::Write {
                        key: key.clone(),
                        target: agent,
                        item: orig.item.clone(),
                        update: true,
                    });
                }
            }
        }
    }
    Ok(())
}

fn remove_action(key: &ItemKey, o: &Observed, original: bool) -> Action {
    Action::Remove {
        key: key.clone(),
        agent: o.agent.clone(),
        native_id: o.native_id.clone(),
        scope: o.item.scope.clone(),
        name: o.item.name.clone(),
        original,
    }
}

fn adapter_for<'a>(adapters: &'a [&'a dyn Adapter], id: &AgentId) -> Result<&'a dyn Adapter> {
    adapters
        .iter()
        .find(|a| a.id() == *id)
        .copied()
        .ok_or_else(|| Error::Parse(format!("no adapter for agent {id}")))
}

fn write_replica(adapter: &dyn Adapter, ledger: &Ledger, key: &ItemKey, item: &Item) -> Result<()> {
    let (native_id, path) = adapter.write_replica(key, item)?;
    let hash = content_hash(item);
    ledger.upsert_item(&ItemRow {
        key: key.clone(),
        kind: item.kind,
        scope: item.scope.clone(),
        hash: hash.clone(),
    })?;
    ledger.upsert_replica(&ReplicaRow {
        key: key.clone(),
        agent: adapter.id(),
        replica_native_id: native_id,
        path,
        hash,
        revision: adapter.writer_revision(item.kind),
    })
}

/// What a run changed: the items it touched and the session replicas it wrote.
#[derive(Default)]
struct Touched {
    keys: BTreeSet<ItemKey>,
    written: BTreeSet<(ItemKey, AgentId)>,
}

fn run(
    action: &Action,
    adapters: &[&dyn Adapter],
    ledger: &Ledger,
    touched: &mut Touched,
) -> Result<()> {
    match action {
        Action::Adopt { item, replica, .. } => {
            ledger.upsert_item(item)?;
            ledger.upsert_replica(replica)?;
            touched.keys.insert(item.key.clone());
        }
        Action::Write {
            key, target, item, ..
        } => {
            write_replica(adapter_for(adapters, target)?, ledger, key, item)?;
            touched.keys.insert(key.clone());
            touched.written.insert((key.clone(), target.clone()));
        }
        Action::WriteBack {
            key, item, targets, ..
        } => {
            adapter_for(adapters, &key.origin)?.write_original(&key.native_id, item)?;
            ledger.upsert_item(&ItemRow {
                key: key.clone(),
                kind: item.kind,
                scope: item.scope.clone(),
                hash: content_hash(item),
            })?;
            touched.keys.insert(key.clone());
            for t in targets {
                write_replica(adapter_for(adapters, t)?, ledger, key, item)?;
            }
        }
        Action::Remove {
            key,
            agent,
            native_id,
            original,
            ..
        } => {
            adapter_for(adapters, agent)?.remove(native_id)?;
            if !original {
                ledger.delete_replica(key, agent)?;
            }
        }
        Action::Forget { key } => ledger.delete_item(key)?,
        Action::Tombstone { replica, .. } => ledger.upsert_replica(replica)?,
    }
    Ok(())
}

/// Records what is actually on disk for every item the run touched, so formats that do not
/// round-trip byte-identically never look like fresh edits.
fn refresh(adapters: &[&dyn Adapter], ledger: &Ledger, touched: &Touched) -> Result<()> {
    if touched.keys.is_empty() {
        return Ok(());
    }
    let observed = scan_all(adapters)?;
    let (originals, replicas) = index(&observed);
    for key in &touched.keys {
        let Some(mut row) = ledger.get_item(key)? else {
            continue;
        };
        if let Some(o) = originals.get(key) {
            row.hash = content_hash(&o.item);
            ledger.upsert_item(&row)?;
        }
        for mut r in ledger.list_replicas(key)? {
            // A session replica the run left alone keeps the hash it was last written with, so
            // an edit made to it stays visible.
            if row.kind == Kind::Session
                && !touched.written.contains(&(key.clone(), r.agent.clone()))
            {
                continue;
            }
            if let Some(o) = replicas.get(&(key.clone(), r.agent.clone())) {
                r.hash = content_hash(&o.item);
                r.replica_native_id = o.native_id.clone();
                r.path = o.path.clone();
                ledger.upsert_replica(&r)?;
            }
        }
    }
    Ok(())
}

fn execute(
    actions: &[Action],
    adapters: &[&dyn Adapter],
    ledger: &Ledger,
    report: &mut Report,
    touched: &mut Touched,
) -> Result<()> {
    // Keys with a skipped operation keep their ledger rows and their remaining removals.
    let mut blocked: BTreeSet<ItemKey> = BTreeSet::new();
    for action in actions {
        if matches!(action, Action::Remove { .. } | Action::Forget { .. })
            && blocked.contains(action.key())
        {
            continue;
        }
        match run(action, adapters, ledger, touched) {
            Ok(()) => report.done.push(action.clone()),
            Err(Error::Unsupported { agent, operation }) => {
                blocked.insert(action.key().clone());
                report.notes.push(Note::Skipped {
                    agent: AgentId(agent),
                    label: action.label(),
                    reason: format!("skipped: {operation}"),
                });
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

fn exceeds_mass_delete_limit(removes: usize, total: usize) -> bool {
    removes > 10 || (removes > 3 && removes * 5 > total)
}

pub fn apply(
    plan: Plan,
    adapters: &[&dyn Adapter],
    ledger: &Ledger,
    opts: &ApplyOptions,
) -> Result<Report> {
    let planned = plan
        .actions
        .iter()
        .filter(|a| matches!(a, Action::Remove { .. }))
        .count();
    let total = ledger
        .list_items()?
        .iter()
        .filter(|r| r.kind != Kind::Session)
        .count();
    if !opts.allow_mass_delete && exceeds_mass_delete_limit(planned, total) {
        return Err(Error::MassDelete { planned, total });
    }

    let mut actions = plan.actions;
    actions.sort_by_key(Action::rank);
    let mut report = Report {
        done: Vec::new(),
        notes: plan.notes,
    };
    let mut touched = Touched::default();
    let outcome = execute(&actions, adapters, ledger, &mut report, &mut touched);
    let refreshed = refresh(adapters, ledger, &touched);
    outcome?;
    refreshed?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::adapters::agents_file::AgentsFileAdapter;
    use crate::adapters::claude::ClaudeAdapter;
    use crate::adapters::codex::CodexAdapter;
    use crate::adapters::pi::PiAdapter;
    use crate::fsx::BackupSession;
    use crate::testkit::{ClaudeHome, CodexHome, FrontmatterVariant};

    const CWD: &str = "/w/app";

    struct Fx {
        _t: tempfile::TempDir,
        claude_root: PathBuf,
        codex_root: PathBuf,
        state: PathBuf,
        ledger: Ledger,
        claude: ClaudeAdapter,
        codex: CodexAdapter,
        ch: ClaudeHome,
        xh: CodexHome,
        app_dir: String,
    }

    fn fx() -> Fx {
        let t = tempfile::tempdir().unwrap();
        let claude_root = t.path().join("claude");
        let codex_root = t.path().join("codex");
        let state = t.path().join("state");
        fs::create_dir_all(&claude_root).unwrap();
        fs::create_dir_all(&codex_root).unwrap();
        let session = BackupSession::with_stamp("T");
        let ch = ClaudeHome::new(&claude_root);
        let app_dir = ch.add_project(Path::new(CWD));
        Fx {
            claude: ClaudeAdapter::new(claude_root.clone(), state.clone(), session.clone())
                .with_fs_root(state.join("no-fs-root")),
            codex: CodexAdapter::new(codex_root.clone(), state.clone(), session),
            xh: CodexHome::new(&codex_root),
            ledger: Ledger::in_memory().unwrap(),
            ch,
            claude_root,
            codex_root,
            state,
            app_dir,
            _t: t,
        }
    }

    impl Fx {
        fn adapters(&self) -> [&dyn Adapter; 2] {
            [&self.claude, &self.codex]
        }

        fn plan(&self) -> Plan {
            plan(&self.adapters(), &self.ledger).unwrap()
        }

        fn try_sync(&self, allow_mass_delete: bool) -> Result<Report> {
            apply(
                self.plan(),
                &self.adapters(),
                &self.ledger,
                &ApplyOptions { allow_mass_delete },
            )
        }

        fn sync(&self) -> Report {
            let r = self.try_sync(false).unwrap();
            let p = self.plan();
            assert!(p.is_empty(), "plan not empty after apply:\n{p}");
            r
        }

        fn add(&self, file: &str, name: &str, category: &str, body: &str) -> PathBuf {
            self.ch.add_memory(
                &self.app_dir,
                file,
                FrontmatterVariant::Nested,
                name,
                &format!("about {name}"),
                category,
                body,
            )
        }

        fn fillers(&self) {
            for i in 0..6 {
                self.add(&format!("f{i}.md"), &format!("filler{i}"), "user", "x");
            }
        }

        fn claude_mem(&self) -> PathBuf {
            self.claude_root
                .join("projects")
                .join(&self.app_dir)
                .join("memory")
        }

        fn codex_replica(&self, name: &str) -> Option<PathBuf> {
            self.codex
                .scan()
                .unwrap()
                .into_iter()
                .find(|o| o.replica_of.is_some() && o.item.name == name)
                .map(|o| o.path)
        }

        fn agents_md(&self) -> String {
            fs::read_to_string(self.codex_root.join("AGENTS.md")).unwrap_or_default()
        }

        fn snapshot(&self) -> BTreeMap<String, Vec<u8>> {
            let mut out = BTreeMap::new();
            for root in [&self.claude_root, &self.codex_root] {
                walk(root, root, &mut out);
            }
            out
        }
    }

    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        for e in fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(root, &p, out);
            } else {
                let rel = format!(
                    "{}:{}",
                    root.display(),
                    p.strip_prefix(root).unwrap().display()
                );
                out.insert(rel, fs::read(&p).unwrap());
            }
        }
    }

    fn replace_in(path: &Path, from: &str, to: &str) {
        let text = fs::read_to_string(path).unwrap();
        assert!(text.contains(from), "{from:?} not in {text}");
        fs::write(path, text.replace(from, to)).unwrap();
    }

    #[test]
    fn new_claude_memory_reaches_codex() {
        let f = fx();
        f.add("a.md", "alpha", "user", "alpha body");
        let p = f.plan();
        assert!(p
            .to_string()
            .contains("+ codex  /w/app  alpha   (from claude)"));
        let r = f.sync();
        assert_eq!(r.done.len(), 1);
        let replica = f.codex_replica("alpha").unwrap();
        assert!(fs::read_to_string(replica).unwrap().contains("alpha body"));
        let agents = f.agents_md();
        assert!(agents.contains("nativemem:begin") && agents.contains("/w/app"));
        assert_eq!(f.ledger.list_items().unwrap().len(), 1);
    }

    #[test]
    fn original_edit_updates_replica() {
        let f = fx();
        let orig = f.add("a.md", "alpha", "user", "alpha body");
        f.sync();
        replace_in(&orig, "alpha body", "changed body");
        assert!(f.plan().to_string().contains("~ codex"));
        f.sync();
        let replica = f.codex_replica("alpha").unwrap();
        assert!(fs::read_to_string(replica)
            .unwrap()
            .contains("changed body"));
    }

    #[test]
    fn replica_edit_updates_original_including_category() {
        let f = fx();
        let orig = f.add("a.md", "alpha", "user", "alpha body");
        f.sync();
        let replica = f.codex_replica("alpha").unwrap();
        replace_in(&replica, "alpha body", "edited in codex");
        replace_in(&replica, "Category: user", "Category: feedback");
        f.sync();
        let text = fs::read_to_string(orig).unwrap();
        assert!(text.contains("edited in codex"));
        assert!(text.contains("type: feedback"));
    }

    #[test]
    fn replica_name_edit_regenerates_replica_side_index() {
        let f = fx();
        f.add("a.md", "alpha", "user", "alpha body");
        f.sync();
        let replica = f.codex_replica("alpha").unwrap();
        let index_path = replica.parent().unwrap().join("MEMORY.md");
        assert!(fs::read_to_string(&index_path)
            .unwrap()
            .contains("- [alpha]"));

        // Rename the replica's heading on the Codex side, then sync.
        replace_in(&replica, "# alpha", "# beta");
        f.sync();

        let index = fs::read_to_string(&index_path).unwrap();
        assert!(
            index.contains("- [beta]"),
            "codex index not updated: {index}"
        );
        assert!(!index.contains("- [alpha]"), "codex index stale: {index}");
        // The original on Claude is updated too.
        let claude_index = fs::read_to_string(f.claude_mem().join("MEMORY.md")).unwrap();
        assert!(claude_index.contains("- [beta]"));
    }

    #[test]
    fn deleting_original_removes_replica_and_routing() {
        let f = fx();
        f.fillers();
        let solo_dir = f.ch.add_project(Path::new("/w/solo"));
        let solo = f.ch.add_memory(
            &solo_dir,
            "s.md",
            FrontmatterVariant::Nested,
            "solo",
            "about solo",
            "user",
            "s",
        );
        f.sync();
        assert!(f.agents_md().contains("/w/solo"));

        fs::remove_file(solo).unwrap();
        f.sync();
        assert!(f.codex_replica("solo").is_none());
        let agents = f.agents_md();
        assert!(!agents.contains("/w/solo") && agents.contains("/w/app"));

        for e in fs::read_dir(f.claude_mem()).unwrap() {
            let p = e.unwrap().path();
            if p.file_name().unwrap() != "MEMORY.md" {
                fs::remove_file(p).unwrap();
            }
        }
        assert!(matches!(f.try_sync(false), Err(Error::MassDelete { .. })));
        f.try_sync(true).unwrap();
        assert!(f.plan().is_empty());
        assert!(!f.agents_md().contains("nativemem:begin"));
    }

    #[test]
    fn deleting_replica_removes_original_with_backup() {
        let f = fx();
        f.fillers();
        let orig = f.add("a.md", "alpha", "user", "alpha body");
        f.sync();
        fs::remove_file(f.codex_replica("alpha").unwrap()).unwrap();
        f.sync();
        assert!(!orig.exists());
        let index = fs::read_to_string(f.claude_mem().join("MEMORY.md")).unwrap();
        assert!(!index.contains("a.md") && index.contains("f0.md"));
        let backup = f
            .state
            .join("backups/T/claude/projects")
            .join(&f.app_dir)
            .join("memory/a.md");
        assert!(backup.exists());
        assert_eq!(f.ledger.list_items().unwrap().len(), 6);
    }

    #[test]
    fn both_sides_edited_is_a_conflict() {
        let f = fx();
        let orig = f.add("a.md", "alpha", "user", "alpha body");
        f.sync();
        replace_in(&orig, "alpha body", "claude edit");
        replace_in(
            &f.codex_replica("alpha").unwrap(),
            "alpha body",
            "codex edit",
        );
        let before = f.snapshot();
        let p = f.plan();
        assert!(p.actions.is_empty());
        assert!(matches!(p.notes[..], [Note::Conflict { .. }]));
        let r = f.try_sync(false).unwrap();
        assert!(r.needs_attention());
        assert_eq!(before, f.snapshot());
        assert!(!f.plan().is_empty());
    }

    const TASK_GROUP: &str =
        "# Task Group: Fix Login\napplies_to: cwd=/w/app; reuse_rule=x\nsome notes\n";

    #[test]
    fn codex_original_replicates_to_claude_and_cannot_be_written_back() {
        let f = fx();
        f.fillers();
        f.xh.write_memory_md(TASK_GROUP);
        f.sync();
        let replica = f
            .claude
            .scan()
            .unwrap()
            .into_iter()
            .find(|o| o.replica_of.is_some())
            .unwrap();
        assert_eq!(replica.item.description, "Fix Login");
        let index = fs::read_to_string(f.claude_mem().join("MEMORY.md")).unwrap();
        assert!(index.contains("fix-login"));

        // Edit on the Claude replica: write-back to a Codex original is unsupported.
        replace_in(&replica.path, "some notes", "edited notes");
        let before = f.snapshot();
        let rows = f.ledger.list_items().unwrap();
        let r = f.try_sync(false).unwrap();
        assert!(r.needs_attention());
        assert!(r.to_string().contains("skipped: write_original"));
        assert_eq!(before, f.snapshot());
        assert_eq!(rows, f.ledger.list_items().unwrap());

        // Deleting the Claude replica must not remove the Codex original either.
        fs::remove_file(&replica.path).unwrap();
        let before = f.snapshot();
        let r = f.try_sync(false).unwrap();
        assert!(r.to_string().contains("skipped: remove original"));
        assert_eq!(before, f.snapshot());
        assert_eq!(rows, f.ledger.list_items().unwrap());
        assert!(fs::read_to_string(f.codex_root.join("memories/MEMORY.md"))
            .unwrap()
            .contains("some notes"));
    }

    #[test]
    fn mass_delete_guard_triggers_and_can_be_lifted() {
        let f = fx();
        f.fillers();
        f.sync();
        for i in 0..4 {
            fs::remove_file(f.claude_mem().join(format!("f{i}.md"))).unwrap();
        }
        let before = f.snapshot();
        let err = f.try_sync(false).unwrap_err();
        assert!(matches!(
            err,
            Error::MassDelete {
                planned: 4,
                total: 6
            }
        ));
        assert_eq!(before.len(), f.snapshot().len());
        assert_eq!(f.ledger.list_items().unwrap().len(), 6);
        f.try_sync(true).unwrap();
        assert!(f.plan().is_empty());
        assert_eq!(f.ledger.list_items().unwrap().len(), 2);
    }

    #[test]
    fn mass_delete_limit_rule() {
        assert!(!exceeds_mass_delete_limit(1, 1));
        assert!(!exceeds_mass_delete_limit(3, 3));
        assert!(exceeds_mass_delete_limit(4, 10));
        assert!(!exceeds_mass_delete_limit(4, 30));
        assert!(!exceeds_mass_delete_limit(10, 100));
        assert!(exceeds_mass_delete_limit(11, 1000));
    }

    #[test]
    fn lost_ledger_is_rebuilt_by_adoption() {
        let mut f = fx();
        f.fillers();
        f.add("a.md", "alpha", "user", "alpha body");
        f.sync();
        let before = f.snapshot();
        f.ledger = Ledger::in_memory().unwrap();
        let p = f.plan();
        assert!(p.actions.iter().all(|a| matches!(a, Action::Adopt { .. })));
        assert_eq!(p.actions.len(), 7);
        f.sync();
        assert_eq!(before, f.snapshot());
        assert_eq!(f.ledger.list_items().unwrap().len(), 7);
    }

    #[test]
    fn orphan_replica_is_reported_not_deleted() {
        let mut f = fx();
        let orig = f.add("a.md", "alpha", "user", "alpha body");
        f.sync();
        fs::remove_file(orig).unwrap();
        f.ledger = Ledger::in_memory().unwrap();
        let p = f.plan();
        assert!(matches!(p.notes[..], [Note::Skipped { .. }]));
        f.try_sync(false).unwrap();
        assert!(f.codex_replica("alpha").is_some());
    }

    #[test]
    fn dry_run_plan_changes_nothing() {
        let f = fx();
        f.add("a.md", "alpha", "user", "alpha body");
        f.xh.write_memory_md(TASK_GROUP);
        let before = f.snapshot();
        let p = f.plan();
        assert_eq!(p.actions.len(), 2);
        assert_eq!(before, f.snapshot());
        assert_eq!(f.ledger.list_items().unwrap().len(), 0);
        assert!(!f.state.exists());
    }

    #[test]
    fn failed_scan_plans_nothing() {
        let f = fx();
        f.add("a.md", "alpha", "user", "alpha body");
        f.sync();
        fs::write(f.codex_root.join("AGENTS.md"), "<!-- nativemem:begin -->\n").unwrap();
        fs::remove_dir_all(f.codex_root.join("memories")).unwrap();
        fs::write(f.codex_root.join("memories"), "not a dir").unwrap();
        assert!(plan(&f.adapters(), &f.ledger).is_err());
    }

    const RULES: &str = "# Rules\n@RTK.md\n\n## Alpha\nalpha body\n\n## Beta\nbeta line\n\n### Sub\nsub text\n\n## Gamma\n```sh\n## not a heading\n```\ngamma end\n";

    impl Fx {
        fn claude_md(&self) -> String {
            fs::read_to_string(self.claude_root.join("CLAUDE.md")).unwrap_or_default()
        }

        fn write_claude_md(&self, text: &str) {
            fs::write(self.claude_root.join("CLAUDE.md"), text).unwrap();
        }

        fn write_agents_md(&self, text: &str) {
            fs::write(self.codex_root.join("AGENTS.md"), text).unwrap();
        }

        fn instruction_names(&self, agent: &dyn Adapter) -> Vec<String> {
            agent
                .scan()
                .unwrap()
                .into_iter()
                .filter(|o| o.item.kind == Kind::Instruction)
                .map(|o| {
                    format!(
                        "{}{}",
                        o.item.name,
                        if o.replica_of.is_some() { "*" } else { "" }
                    )
                })
                .collect()
        }
    }

    #[test]
    fn claude_sections_reach_codex_and_second_plan_is_empty() {
        let f = fx();
        f.write_claude_md(RULES);
        let r = f.sync();
        assert_eq!(r.done.len(), 3);
        assert_eq!(f.claude_md(), RULES);
        let agents = f.agents_md();
        assert!(agents.starts_with("<!-- nativemem:instructions:begin -->\n"));
        assert_eq!(agents.matches("<!-- nativemem origin=claude").count(), 3);
        assert!(agents.contains("## Gamma\n\n```sh\n## not a heading\n```\ngamma end\n"));
        assert!(agents.contains("### Sub\nsub text"));
        assert_eq!(f.instruction_names(&f.codex), ["alpha*", "beta*", "gamma*"]);
        assert_eq!(f.ledger.list_items().unwrap().len(), 3);
    }

    #[test]
    fn preamble_only_and_missing_files_sync_nothing() {
        let f = fx();
        assert!(f.plan().is_empty());
        f.write_claude_md("# Title\n@RTK.md\nintro\n");
        f.write_agents_md("# Codex notes\n");
        assert!(f.plan().is_empty());
        assert!(!f
            .claude_root
            .join("CLAUDE.md")
            .with_extension("bak")
            .exists());
    }

    #[test]
    fn codex_section_reaches_claude_and_routing_block_is_no_section() {
        let f = fx();
        let agents = "# Mine\n\n## Style\nbe terse\n\n<!-- nativemem:begin -->\n## Project memory (managed by nativemem)\n- /x → /y\n<!-- nativemem:end -->\n";
        f.write_agents_md(agents);
        f.write_claude_md("# Claude\n");
        f.sync();
        assert_eq!(f.instruction_names(&f.codex), ["style"]);
        assert_eq!(f.instruction_names(&f.claude), ["style*"]);
        assert_eq!(f.agents_md(), agents);
        let claude = f.claude_md();
        assert!(claude.starts_with("# Claude\n\n<!-- nativemem:instructions:begin -->\n"));
        assert!(claude.contains("origin=codex id=AGENTS.md%23style -->\n## Style\n\nbe terse\n"));
        assert!(!claude.contains("Project memory"));
    }

    #[test]
    fn routing_block_and_instruction_region_coexist_in_order() {
        let f = fx();
        let user = "# Mine\n\n## Style\nbe terse\n";
        f.write_agents_md(user);
        f.write_claude_md(RULES);
        f.add("a.md", "alpha", "user", "alpha body");
        f.sync();
        let agents = f.agents_md();
        assert!(agents.starts_with(user));
        let block = agents.find("<!-- nativemem:begin -->").unwrap();
        let region = agents
            .find("<!-- nativemem:instructions:begin -->")
            .unwrap();
        assert!(block < region, "{agents}");
        assert_eq!(f.instruction_names(&f.codex).len(), 4);

        // Removing the memory removes the block and leaves the region intact.
        fs::remove_file(f.claude_mem().join("a.md")).unwrap();
        f.sync();
        let agents = f.agents_md();
        assert!(!agents.contains("nativemem:begin"));
        assert!(agents.starts_with(&format!("{user}\n<!-- nativemem:instructions:begin -->")));
        assert_eq!(agents.matches("<!-- nativemem origin=claude").count(), 3);
    }

    #[test]
    fn replica_edit_writes_back_to_the_original() {
        let f = fx();
        f.write_claude_md(RULES);
        f.sync();
        replace_in(
            &f.codex_root.join("AGENTS.md"),
            "beta line",
            "beta changed\nsecond",
        );
        assert!(f.plan().to_string().contains("~ claude  global  beta"));
        f.sync();
        assert_eq!(
            f.claude_md(),
            RULES.replace("beta line", "beta changed\nsecond")
        );
        assert!(f.state.join("backups/T/claude/CLAUDE.md").is_file());
    }

    #[test]
    fn instruction_original_edit_updates_replica() {
        let f = fx();
        f.write_claude_md(RULES);
        f.sync();
        f.write_claude_md(&RULES.replace("alpha body", "alpha new"));
        f.sync();
        assert!(f.agents_md().contains("alpha new"));
        assert!(!f.agents_md().contains("alpha body"));
    }

    #[test]
    fn replica_heading_edit_is_restored() {
        let f = fx();
        f.write_claude_md(RULES);
        f.sync();
        let before = f.agents_md();
        replace_in(&f.codex_root.join("AGENTS.md"), "## Beta", "## Renamed");
        f.sync();
        assert_eq!(f.agents_md(), before);
        assert_eq!(f.claude_md(), RULES);
    }

    #[test]
    fn both_sides_changed_is_a_conflict() {
        let f = fx();
        f.write_claude_md(RULES);
        f.sync();
        f.write_claude_md(&RULES.replace("alpha body", "claude side"));
        replace_in(&f.codex_root.join("AGENTS.md"), "alpha body", "codex side");
        let p = f.plan();
        assert!(matches!(p.notes[..], [Note::Conflict { .. }]), "{p}");
        assert!(p.actions.is_empty());
    }

    #[test]
    fn deleting_original_sections_removes_replicas_and_empty_region() {
        let f = fx();
        f.write_claude_md(RULES);
        f.sync();
        f.write_claude_md("# Rules\n@RTK.md\n\n## Alpha\nalpha body\n");
        f.sync();
        assert_eq!(f.instruction_names(&f.codex), ["alpha*"]);
        f.write_claude_md("# Rules\n@RTK.md\n");
        f.sync();
        assert_eq!(f.agents_md(), "");
        assert_eq!(f.claude_md(), "# Rules\n@RTK.md\n");
    }

    #[test]
    fn deleting_a_replica_section_removes_the_original_with_backup() {
        let f = fx();
        f.write_claude_md(RULES);
        f.sync();
        let agents = f.agents_md();
        let start = agents
            .find("<!-- nativemem origin=claude id=CLAUDE.md%23beta")
            .unwrap();
        let end = agents
            .find("<!-- nativemem origin=claude id=CLAUDE.md%23gamma")
            .unwrap();
        f.write_agents_md(&format!("{}{}", &agents[..start], &agents[end..]));
        f.sync();
        assert_eq!(f.instruction_names(&f.claude), ["alpha", "gamma"]);
        let claude = f.claude_md();
        assert!(!claude.contains("## Beta") && !claude.contains("beta line"));
        assert!(claude.starts_with("# Rules\n@RTK.md\n\n## Alpha\nalpha body\n\n## Gamma\n"));
        let backup = fs::read_to_string(f.state.join("backups/T/claude/CLAUDE.md")).unwrap();
        assert_eq!(backup, RULES);
        assert_eq!(f.agents_md().matches("<!-- nativemem origin=").count(), 2);
    }

    #[test]
    fn duplicate_headings_and_empty_bodies_sync() {
        let f = fx();
        let text = "## Same\none\n\n## Same\ntwo\n\n## Empty\n\n## Tail\nt\n";
        f.write_claude_md(text);
        f.sync();
        assert_eq!(
            f.instruction_names(&f.codex),
            ["empty*", "same*", "same*", "tail*"]
        );
        replace_in(
            &f.codex_root.join("AGENTS.md"),
            "## Empty\n",
            "## Empty\n\nfilled\n",
        );
        f.sync();
        assert_eq!(
            f.claude_md(),
            "## Same\none\n\n## Same\ntwo\n\n## Empty\n\nfilled\n\n## Tail\nt\n"
        );
    }

    #[test]
    fn memory_and_instruction_with_same_name_do_not_collide() {
        let f = fx();
        f.add("a.md", "alpha", "user", "memory alpha");
        f.write_claude_md("## Alpha\ninstruction alpha\n");
        f.sync();
        assert_eq!(f.ledger.list_items().unwrap().len(), 2);
        assert!(f.agents_md().contains("instruction alpha"));
    }

    struct Tri {
        _t: tempfile::TempDir,
        claude_root: PathBuf,
        codex_root: PathBuf,
        opencode_root: PathBuf,
        pi_root: PathBuf,
        ledger: Ledger,
        claude: ClaudeAdapter,
        codex: CodexAdapter,
        opencode: AgentsFileAdapter,
        pi: PiAdapter,
        app_dir: String,
    }

    fn tri() -> Tri {
        let t = tempfile::tempdir().unwrap();
        let [claude_root, codex_root, opencode_root, pi_root] =
            ["claude", "codex", "opencode", "pi"].map(|n| t.path().join(n));
        let state = t.path().join("state");
        for d in [&claude_root, &codex_root, &opencode_root, &pi_root] {
            fs::create_dir_all(d).unwrap();
        }
        let session = BackupSession::with_stamp("T");
        let app_dir = ClaudeHome::new(&claude_root).add_project(Path::new(CWD));
        Tri {
            claude: ClaudeAdapter::new(claude_root.clone(), state.clone(), session.clone())
                .with_fs_root(state.join("no-fs-root")),
            codex: CodexAdapter::new(codex_root.clone(), state.clone(), session.clone()),
            opencode: AgentsFileAdapter::new(
                "opencode",
                opencode_root.clone(),
                state.clone(),
                session.clone(),
            ),
            pi: PiAdapter::new(pi_root.clone(), state, session),
            ledger: Ledger::in_memory().unwrap(),
            claude_root,
            codex_root,
            opencode_root,
            pi_root,
            app_dir,
            _t: t,
        }
    }

    /// Instruction sections only: what an agent without a memory store declares.
    struct InstructionOnly(AgentsFileAdapter);

    impl Adapter for InstructionOnly {
        fn id(&self) -> AgentId {
            self.0.id()
        }
        fn supports(&self, kind: Kind, scope: &Scope) -> bool {
            (kind, scope) == (Kind::Instruction, &Scope::Global)
        }
        fn scan(&self) -> Result<Vec<Observed>> {
            self.0.scan()
        }
        fn write_replica(&self, key: &ItemKey, item: &Item) -> Result<(String, PathBuf)> {
            self.0.write_replica(key, item)
        }
        fn write_original(&self, native_id: &str, item: &Item) -> Result<()> {
            self.0.write_original(native_id, item)
        }
        fn remove(&self, native_id: &str) -> Result<()> {
            self.0.remove(native_id)
        }
    }

    const MEMORY_BASES: [(&str, &str); 3] = [
        ("codex", "memories/extensions/nativemem/resources"),
        ("opencode", "nativemem/memory"),
        ("pi", "nativemem/memory"),
    ];

    impl Tri {
        fn adapters(&self) -> [&dyn Adapter; 4] {
            [&self.claude, &self.codex, &self.opencode, &self.pi]
        }

        fn plan(&self) -> Plan {
            plan(&self.adapters(), &self.ledger).unwrap()
        }

        fn sync(&self) -> Report {
            let r = apply(
                self.plan(),
                &self.adapters(),
                &self.ledger,
                &ApplyOptions::default(),
            )
            .unwrap();
            let p = self.plan();
            assert!(p.is_empty(), "plan not empty after apply:\n{p}");
            r
        }

        fn root(&self, agent: &str) -> &Path {
            match agent {
                "claude" => &self.claude_root,
                "codex" => &self.codex_root,
                "opencode" => &self.opencode_root,
                _ => &self.pi_root,
            }
        }

        fn agents_md(&self, agent: &str) -> String {
            fs::read_to_string(self.root(agent).join("AGENTS.md")).unwrap_or_default()
        }

        fn add_memory(&self, body: &str) -> PathBuf {
            ClaudeHome::new(&self.claude_root).add_memory(
                &self.app_dir,
                "a.md",
                FrontmatterVariant::Nested,
                "alpha",
                "about alpha",
                "user",
                body,
            )
        }

        /// The item files of the one project store of `agent`.
        fn replica_file(&self, agent: &str) -> PathBuf {
            let base = MEMORY_BASES.iter().find(|(a, _)| *a == agent).unwrap().1;
            let projects: Vec<_> = fs::read_dir(self.root(agent).join(base))
                .unwrap()
                .map(|e| e.unwrap().path())
                .collect();
            assert_eq!(projects.len(), 1, "{projects:?}");
            projects[0].join("alpha.md")
        }

        fn files(&self, agent: &str) -> BTreeMap<String, Vec<u8>> {
            let mut out = BTreeMap::new();
            walk(self.root(agent), self.root(agent), &mut out);
            let prefix = format!("{}:", self.root(agent).display());
            out.into_iter()
                .map(|(k, v)| (k.strip_prefix(&prefix).unwrap().to_string(), v))
                .collect()
        }
    }

    #[test]
    fn memory_is_planned_only_for_agents_that_support_it() {
        let f = tri();
        let only = InstructionOnly(AgentsFileAdapter::new(
            "opencode",
            f.opencode_root.clone(),
            f._t.path().join("state"),
            BackupSession::with_stamp("T"),
        ));
        f.add_memory("alpha body");
        let adapters: [&dyn Adapter; 3] = [&f.claude, &f.codex, &only];
        let p = plan(&adapters, &f.ledger).unwrap();
        assert!(p.notes.is_empty(), "{p}");
        assert_eq!(p.actions.len(), 1, "{p}");
        assert!(
            matches!(&p.actions[0], Action::Write { target, .. } if target.as_str() == "codex")
        );
        let r = apply(p, &adapters, &f.ledger, &ApplyOptions::default()).unwrap();
        assert!(r.notes.is_empty());
        assert_eq!(r.done.len(), 1);
        assert!(!f.opencode_root.join("AGENTS.md").exists());
    }

    #[test]
    fn instruction_reaches_every_instruction_capable_agent() {
        let f = tri();
        fs::write(f.claude_root.join("CLAUDE.md"), RULES).unwrap();
        let p = f.plan();
        assert!(p.notes.is_empty(), "{p}");
        assert_eq!(p.actions.len(), 9, "{p}");
        let r = f.sync();
        assert_eq!(r.done.len(), 9);
        for agent in ["codex", "opencode", "pi"] {
            let text = f.agents_md(agent);
            assert_eq!(text.matches("<!-- nativemem origin=claude").count(), 3);
        }
        assert_eq!(f.ledger.list_items().unwrap().len(), 3);
    }

    #[test]
    fn opencode_section_reaches_the_others_and_edits_write_back() {
        let f = tri();
        fs::write(f.opencode_root.join("AGENTS.md"), "## Style\nbe terse\n").unwrap();
        f.sync();
        assert!(fs::read_to_string(f.claude_root.join("CLAUDE.md"))
            .unwrap()
            .contains("be terse"));
        let claude = f.claude_root.join("CLAUDE.md");
        replace_in(&claude, "be terse", "be terser");
        f.sync();
        assert_eq!(
            fs::read_to_string(f.opencode_root.join("AGENTS.md")).unwrap(),
            "## Style\nbe terser\n"
        );
        assert!(f.agents_md("codex").contains("be terser"));
        assert!(f.agents_md("pi").contains("be terser"));
    }

    #[test]
    fn memory_reaches_codex_pi_and_opencode_without_touching_other_bytes() {
        let f = tri();
        let user = "# Mine\n\nplain intro\n";
        for agent in ["codex", "opencode", "pi"] {
            fs::write(f.root(agent).join("AGENTS.md"), user).unwrap();
        }
        let claude_before = f.files("claude");
        f.add_memory("alpha body");
        let claude_with_memory = f.files("claude");
        let r = f.sync();
        assert!(r.notes.is_empty());
        assert_eq!(f.files("claude"), claude_with_memory, "claude is only read");
        assert_ne!(claude_before, claude_with_memory);
        for (agent, base) in MEMORY_BASES {
            let text = f.agents_md(agent);
            assert!(
                text.starts_with(&format!("{user}\n<!-- nativemem:begin -->\n")),
                "{agent}"
            );
            let file = f.replica_file(agent);
            assert!(file.starts_with(f.root(agent).join(base)));
            let index = file.with_file_name("MEMORY.md");
            assert!(
                text.contains(&format!("- {CWD} → {}\n", index.display())),
                "{agent}"
            );
            let item = fs::read_to_string(&file).unwrap();
            assert!(
                item.starts_with("<!-- nativemem origin=claude id="),
                "{agent}"
            );
            assert!(item.trim_end().ends_with("alpha body"), "{agent}");
            assert!(file.with_file_name("scope.json").is_file());
        }
        let before: Vec<_> = ["codex", "opencode", "pi"].map(|a| f.files(a)).into();
        f.sync();
        let after: Vec<_> = ["codex", "opencode", "pi"].map(|a| f.files(a)).into();
        assert_eq!(before, after, "second apply changes nothing");
    }

    #[test]
    fn memory_writes_stay_inside_the_declared_paths() {
        let f = tri();
        f.add_memory("alpha body");
        let before: Vec<_> = ["claude", "codex", "opencode", "pi"]
            .map(|a| f.files(a))
            .into();
        f.sync();
        assert_eq!(f.files("claude"), before[0]);
        for (agent, base) in MEMORY_BASES {
            for path in f.files(agent).keys() {
                assert!(
                    path == "AGENTS.md" || path.starts_with(&format!("{base}/")),
                    "{agent} wrote {path}"
                );
            }
        }
        assert!(!f._t.path().join("state/claude").exists());
    }

    #[test]
    fn memory_update_and_delete_propagate_to_all_three_stores() {
        let f = tri();
        let user = "# Mine\n";
        for agent in ["codex", "opencode", "pi"] {
            fs::write(f.root(agent).join("AGENTS.md"), user).unwrap();
        }
        let original = f.add_memory("alpha body");
        f.sync();
        replace_in(&original, "alpha body", "alpha edited");
        f.sync();
        for (agent, _) in MEMORY_BASES {
            let item = fs::read_to_string(f.replica_file(agent)).unwrap();
            assert!(item.trim_end().ends_with("alpha edited"), "{agent}");
        }
        fs::remove_file(&original).unwrap();
        let r = f.sync();
        assert!(r.notes.is_empty());
        for (agent, base) in MEMORY_BASES {
            assert_eq!(f.agents_md(agent), user, "{agent}");
            let left: Vec<_> = f
                .files(agent)
                .into_keys()
                .filter(|p| p != "AGENTS.md")
                .collect();
            assert!(
                left.iter().all(|p| !p.starts_with(&format!("{base}/"))),
                "{agent}: {left:?}"
            );
        }
        assert!(!f.pi_root.join("nativemem").exists());
        assert!(!f.opencode_root.join("nativemem").exists());
    }

    #[test]
    fn editing_a_pi_or_opencode_replica_writes_back_to_claude() {
        for edited in ["pi", "opencode"] {
            let f = tri();
            let original = f.add_memory("alpha body");
            f.sync();
            replace_in(&f.replica_file(edited), "alpha body", "alpha from replica");
            f.sync();
            let text = fs::read_to_string(&original).unwrap();
            assert!(text.contains("alpha from replica"), "{edited}: {text}");
            for (agent, _) in MEMORY_BASES {
                let item = fs::read_to_string(f.replica_file(agent)).unwrap();
                assert!(
                    item.trim_end().ends_with("alpha from replica"),
                    "{edited} -> {agent}"
                );
            }
        }
    }

    #[test]
    fn routing_block_and_instruction_region_coexist_in_pi_and_opencode() {
        let f = tri();
        fs::write(f.claude_root.join("CLAUDE.md"), "## Style\nbe terse\n").unwrap();
        let original = f.add_memory("alpha body");
        f.sync();
        for agent in ["opencode", "pi"] {
            let text = f.agents_md(agent);
            let block = text.find("<!-- nativemem:begin -->").unwrap();
            let region = text.find("<!-- nativemem:instructions:begin -->").unwrap();
            assert!(block < region, "{agent}\n{text}");
            assert_eq!(text.matches("<!-- nativemem:begin -->").count(), 1);
            assert!(
                text.ends_with("<!-- nativemem:instructions:end -->\n"),
                "{agent}"
            );
        }
        let with_both = f.agents_md("pi");
        fs::remove_file(&original).unwrap();
        f.sync();
        for agent in ["opencode", "pi"] {
            let text = f.agents_md(agent);
            assert!(!text.contains("nativemem:begin"), "{agent}");
            assert!(
                text.trim_start()
                    .starts_with("<!-- nativemem:instructions:begin -->"),
                "{agent}\n{text:?}"
            );
            assert!(with_both.ends_with(text.trim_start()));
        }
    }
}

#[cfg(test)]
mod session_tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::adapters::claude::ClaudeAdapter;
    use crate::adapters::codex::CodexAdapter;
    use crate::adapters::pi::PiAdapter;
    use crate::fsx::BackupSession;
    use crate::session::{Session, SessionOptions};
    use crate::testkit::{age_days, ClaudeHome, CodexHome, PiHome};

    const CWD: &str = "/w/app";
    const TID: &str = "0198a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b";
    const TID2: &str = "0198a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5c";

    struct Fx {
        _t: tempfile::TempDir,
        root: PathBuf,
        ledger: Ledger,
        claude: ClaudeAdapter,
        codex: CodexAdapter,
        pi: PiAdapter,
    }

    fn fx_with(options: Option<SessionOptions>) -> Fx {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().to_path_buf();
        for d in ["claude", "codex", "pi"] {
            fs::create_dir_all(root.join(d)).unwrap();
        }
        let state = root.join("state");
        let backups = BackupSession::with_stamp("T");
        Fx {
            claude: {
                let a = ClaudeAdapter::new(root.join("claude"), state.clone(), backups.clone())
                    .with_fs_root(state.join("no-fs-root"));
                match options {
                    Some(o) => a.with_sessions(o),
                    None => a,
                }
            },
            codex: {
                let a = CodexAdapter::new(root.join("codex"), state.clone(), backups.clone())
                    .with_claude_home(root.join("claude"));
                match options {
                    Some(o) => a.with_sessions(o),
                    None => a,
                }
            },
            pi: {
                let a = PiAdapter::new(root.join("pi"), state, backups);
                match options {
                    Some(o) => a.with_sessions(o),
                    None => a,
                }
            },
            ledger: Ledger::in_memory().unwrap(),
            root,
            _t: t,
        }
    }

    fn fx() -> Fx {
        fx_with(Some(SessionOptions::default()))
    }

    fn windowed(days: u32) -> Fx {
        fx_with(Some(SessionOptions {
            days: Some(days),
            ..SessionOptions::default()
        }))
    }

    const CHAT: &[(&str, &str)] = &[
        ("user", "how do I fix the login?"),
        ("assistant", "check the token expiry"),
        ("user", "thanks"),
    ];

    impl Fx {
        fn adapters(&self) -> [&dyn Adapter; 3] {
            [&self.claude, &self.codex, &self.pi]
        }

        fn plan(&self) -> Plan {
            plan(&self.adapters(), &self.ledger).unwrap()
        }

        fn sync(&self) -> Report {
            let r = apply(
                self.plan(),
                &self.adapters(),
                &self.ledger,
                &ApplyOptions::default(),
            )
            .unwrap();
            let p = self.plan();
            assert!(p.is_empty(), "plan not empty after apply:\n{p}");
            r
        }

        fn claude_session(&self, id: &str, title: &str, turns: &[(&str, &str)]) -> PathBuf {
            ClaudeHome::new(&self.root.join("claude")).add_session(
                Path::new(CWD),
                id,
                Some(title),
                turns,
            )
        }

        fn pi_session(&self, id: &str, title: &str, turns: &[(&str, &str)]) -> PathBuf {
            PiHome::new(&self.root.join("pi")).add_session(Path::new(CWD), id, Some(title), turns)
        }

        fn sessions(&self, adapter: &dyn Adapter) -> Vec<Observed> {
            adapter
                .scan()
                .unwrap()
                .into_iter()
                .filter(|o| o.item.kind == Kind::Session)
                .collect()
        }

        fn replica_of(&self, adapter: &dyn Adapter) -> Observed {
            let mut v: Vec<_> = self
                .sessions(adapter)
                .into_iter()
                .filter(|o| o.replica_of.is_some())
                .collect();
            assert_eq!(v.len(), 1, "expected one replica");
            v.remove(0)
        }

        fn files(&self) -> usize {
            fn count(dir: &Path) -> usize {
                fs::read_dir(dir).unwrap().flatten().fold(0, |n, e| {
                    let p = e.path();
                    n + if p.is_dir() { count(&p) } else { 1 }
                })
            }
            count(&self.root)
        }
    }

    fn parsed(o: &Observed) -> Session {
        Session::from_body(&o.item.body).unwrap()
    }

    #[test]
    fn claude_session_reaches_pi_and_round_trips() {
        let f = fx();
        f.claude_session("c1", "Fix login", CHAT);
        let p = f.plan();
        let text = p.to_string();
        assert!(text.contains("+ pi  /w/app  session \"Fix login\"  (from claude)"));
        assert!(text.contains("+ codex  /w/app  session \"Fix login\"  (from claude)"));
        assert_eq!(p.actions.len(), 2, "{p}");
        f.sync();

        let replica = f.replica_of(&f.pi);
        let s = parsed(&replica);
        assert_eq!(s.title, "Fix login");
        assert_eq!(s.cwd, PathBuf::from(CWD));
        assert_eq!(s.turns.len(), 3);
        let roles: Vec<_> = s.turns.iter().map(|t| t.role.as_str()).collect();
        assert_eq!(roles, ["user", "assistant", "user"]);
        assert_eq!(s.turns[1].text, "check the token expiry");
        assert_eq!(
            replica.replica_of.as_ref().unwrap().native_id,
            "-w-app/c1.jsonl"
        );
        assert!(replica.native_id.starts_with("--w-app--/"));
        let original = f
            .sessions(&f.claude)
            .into_iter()
            .find(|o| o.replica_of.is_none())
            .unwrap();
        assert_eq!(content_hash(&original.item), content_hash(&replica.item));
        let codex = f.replica_of(&f.codex);
        assert_eq!(content_hash(&original.item), content_hash(&codex.item));
        assert_eq!(
            codex.replica_of.as_ref().unwrap().native_id,
            "-w-app/c1.jsonl"
        );
    }

    #[test]
    fn pi_session_reaches_claude_and_reparses() {
        let f = fx();
        f.pi_session("p1", "Pi chat", CHAT);
        let r = f.sync();
        assert_eq!(r.done.len(), 2, "claude and codex");
        let replica = f.replica_of(&f.claude);
        let s = parsed(&replica);
        assert_eq!((s.title.as_str(), s.turns.len()), ("Pi chat", 3));
        assert_eq!(s.cwd, PathBuf::from(CWD));
        assert_eq!(s.turns[0].text, "how do I fix the login?");
        assert!(replica.native_id.starts_with("-w-app/"));
        assert!(replica.path.is_file());
        assert_eq!(s.id, replica.path.file_stem().unwrap().to_string_lossy());
        assert_eq!(
            replica.replica_of.as_ref().unwrap().native_id,
            "--w-app--/2026-11-10T10-00-00-000Z_p1.jsonl"
        );
    }

    #[test]
    fn replica_of_a_new_cwd_creates_the_project_dir() {
        let f = fx();
        PiHome::new(&f.root.join("pi")).add_session(Path::new("/other/proj"), "p9", None, CHAT);
        f.sync();
        let dir = f.root.join("claude/projects/-other-proj");
        assert_eq!(fs::read_dir(dir).unwrap().count(), 1);
    }

    #[test]
    fn codex_rollout_reaches_claude_and_pi() {
        let f = fx();
        CodexHome::new(&f.root.join("codex")).add_rollout(TID, Path::new(CWD), CHAT, false);
        let p = f.plan().to_string();
        assert!(
            p.contains("+ claude  /w/app  session \"how do I fix the login?\"  (from codex)"),
            "{p}"
        );
        assert!(
            p.contains("+ pi  /w/app  session \"how do I fix the login?\"  (from codex)"),
            "{p}"
        );
        f.sync();
        assert_eq!(parsed(&f.replica_of(&f.claude)).turns.len(), 3);
        assert_eq!(
            parsed(&f.replica_of(&f.pi)).turns[1].text,
            "check the token expiry"
        );
        // Injected context never became a turn.
        assert!(!parsed(&f.replica_of(&f.pi))
            .turns
            .iter()
            .any(|t| t.text.contains("injected")));
    }

    #[test]
    fn imported_codex_sessions_are_not_originals() {
        let f = fx();
        let x = CodexHome::new(&f.root.join("codex"));
        x.add_rollout(TID, Path::new(CWD), CHAT, false);
        x.add_rollout(TID2, Path::new(CWD), CHAT, true);
        let originals = |f: &Fx| {
            f.sessions(&f.codex)
                .into_iter()
                .filter(|o| o.replica_of.is_none())
                .count()
        };
        assert_eq!(originals(&f), 1, "marker excludes the second");
        x.write_import_ledger(&[TID]);
        assert_eq!(originals(&f), 0, "ledger excludes the first");
        assert!(f.plan().is_empty());
    }

    #[test]
    fn grown_original_refreshes_an_unmodified_replica_with_backup() {
        let f = fx();
        f.claude_session("c1", "Fix login", CHAT);
        f.sync();
        let more: Vec<(&str, &str)> = CHAT
            .iter()
            .copied()
            .chain([("assistant", "glad to help"), ("user", "one more thing")])
            .collect();
        f.claude_session("c1", "Fix login", &more);
        let p = f.plan();
        assert!(p
            .to_string()
            .contains("~ pi  /w/app  session \"Fix login\"  (from claude)"));
        f.sync();
        assert_eq!(parsed(&f.replica_of(&f.pi)).turns.len(), 5);
        assert_eq!(parsed(&f.replica_of(&f.codex)).turns.len(), 5);
        let backups = f.root.join("state/backups/T/pi/sessions");
        assert!(backups.is_dir(), "refresh backs the old replica up");
    }

    #[test]
    fn modified_replica_is_left_alone_without_a_note() {
        let f = fx();
        f.claude_session("c1", "Fix login", CHAT);
        f.sync();
        let replica = f.replica_of(&f.pi);
        let mut text = fs::read_to_string(&replica.path).unwrap();
        let last_id = text
            .lines()
            .last()
            .map(|l| {
                serde_json::from_str::<serde_json::Value>(l).unwrap()["id"]
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .unwrap();
        text.push_str(&format!("{}\n", serde_json::json!({
            "type": "message", "id": "zz", "parentId": last_id, "timestamp": "2026-11-10T11:00:00.000Z",
            "message": {"role": "user", "content": [{"type": "text", "text": "continued in pi"}], "timestamp": 1},
        })));
        fs::write(&replica.path, &text).unwrap();
        let more: Vec<(&str, &str)> = CHAT.iter().copied().chain([("user", "grown")]).collect();
        f.claude_session("c1", "Fix login", &more);
        let p = f.plan().to_string();
        assert!(!p.contains(" pi "), "{p}");
        assert_eq!(fs::read_to_string(&replica.path).unwrap(), text);
        assert_eq!(parsed(&f.replica_of(&f.pi)).turns.len(), 4);
    }

    #[test]
    fn deleted_replica_is_tombstoned_and_never_recreated() {
        let f = fx();
        f.claude_session("c1", "Fix login", CHAT);
        f.sync();
        fs::remove_file(f.replica_of(&f.pi).path).unwrap();
        let p = f.plan();
        assert!(p
            .to_string()
            .contains("x pi  /w/app  session \"Fix login\"  (replica deleted; not recreated)"));
        f.sync();
        let more: Vec<(&str, &str)> = CHAT.iter().copied().chain([("user", "grown")]).collect();
        f.claude_session("c1", "Fix login", &more);
        let p = f.plan().to_string();
        assert!(p.contains("~ codex") && !p.contains(" pi "), "{p}");
        assert_eq!(f.sessions(&f.pi).len(), 0);
    }

    #[test]
    fn vanished_original_leaves_replicas_alone() {
        let f = fx();
        let orig = f.claude_session("c1", "Fix login", CHAT);
        f.sync();
        fs::remove_file(orig).unwrap();
        let p = f.plan();
        assert!(p.is_empty(), "{p}");
        f.sync();
        assert_eq!(f.sessions(&f.pi).len(), 1);
        assert_eq!(f.ledger.list_items().unwrap().len(), 1);
    }

    #[test]
    fn lost_ledger_adopts_replicas_and_never_loops() {
        let mut f = fx();
        f.claude_session("c1", "Fix login", CHAT);
        f.sync();
        let files = f.files();
        f.ledger = Ledger::in_memory().unwrap();
        let p = f.plan();
        assert!(
            p.actions.iter().all(|a| matches!(a, Action::Adopt { .. })),
            "only adoption, no writes:\n{p}"
        );
        f.sync();
        assert_eq!(f.files(), files, "no replica-of-replica, no duplicate");
        assert_eq!(f.sessions(&f.claude).len(), 1);
        assert_eq!(f.sessions(&f.pi).len(), 1);
    }

    #[test]
    fn lost_ledger_and_pruned_original_report_nothing() {
        let mut f = fx();
        let orig = f.claude_session("c1", "Fix login", CHAT);
        f.sync();
        fs::remove_file(orig).unwrap();
        f.ledger = Ledger::in_memory().unwrap();
        assert!(f.plan().is_empty());
    }

    #[test]
    fn replicas_are_never_originals_for_a_third_agent() {
        let f = fx();
        f.claude_session("c1", "Fix login", CHAT);
        f.sync();
        // Pi holds a replica of a Claude original; nothing may flow from it to anyone.
        assert_eq!(
            f.sessions(&f.pi)
                .iter()
                .filter(|o| o.replica_of.is_none())
                .count(),
            0
        );
        assert_eq!(f.ledger.list_items().unwrap().len(), 1);
    }

    #[test]
    fn window_is_opt_in_and_limits_which_sessions_are_read() {
        let unlimited = fx();
        let ancient = unlimited.claude_session("ancient", "Ancient", CHAT);
        age_days(&ancient, 900);
        assert_eq!(unlimited.sessions(&unlimited.claude).len(), 1);
        unlimited.sync();
        assert_eq!(unlimited.sessions(&unlimited.pi).len(), 1);

        let f = windowed(14);
        let old = f.claude_session("old", "Old", CHAT);
        age_days(&old, 15);
        let recent = f.claude_session("recent", "Recent", CHAT);
        age_days(&recent, 13);
        let seen: Vec<String> = f
            .sessions(&f.claude)
            .into_iter()
            .map(|o| o.native_id)
            .collect();
        assert_eq!(seen, ["-w-app/recent.jsonl"]);
        f.sync();
        assert_eq!(f.sessions(&f.pi).len(), 1);

        let off = fx_with(None);
        off.claude_session("c", "x", CHAT);
        assert_eq!(off.sessions(&off.claude).len(), 0);
        assert!(off.plan().is_empty());
    }

    #[test]
    fn replica_keeps_the_originals_mtime_so_both_age_out_together() {
        let f = fx();
        let orig = f.claude_session("c1", "Fix login", CHAT);
        age_days(&orig, 10);
        f.sync();
        let m = |p: &Path| fs::metadata(p).unwrap().modified().unwrap();
        let replica = f.replica_of(&f.pi);
        let diff = m(&orig)
            .duration_since(m(&replica.path))
            .unwrap_or_else(|e| e.duration());
        assert!(diff.as_secs() < 2, "{diff:?}");
    }

    #[test]
    fn sessions_without_user_turns_or_cwd_are_not_synced() {
        let f = fx();
        f.claude_session("assistant-only", "x", &[("assistant", "hello")]);
        assert!(f.plan().is_empty());
        let dir = f.root.join("claude/projects/-no-cwd");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("n.jsonl"),
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"q\"}}\n",
        )
        .unwrap();
        assert!(f.plan().is_empty());
        let skipped = f.claude.session_report().unwrap().skipped;
        assert_eq!(
            skipped,
            [(
                "-no-cwd/n.jsonl".to_string(),
                "no cwd in any line".to_string()
            )]
        );
    }

    #[test]
    fn oversized_sources_are_skipped_and_reported() {
        assert_eq!(crate::session::MAX_SOURCE_BYTES, 200 * 1024 * 1024);
        let f = fx();
        let dir = f.root.join("claude/projects/-big");
        fs::create_dir_all(&dir).unwrap();
        let file = fs::File::create(dir.join("b.jsonl")).unwrap();
        file.set_len(crate::session::MAX_SOURCE_BYTES + 1).unwrap();
        assert!(f.plan().is_empty());
        let skipped = f.claude.session_report().unwrap().skipped;
        assert_eq!(skipped.len(), 1);
        assert!(
            skipped[0].1.starts_with("larger than 200 MB"),
            "{skipped:?}"
        );
    }

    fn mark_entrypoint(path: &Path, entrypoint: &str) {
        let text: String = fs::read_to_string(path)
            .unwrap()
            .lines()
            .map(|l| {
                let mut v: serde_json::Value = serde_json::from_str(l).unwrap();
                if v["type"] == "user" || v["type"] == "assistant" {
                    v["entrypoint"] = serde_json::json!(entrypoint);
                }
                format!("{v}\n")
            })
            .collect();
        fs::write(path, text).unwrap();
    }

    #[test]
    fn automated_claude_and_codex_sessions_are_counted_and_optionally_excluded() {
        for exclude in [false, true] {
            let f = fx_with(Some(SessionOptions {
                exclude_automated: exclude,
                ..SessionOptions::default()
            }));
            let sdk = f.claude_session("sdk", "Headless", CHAT);
            mark_entrypoint(&sdk, "sdk-cli");
            let cli = f.claude_session("cli", "Interactive", CHAT);
            mark_entrypoint(&cli, "cli");
            f.claude_session("none", "Unmarked", CHAT);
            let x = CodexHome::new(&f.root.join("codex"));
            x.add_rollout(TID, Path::new(CWD), CHAT, false);
            let exec = x.add_rollout(TID2, Path::new(CWD), CHAT, false);
            let text = fs::read_to_string(&exec).unwrap().replacen(
                "\"source\":\"cli\"",
                "\"source\":\"exec\"",
                1,
            );
            fs::write(&exec, text).unwrap();
            let claude_ids: Vec<String> = f
                .sessions(&f.claude)
                .into_iter()
                .map(|o| o.native_id)
                .collect();
            assert_eq!(
                claude_ids.contains(&"-w-app/sdk.jsonl".to_string()),
                !exclude
            );
            assert!(claude_ids.contains(&"-w-app/cli.jsonl".to_string()));
            assert!(
                claude_ids.contains(&"-w-app/none.jsonl".to_string()),
                "unmarked fails open"
            );
            assert_eq!(f.sessions(&f.codex).len(), if exclude { 1 } else { 2 });
            assert_eq!(f.claude.session_report().unwrap().automated, 1);
            assert_eq!(f.codex.session_report().unwrap().automated, 1);
        }
    }

    #[test]
    fn codex_tool_activity_reads_the_same_in_claude_and_pi() {
        let f = fx();
        let path =
            CodexHome::new(&f.root.join("codex")).add_rollout(TID, Path::new(CWD), CHAT, false);
        let mut text = fs::read_to_string(&path).unwrap();
        for l in [
            serde_json::json!({"timestamp": "2026-11-10T10:00:01.500Z", "type": "response_item", "payload": {
                "type": "custom_tool_call", "name": "exec", "call_id": "c1",
                "input": "text(await tools.exec_command({cmd:\"cat notes.txt\",max_output_tokens:5000}));"}}),
            serde_json::json!({"timestamp": "2026-11-10T10:00:01.600Z", "type": "response_item", "payload": {
                "type": "custom_tool_call_output", "call_id": "c1",
                "output": "{\"chunk_id\":\"1\",\"wall_time_seconds\":0.2,\"exit_code\":1,\"original_token_count\":3,\"output\":\"no such file\"}"}}),
        ] {
            text.push_str(&format!("{l}\n"));
        }
        fs::write(&path, text).unwrap();
        f.sync();
        let want =
            "Tool: exec\n```\ncat notes.txt\n```\nResult (exit code 1):\n```\nno such file\n```";
        let original = f.sessions(&f.codex).remove(0);
        for adapter in [&f.claude as &dyn Adapter, &f.pi] {
            let replica = f.replica_of(adapter);
            let s = parsed(&replica);
            assert_eq!(s.turns.iter().filter(|t| t.text == want).count(), 1);
            assert!(s
                .turns
                .iter()
                .all(|t| !t.text.is_empty() && !t.text.contains("external_agent")));
            assert_eq!(
                content_hash(&replica.item),
                content_hash(&original.item),
                "stable across a re-read"
            );
        }
    }

    fn codex_files(f: &Fx) -> std::collections::BTreeSet<PathBuf> {
        fn walk(dir: &Path, out: &mut std::collections::BTreeSet<PathBuf>) {
            for e in fs::read_dir(dir).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, out);
                } else {
                    out.insert(p);
                }
            }
        }
        let mut out = Default::default();
        walk(&f.root.join("codex"), &mut out);
        out
    }

    fn import_records(f: &Fx) -> Vec<serde_json::Value> {
        let text =
            fs::read_to_string(f.root.join("codex/external_agent_session_imports.json")).unwrap();
        serde_json::from_str::<serde_json::Value>(&text).unwrap()["records"]
            .as_array()
            .unwrap()
            .clone()
    }

    #[test]
    fn claude_session_becomes_a_codex_rollout_with_an_import_record() {
        let f = fx();
        let src = f.claude_session("c1", "Fix login", CHAT);
        age_days(&src, 3);
        f.sync();
        let replica = f.replica_of(&f.codex);
        assert!(replica
            .native_id
            .starts_with("2026/11/10/rollout-2026-11-10T10-00-00-"));
        let s = parsed(&replica);
        assert_eq!((s.title.as_str(), s.turns.len()), ("Fix login", 3));
        assert_eq!(
            content_hash(&replica.item),
            content_hash(&f.sessions(&f.claude)[0].item)
        );
        let text = fs::read_to_string(&replica.path).unwrap();
        assert_eq!(text.matches("<EXTERNAL SESSION IMPORTED>").count(), 1);
        assert!(
            text.contains("external-import-turn-2") && !text.contains("external-import-turn-3")
        );
        let tid = replica
            .path
            .file_stem()
            .unwrap()
            .to_string_lossy()
            .rsplit('-')
            .take(5)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("-");
        assert_eq!(tid.as_bytes()[14], b'7', "{tid}");

        let records = import_records(&f);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0]["imported_thread_id"], tid);
        assert_eq!(records[0]["source_path"], src.to_string_lossy().as_ref());
        assert_eq!(
            records[0]["content_sha256"],
            crate::session::sha_hex(&fs::read(&src).unwrap())
        );
        f.sync();
        assert_eq!(import_records(&f).len(), 1, "idempotent");

        let m = |p: &Path| fs::metadata(p).unwrap().modified().unwrap();
        let diff = m(&src)
            .duration_since(m(&replica.path))
            .unwrap_or_else(|e| e.duration());
        assert!(diff.as_secs() < 2);

        // Nothing but sessions/ and the import ledger was written.
        let changed: Vec<String> = codex_files(&f)
            .iter()
            .map(|p| {
                p.strip_prefix(f.root.join("codex"))
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert!(
            changed
                .iter()
                .all(|p| p.starts_with("sessions/") || p == "external_agent_session_imports.json"),
            "{changed:?}"
        );
    }

    #[test]
    fn pi_session_becomes_a_codex_rollout_without_an_import_record() {
        let f = fx();
        f.pi_session("p1", "Pi chat", CHAT);
        f.sync();
        let replica = f.replica_of(&f.codex);
        assert_eq!(parsed(&replica).turns.len(), 3);
        let first = fs::read_to_string(&replica.path).unwrap();
        let meta: serde_json::Value = serde_json::from_str(first.lines().next().unwrap()).unwrap();
        assert_eq!(meta["payload"]["originator"], "Pi");
        assert_eq!(meta["payload"]["source"], "vscode");
        assert!(!f
            .root
            .join("codex/external_agent_session_imports.json")
            .exists());
    }

    #[test]
    fn a_session_imported_by_codex_is_adopted_not_written_again() {
        let f = fx();
        let src = f.claude_session("c1", "Fix login", CHAT);
        let x = CodexHome::new(&f.root.join("codex"));
        x.add_rollout(TID, Path::new(CWD), CHAT, true);
        fs::write(
            f.root.join("codex/external_agent_session_imports.json"),
            serde_json::json!({"records": [{"source_path": src.to_string_lossy(), "content_sha256": "x",
                "imported_thread_id": TID, "imported_at": 1, "source_modified_at": 2, "connector_names": [], "title": "t"}]})
            .to_string(),
        )
        .unwrap();
        let before = codex_files(&f);
        let p = f.plan();
        let text = p.to_string();
        assert!(
            text.contains("= codex  /w/app  session \"Fix login\"   (adopt replica of claude)"),
            "{text}"
        );
        assert!(!text.contains("+ codex"), "{text}");
        f.sync();
        assert_eq!(codex_files(&f), before, "no new rollout, ledger untouched");
        assert_eq!(import_records(&f).len(), 1);
        let more: Vec<(&str, &str)> = CHAT.iter().copied().chain([("user", "grown")]).collect();
        f.claude_session("c1", "Fix login", &more);
        assert!(
            !f.plan().to_string().contains("codex"),
            "an adopted import is never rewritten"
        );
    }

    #[test]
    fn tool_activity_renders_identically_in_every_target() {
        let f = fx();
        let src = f.claude_session("c1", "Fix login", &[("user", "list files")]);
        let mut text = fs::read_to_string(&src).unwrap();
        for l in [
            serde_json::json!({"type": "assistant", "timestamp": "2026-11-10T10:00:01.000Z", "cwd": CWD, "message": {"role": "assistant",
                "content": [{"type": "tool_use", "id": "t1", "name": "Bash", "input": {"command": "ls", "description": "d"}}]}}),
            serde_json::json!({"type": "user", "timestamp": "2026-11-10T10:00:02.000Z", "cwd": CWD, "message": {"role": "user",
                "content": [{"type": "tool_result", "tool_use_id": "t1", "content": "a\nb"}]}}),
        ] {
            text.push_str(&format!("{l}\n"));
        }
        fs::write(&src, text).unwrap();
        f.sync();
        let want = "Tool: Bash\n```\nls\n```\nResult:\n```\na\nb\n```";
        for a in [&f.pi as &dyn Adapter, &f.codex] {
            let s = parsed(&f.replica_of(a));
            assert_eq!(s.turns.iter().filter(|t| t.text == want).count(), 1);
        }
    }

    #[test]
    fn claude_replicas_are_refreshed_on_a_revision_bump_too() {
        let f = fx();
        PiHome::new(&f.root.join("pi")).add_session(Path::new(CWD), "p1", Some("Pi chat"), CHAT);
        f.sync();
        let key = ItemKey {
            origin: AgentId::new("pi"),
            native_id: "--w-app--/2026-11-10T10-00-00-000Z_p1.jsonl".into(),
        };
        let mut row = f
            .ledger
            .get_replica(&key, &AgentId::new("claude"))
            .unwrap()
            .unwrap();
        assert_eq!(row.revision, 1);
        row.revision = 0;
        f.ledger.upsert_replica(&row).unwrap();
        assert!(f.plan().to_string().starts_with("~ claude"));
        f.sync();
    }

    fn set_revision(f: &Fx, revision: u32) -> crate::ledger::ReplicaRow {
        let key = ItemKey {
            origin: AgentId::new("claude"),
            native_id: "-w-app/c1.jsonl".into(),
        };
        let mut row = f
            .ledger
            .get_replica(&key, &AgentId::new("pi"))
            .unwrap()
            .unwrap();
        row.revision = revision;
        f.ledger.upsert_replica(&row).unwrap();
        row
    }

    #[test]
    fn written_pi_replica_names_no_placeholder_model() {
        let f = fx();
        f.claude_session("c1", "Fix login", CHAT);
        f.sync();
        let text = fs::read_to_string(f.replica_of(&f.pi).path).unwrap();
        assert!(!text.contains("\"api\":\"nativemem\""), "{text}");
        assert!(!text.contains("model_change"));
        assert!(text.contains("\"api\":\"pi-virtual\""));
        assert_eq!(set_revision(&f, 2).revision, 2);
    }

    #[test]
    fn unmodified_replica_with_an_older_writer_revision_is_rewritten_once() {
        let f = fx();
        f.claude_session("c1", "Fix login", CHAT);
        f.sync();
        assert_eq!(set_revision(&f, 0).revision, 0);
        let path = f.replica_of(&f.pi).path;
        let old_format = fs::read_to_string(&path)
            .unwrap()
            .replace("pi-virtual", "nativemem");
        fs::write(&path, old_format).unwrap();
        let p = f.plan();
        assert_eq!(
            p.to_string().lines().next().unwrap(),
            "~ pi  /w/app  session \"Fix login\"  (from claude)"
        );
        f.sync();
        assert_eq!(
            set_revision(&f, 1).revision,
            1,
            "rewritten at the current revision"
        );
        assert!(f.root.join("state/backups/T/pi/sessions").is_dir());
        assert!(fs::read_to_string(&path).unwrap().contains("pi-virtual"));
        assert_eq!(parsed(&f.replica_of(&f.pi)).turns.len(), 3);
    }

    #[test]
    fn modified_replica_with_an_older_writer_revision_is_left_alone() {
        let f = fx();
        f.claude_session("c1", "Fix login", CHAT);
        f.sync();
        let replica = f.replica_of(&f.pi);
        let mut text = fs::read_to_string(&replica.path).unwrap();
        let last_id = text
            .lines()
            .last()
            .map(|l| {
                serde_json::from_str::<serde_json::Value>(l).unwrap()["id"]
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .unwrap();
        text.push_str(&format!(
            "{}\n",
            serde_json::json!({"type": "message", "id": "zz", "parentId": last_id,
                "timestamp": "2026-11-10T11:00:00.000Z",
                "message": {"role": "user", "content": [{"type": "text", "text": "mine"}], "timestamp": 1}})
        ));
        fs::write(&replica.path, &text).unwrap();
        set_revision(&f, 0);
        assert!(f.plan().is_empty());
        assert_eq!(fs::read_to_string(&replica.path).unwrap(), text);
    }

    #[test]
    fn memory_and_session_items_coexist_in_one_ledger() {
        let f = fx();
        f.claude_session("c1", "Fix login", CHAT);
        f.sync();
        let kinds: Vec<_> = f
            .ledger
            .list_items()
            .unwrap()
            .into_iter()
            .map(|r| r.kind)
            .collect();
        assert_eq!(kinds, [Kind::Session]);
        let r = f
            .ledger
            .list_replicas(&ItemKey {
                origin: AgentId::new("claude"),
                native_id: "-w-app/c1.jsonl".into(),
            })
            .unwrap();
        let agents: Vec<_> = r.iter().map(|x| x.agent.as_str()).collect();
        assert_eq!(agents, ["codex", "pi"]);
        assert!(r.iter().all(|x| !x.hash.is_empty()));
    }
}
