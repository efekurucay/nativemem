use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::error::{Error, Result};
use crate::ledger::{ItemRow, Ledger, ReplicaRow};
use crate::model::{content_hash, Adapter, AgentId, Item, ItemKey, Observed, Scope};

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
}

impl Action {
    fn key(&self) -> &ItemKey {
        match self {
            Action::Adopt { item, .. } => &item.key,
            Action::Write { key, .. }
            | Action::WriteBack { key, .. }
            | Action::Remove { key, .. }
            | Action::Forget { key } => key,
        }
    }

    fn rank(&self) -> u8 {
        match self {
            Action::Adopt { .. } => 0,
            Action::Write { .. } | Action::WriteBack { .. } => 1,
            Action::Remove { .. } => 2,
            Action::Forget { .. } => 3,
        }
    }

    fn label(&self) -> String {
        match self {
            Action::Adopt { name, .. } | Action::Remove { name, .. } => name.clone(),
            Action::Write { item, .. } | Action::WriteBack { item, .. } => item.name.clone(),
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
                "{} {}  {}  {}   (from {})",
                if *update { '~' } else { '+' },
                target,
                scope_label(&item.scope),
                item.name,
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
            Action::Adopt { .. } | Action::Forget { .. } => {}
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
    let ids: Vec<AgentId> = adapters.iter().map(|a| a.id()).collect();

    let mut keys: BTreeSet<ItemKey> = originals.keys().cloned().collect();
    keys.extend(replicas.keys().map(|(k, _)| k.clone()));
    keys.extend(ledger.list_items()?.into_iter().map(|r| r.key));

    let mut plan = Plan::default();
    for key in keys {
        if ids.contains(&key.origin) {
            plan_key(&ids, &originals, &replicas, ledger, &key, &mut plan)?;
        }
    }
    Ok(plan)
}

fn plan_key(
    ids: &[AgentId],
    originals: &BTreeMap<ItemKey, &Observed>,
    replicas: &Replicas<'_>,
    ledger: &Ledger,
    key: &ItemKey,
    plan: &mut Plan,
) -> Result<()> {
    let item_row = ledger.get_item(key)?;
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
            item: c
                .obs
                .map(|o| o.item.clone())
                .unwrap_or_else(|| orig.item.clone()),
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
    })
}

fn run(
    action: &Action,
    adapters: &[&dyn Adapter],
    ledger: &Ledger,
    touched: &mut BTreeSet<ItemKey>,
) -> Result<()> {
    match action {
        Action::Adopt { item, replica, .. } => {
            ledger.upsert_item(item)?;
            ledger.upsert_replica(replica)?;
            touched.insert(item.key.clone());
        }
        Action::Write {
            key, target, item, ..
        } => {
            write_replica(adapter_for(adapters, target)?, ledger, key, item)?;
            touched.insert(key.clone());
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
            touched.insert(key.clone());
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
    }
    Ok(())
}

/// Records what is actually on disk for every item the run touched, so formats that do not
/// round-trip byte-identically never look like fresh edits.
fn refresh(adapters: &[&dyn Adapter], ledger: &Ledger, touched: &BTreeSet<ItemKey>) -> Result<()> {
    if touched.is_empty() {
        return Ok(());
    }
    let observed = scan_all(adapters)?;
    let (originals, replicas) = index(&observed);
    for key in touched {
        let Some(mut row) = ledger.get_item(key)? else {
            continue;
        };
        if let Some(o) = originals.get(key) {
            row.hash = content_hash(&o.item);
            ledger.upsert_item(&row)?;
        }
        for mut r in ledger.list_replicas(key)? {
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
    touched: &mut BTreeSet<ItemKey>,
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
    let total = ledger.list_items()?.len();
    if !opts.allow_mass_delete && exceeds_mass_delete_limit(planned, total) {
        return Err(Error::MassDelete { planned, total });
    }

    let mut actions = plan.actions;
    actions.sort_by_key(Action::rank);
    let mut report = Report {
        done: Vec::new(),
        notes: plan.notes,
    };
    let mut touched = BTreeSet::new();
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
    use crate::adapters::claude::ClaudeAdapter;
    use crate::adapters::codex::CodexAdapter;
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
}
