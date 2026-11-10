//! Skills are mirrored between the four agents' own skills directories. Temp directories only.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use nativemem_core::adapters::agents_file::AgentsFileAdapter;
use nativemem_core::adapters::claude::ClaudeAdapter;
use nativemem_core::adapters::codex::CodexAdapter;
use nativemem_core::adapters::pi::PiAdapter;
use nativemem_core::engine::{self, Action, ApplyOptions, Note, Plan, Report};
use nativemem_core::error::Error;
use nativemem_core::fsx::BackupSession;
use nativemem_core::ledger::Ledger;
use nativemem_core::model::{content_hash, Adapter, Kind, Scope};

struct Fx {
    _t: tempfile::TempDir,
    root: PathBuf,
    shared: PathBuf,
    ledger: Ledger,
    blocked_codex: Vec<Kind>,
    visible: bool,
}

const AGENTS: [&str; 4] = ["claude", "codex", "opencode", "pi"];

fn fx() -> Fx {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().to_path_buf();
    for a in AGENTS {
        fs::create_dir_all(root.join(a)).unwrap();
    }
    Fx {
        shared: root.join("shared"),
        ledger: Ledger::in_memory().unwrap(),
        root,
        _t: t,
        blocked_codex: Vec::new(),
        visible: true,
    }
}

fn put(dir: &Path, rel: &str, text: &str) {
    let p = dir.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
}

#[cfg(unix)]
fn make_executable(p: &Path) {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(p, fs::Permissions::from_mode(0o755)).unwrap();
}

impl Fx {
    fn skills(&self, agent: &str) -> PathBuf {
        self.root.join(agent).join("skills")
    }

    fn skill(&self, agent: &str, name: &str, body: &str) -> PathBuf {
        self.skill_in(&self.skills(agent), name, body)
    }

    fn skill_in(&self, dir: &Path, name: &str, body: &str) -> PathBuf {
        let d = dir.join(name);
        put(
            &d,
            "SKILL.md",
            &format!("---\nname: {name}\ndescription: Does {name}\n---\n{body}\n"),
        );
        d
    }

    fn state(&self) -> PathBuf {
        self.root.join("state")
    }

    fn with_adapters<T>(&self, f: impl FnOnce(&[&dyn Adapter]) -> T) -> T {
        let stamp = || BackupSession::with_stamp("T");
        let visible = || {
            if self.visible {
                vec![self.shared.clone()]
            } else {
                Vec::new()
            }
        };
        let claude = ClaudeAdapter::new(self.root.join("claude"), self.state(), stamp());
        let codex = CodexAdapter::new(self.root.join("codex"), self.state(), stamp())
            .with_visible_roots(visible())
            .with_blocked_kinds(&self.blocked_codex);
        let opencode = AgentsFileAdapter::new(
            "opencode",
            self.root.join("opencode"),
            self.state(),
            stamp(),
        );
        let pi = PiAdapter::new(self.root.join("pi"), self.state(), stamp())
            .with_visible_roots(visible());
        f(&[&claude, &codex, &opencode, &pi])
    }

    fn plan(&self) -> Plan {
        self.with_adapters(|a| engine::plan(a, &self.ledger).unwrap())
    }

    fn try_sync(&self, allow_mass_delete: bool) -> Result<Report, Error> {
        self.with_adapters(|a| {
            let plan = engine::plan(a, &self.ledger).unwrap();
            engine::apply(plan, a, &self.ledger, &ApplyOptions { allow_mass_delete })
        })
    }

    fn sync(&self) -> Report {
        let r = self.try_sync(false).unwrap();
        let p = self.plan();
        assert!(p.is_empty(), "plan not empty after apply:\n{p}");
        r
    }

    fn hash_of(&self, agent: &str, name: &str) -> String {
        self.with_adapters(|adapters| {
            let a = adapters.iter().find(|a| a.id().as_str() == agent).unwrap();
            let o = a
                .scan()
                .unwrap()
                .into_iter()
                .find(|o| o.item.kind == Kind::Skill && o.item.name == name)
                .unwrap_or_else(|| panic!("{agent} holds no {name}"));
            content_hash(&o.item)
        })
    }

    fn holds(&self, agent: &str, name: &str) -> bool {
        self.skills(agent).join(name).join("SKILL.md").is_file()
    }

    fn all_agree(&self, name: &str) {
        let first = self.hash_of("claude", name);
        for a in &AGENTS[1..] {
            assert_eq!(self.hash_of(a, name), first, "{a} differs for {name}");
        }
    }

    fn snapshot(&self) -> BTreeMap<String, Vec<u8>> {
        let mut out = BTreeMap::new();
        for a in AGENTS {
            walk(&self.root.join(a), &self.root, &mut out);
        }
        out
    }
}

fn walk(dir: &Path, root: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        let rel = p.strip_prefix(root).unwrap().display().to_string();
        if fs::symlink_metadata(&p).unwrap().file_type().is_symlink() {
            out.insert(
                rel,
                fs::read_link(&p)
                    .unwrap()
                    .display()
                    .to_string()
                    .into_bytes(),
            );
        } else if p.is_dir() {
            walk(&p, root, out);
        } else {
            out.insert(rel, fs::read(&p).unwrap());
        }
    }
}

fn adopts(plan: &Plan) -> usize {
    plan.actions
        .iter()
        .filter(|a| matches!(a, Action::Adopt { .. }))
        .count()
}

fn writes(plan: &Plan) -> usize {
    plan.actions
        .iter()
        .filter(|a| !matches!(a, Action::Adopt { .. } | Action::Forget { .. }))
        .count()
}

#[test]
fn identical_copies_are_adopted_without_any_write() {
    let f = fx();
    for a in ["claude", "codex", "opencode"] {
        f.skill(a, "demo-skill", "same");
    }
    let before = f.snapshot();
    let plan = f.plan();
    assert_eq!(writes(&plan), 1, "{plan}");
    assert_eq!(adopts(&plan), 2, "{plan}");
    assert!(plan.notes.is_empty(), "{plan}");
    // Pi lacks the skill: it is the only agent that gets a copy.
    assert!(plan.actions.iter().any(
        |a| matches!(a, Action::Write { target, update: false, .. } if target.as_str() == "pi")
    ));
    f.sync();
    assert!(f.holds("pi", "demo-skill"));
    for a in ["claude", "codex", "opencode"] {
        assert_eq!(
            fs::read(f.skills(a).join("demo-skill/SKILL.md")).unwrap(),
            before[&format!("{a}/skills/demo-skill/SKILL.md")]
        );
    }
    let items = f.ledger.list_items().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].key.origin.as_str(), "claude");
    assert_eq!(f.ledger.list_replicas(&items[0].key).unwrap().len(), 3);
}

#[test]
fn identical_copies_everywhere_change_nothing_on_disk() {
    let f = fx();
    for a in AGENTS {
        f.skill(a, "demo-skill", "same");
    }
    let before = f.snapshot();
    let plan = f.plan();
    assert_eq!(writes(&plan), 0, "{plan}");
    assert_eq!(adopts(&plan), 3);
    f.sync();
    assert_eq!(f.snapshot(), before);
    assert!(!f.state().join("backups").exists());
}

#[test]
fn same_name_with_different_content_is_a_conflict_and_nothing_is_written() {
    let f = fx();
    f.skill("claude", "demo-skill", "one");
    f.skill("codex", "demo-skill", "two");
    let before = f.snapshot();
    let plan = f.plan();
    assert!(plan.actions.is_empty(), "{plan}");
    assert!(matches!(&plan.notes[..], [Note::Conflict { .. }]), "{plan}");
    f.try_sync(false).unwrap();
    assert_eq!(f.snapshot(), before);
    assert!(f.ledger.list_items().unwrap().is_empty());
}

#[test]
fn a_new_skill_reaches_the_other_agents_with_its_scripts_executable() {
    let f = fx();
    let d = f.skill("claude", "demo-skill", "body");
    put(&d, "scripts/run.sh", "#!/bin/sh\necho hi\n");
    put(&d, "references/notes.md", "notes");
    #[cfg(unix)]
    make_executable(&d.join("scripts/run.sh"));
    let r = f.sync();
    assert_eq!(r.done.len(), 3, "{r}");
    f.all_agree("demo-skill");
    for a in &AGENTS[1..] {
        let copy = f.skills(a).join("demo-skill");
        assert_eq!(
            fs::read_to_string(copy.join("references/notes.md")).unwrap(),
            "notes"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(copy.join("scripts/run.sh"))
                .unwrap()
                .permissions()
                .mode();
            assert_ne!(mode & 0o111, 0, "{a}");
        }
    }
}

#[test]
fn the_skills_directory_is_created_on_the_first_write_when_the_home_exists() {
    let f = fx();
    f.skill("claude", "demo-skill", "body");
    assert!(!f.skills("pi").exists());
    f.sync();
    assert!(f.holds("pi", "demo-skill"));
}

#[test]
fn an_agent_without_a_home_takes_no_part() {
    let f = fx();
    fs::remove_dir_all(f.root.join("opencode")).unwrap();
    f.skill("claude", "demo-skill", "body");
    let plan = f.with_adapters(|a| {
        let present: Vec<&dyn Adapter> = a
            .iter()
            .copied()
            .filter(|x| x.id().as_str() != "opencode")
            .collect();
        engine::plan(&present, &f.ledger).unwrap()
    });
    assert_eq!(plan.actions.len(), 2, "{plan}");
}

#[test]
fn an_edit_in_the_origin_updates_every_replica_with_a_backup() {
    let f = fx();
    f.skill("claude", "demo-skill", "v1");
    f.sync();
    let d = f.skills("claude").join("demo-skill");
    put(
        &d,
        "SKILL.md",
        "---\nname: demo-skill\ndescription: edited\n---\nv2\n",
    );
    put(&d, "references/new.md", "new");
    let plan = f.plan();
    assert_eq!(writes(&plan), 3, "{plan}");
    f.sync();
    f.all_agree("demo-skill");
    assert!(f
        .skills("pi")
        .join("demo-skill/references/new.md")
        .is_file());
    assert!(f
        .state()
        .join("backups/T/codex/skills/demo-skill/SKILL.md")
        .is_file());
}

#[test]
fn an_edit_in_a_replica_is_written_back_to_the_origin_and_the_others() {
    for editor in ["pi", "codex"] {
        let f = fx();
        f.skill("claude", "demo-skill", "v1");
        f.sync();
        let d = f.skills(editor).join("demo-skill");
        put(
            &d,
            "SKILL.md",
            "---\nname: demo-skill\ndescription: from replica\n---\nv2\n",
        );
        put(&d, "scripts/new.sh", "echo");
        f.sync();
        f.all_agree("demo-skill");
        let origin = fs::read_to_string(f.skills("claude").join("demo-skill/SKILL.md")).unwrap();
        assert!(origin.contains("from replica"), "{editor}");
        assert!(f
            .skills("opencode")
            .join("demo-skill/scripts/new.sh")
            .is_file());
        assert!(f
            .state()
            .join("backups/T/claude/skills/demo-skill/SKILL.md")
            .is_file());
    }
}

#[test]
fn edits_on_both_sides_are_a_conflict() {
    let f = fx();
    f.skill("claude", "demo-skill", "v1");
    f.sync();
    put(&f.skills("claude"), "demo-skill/a.md", "a");
    put(&f.skills("pi"), "demo-skill/b.md", "b");
    let before = f.snapshot();
    let plan = f.plan();
    assert!(plan.actions.is_empty(), "{plan}");
    assert!(matches!(&plan.notes[..], [Note::Conflict { .. }]), "{plan}");
    f.try_sync(false).unwrap();
    assert_eq!(f.snapshot(), before);
}

#[test]
fn deleting_the_origin_removes_every_copy_with_backups() {
    let f = fx();
    f.skill("claude", "demo-skill", "v1");
    f.sync();
    fs::remove_dir_all(f.skills("claude").join("demo-skill")).unwrap();
    f.sync();
    for a in AGENTS {
        assert!(!f.holds(a, "demo-skill"), "{a}");
    }
    assert!(f.ledger.list_items().unwrap().is_empty());
    for a in ["codex", "opencode", "pi"] {
        assert!(f
            .state()
            .join(format!("backups/T/{a}/skills/demo-skill/SKILL.md"))
            .is_file());
    }
}

#[test]
fn deleting_a_replica_removes_the_origin_and_the_others() {
    let f = fx();
    f.skill("claude", "demo-skill", "v1");
    f.sync();
    fs::remove_dir_all(f.skills("codex").join("demo-skill")).unwrap();
    f.sync();
    for a in AGENTS {
        assert!(!f.holds(a, "demo-skill"), "{a}");
    }
    assert!(f
        .state()
        .join("backups/T/claude/skills/demo-skill/SKILL.md")
        .is_file());
}

#[test]
fn a_delete_against_an_edit_is_a_conflict() {
    let f = fx();
    f.skill("claude", "demo-skill", "v1");
    f.sync();
    fs::remove_dir_all(f.skills("codex").join("demo-skill")).unwrap();
    put(&f.skills("pi"), "demo-skill/extra.md", "x");
    let plan = f.plan();
    assert!(plan.actions.is_empty(), "{plan}");
    assert!(matches!(&plan.notes[..], [Note::Conflict { .. }]), "{plan}");
}

#[cfg(unix)]
#[test]
fn a_symlinked_skill_counts_as_present_and_is_replaced_never_written_through() {
    let f = fx();
    let shared_copy = f.skill_in(&f.shared, "demo-skill", "same");
    f.skill("claude", "demo-skill", "same");
    fs::create_dir_all(f.skills("pi")).unwrap();
    std::os::unix::fs::symlink(&shared_copy, f.skills("pi").join("demo-skill")).unwrap();

    let plan = f.plan();
    assert_eq!(adopts(&plan), 1, "{plan}");
    let adopted: Vec<_> = plan
        .actions
        .iter()
        .filter_map(|a| match a {
            Action::Adopt { replica, .. } => Some(replica.agent.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(adopted, ["pi"]);
    f.sync();
    let link = f.skills("pi").join("demo-skill");
    assert!(fs::symlink_metadata(&link)
        .unwrap()
        .file_type()
        .is_symlink());

    put(
        &f.skills("claude"),
        "demo-skill/SKILL.md",
        "---\ndescription: new\n---\nv2\n",
    );
    f.try_sync(false).unwrap();
    assert!(!fs::symlink_metadata(&link)
        .unwrap()
        .file_type()
        .is_symlink());
    for a in ["opencode", "pi"] {
        assert_eq!(
            f.hash_of(a, "demo-skill"),
            f.hash_of("claude", "demo-skill"),
            "{a}"
        );
    }
    let shared_text = fs::read_to_string(shared_copy.join("SKILL.md")).unwrap();
    assert!(
        shared_text.contains("Does demo-skill"),
        "the shared copy is untouched"
    );
    let record = f.state().join("backups/T/pi/skills/demo-skill");
    assert!(fs::read_to_string(record)
        .unwrap()
        .starts_with("symlink ->"));
}

#[cfg(unix)]
#[test]
fn deleting_a_symlinked_replica_removes_the_link_and_not_its_target() {
    let f = fx();
    let target = f.skill_in(&f.shared, "demo-skill", "same");
    f.skill("claude", "demo-skill", "same");
    fs::create_dir_all(f.skills("pi")).unwrap();
    std::os::unix::fs::symlink(&target, f.skills("pi").join("demo-skill")).unwrap();
    f.sync();
    fs::remove_dir_all(f.skills("claude").join("demo-skill")).unwrap();
    f.sync();
    assert!(fs::symlink_metadata(f.skills("pi").join("demo-skill")).is_err());
    assert!(target.join("SKILL.md").is_file());
}

#[test]
fn a_matching_copy_in_a_visible_root_means_no_copy_is_written() {
    let f = fx();
    f.skill_in(&f.shared, "demo-skill", "same");
    f.skill("claude", "demo-skill", "same");
    let plan = f.plan();
    let targets: Vec<_> = plan
        .actions
        .iter()
        .filter_map(|a| match a {
            Action::Write { target, .. } => Some(target.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(targets, ["opencode"], "{plan}");
    assert!(plan.notes.is_empty(), "{plan}");
    f.sync();
    assert!(!f.holds("codex", "demo-skill"));
    assert!(!f.holds("pi", "demo-skill"));
}

#[test]
fn a_visible_copy_with_other_content_is_not_written_over_and_is_reported() {
    let f = fx();
    f.skill_in(&f.shared, "demo-skill", "shared text");
    f.skill("claude", "demo-skill", "claude text");
    let plan = f.plan();
    let skipped: Vec<_> = plan
        .notes
        .iter()
        .filter_map(|n| match n {
            Note::Skipped { agent, .. } => Some(agent.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(skipped, ["codex", "pi"], "{plan}");
    f.try_sync(false).unwrap();
    assert!(!f.holds("codex", "demo-skill"));
    assert!(!f.holds("pi", "demo-skill"));
    assert!(f.holds("opencode", "demo-skill"));
    assert!(fs::read_to_string(f.shared.join("demo-skill/SKILL.md"))
        .unwrap()
        .contains("shared text"));
}

#[test]
fn a_shadowing_visible_copy_shows_in_the_skill_report() {
    let f = fx();
    f.skill_in(&f.shared, "demo-skill", "shared text");
    f.skill("codex", "demo-skill", "own text");
    let shadowed = f.with_adapters(|a| {
        a.iter()
            .map(|x| (x.id().to_string(), x.skill_report().unwrap().shadowed))
            .collect::<Vec<_>>()
    });
    assert_eq!(
        shadowed[1],
        ("codex".to_string(), vec!["demo-skill".to_string()])
    );
    assert!(shadowed[0].1.is_empty() && shadowed[3].1.is_empty());
}

#[test]
fn excluded_directories_are_never_read_or_touched() {
    let f = fx();
    f.skill("claude", "demo-skill", "body");
    f.skill("claude", ".system", "hidden");
    f.skill("claude", "synced", "managed");
    put(&f.skills("claude"), "container/inner/SKILL.md", "x");
    put(&f.skills("codex"), ".system/keep/SKILL.md", "x");
    put(&f.skills("codex"), "synced/SKILL.md", "x");
    let before: BTreeMap<_, _> = f
        .snapshot()
        .into_iter()
        .filter(|(k, _)| !k.contains("demo-skill"))
        .collect();
    f.sync();
    let after: BTreeMap<_, _> = f
        .snapshot()
        .into_iter()
        .filter(|(k, _)| !k.contains("demo-skill"))
        .collect();
    assert_eq!(before, after);
    for a in ["codex", "opencode", "pi"] {
        assert!(!f.skills(a).join("container").exists(), "{a}");
        assert!(!f.skills(a).join(".system/SKILL.md").exists(), "{a}");
    }
    assert!(!f.skills("pi").join("synced").exists());
    assert!(!f.skills("opencode").join("synced").exists());
}

#[test]
fn no_temporary_directory_is_left_behind() {
    let f = fx();
    f.skill("claude", "demo-skill", "v1");
    f.sync();
    put(&f.skills("claude"), "demo-skill/a.md", "a");
    f.sync();
    for a in AGENTS {
        let names: Vec<String> = fs::read_dir(f.skills(a))
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names, ["demo-skill"], "{a}");
    }
}

#[test]
fn the_mass_delete_guard_covers_skill_removals() {
    let f = fx();
    for i in 0..6 {
        f.skill("claude", &format!("demo-skill-{i}"), "body");
    }
    f.sync();
    for i in 0..6 {
        fs::remove_dir_all(f.skills("claude").join(format!("demo-skill-{i}"))).unwrap();
    }
    let before = f.snapshot();
    match f.try_sync(false) {
        Err(Error::MassDelete { planned, .. }) => assert_eq!(planned, 18),
        other => panic!("expected the guard, got {other:?}"),
    }
    assert_eq!(f.snapshot(), before);
    f.try_sync(true).unwrap();
    assert!(!f.holds("pi", "demo-skill-0"));
}

#[test]
fn a_second_plan_after_apply_is_empty() {
    let f = fx();
    f.skill("claude", "demo-skill", "body");
    f.skill("pi", "other-skill", "body");
    f.skill("codex", "third-skill", "body");
    f.sync();
    for n in ["demo-skill", "other-skill", "third-skill"] {
        f.all_agree(n);
    }
    assert!(f.plan().is_empty());
}

#[test]
fn a_skill_born_in_pi_has_pi_as_its_origin() {
    let f = fx();
    f.skill("pi", "demo-skill", "body");
    f.sync();
    let items = f.ledger.list_items().unwrap();
    assert_eq!(items[0].key.origin.as_str(), "pi");
    assert_eq!(items[0].kind, Kind::Skill);
    assert_eq!(items[0].scope, Scope::Global);
    f.all_agree("demo-skill");
}

#[test]
fn a_paused_skill_kind_is_neither_read_nor_written_and_what_exists_is_left_alone() {
    let mut f = fx();
    f.skill("claude", "demo-skill", "v1");
    f.sync();
    f.blocked_codex = vec![Kind::Skill];
    let codex_copy = fs::read(f.skills("codex").join("demo-skill/SKILL.md")).unwrap();
    f.with_adapters(|a| {
        assert!(!a[1].supports(Kind::Skill, &Scope::Global));
        assert!(a[1]
            .scan()
            .unwrap()
            .iter()
            .all(|o| o.item.kind != Kind::Skill));
    });
    put(&f.skills("claude"), "demo-skill/a.md", "a");
    let plan = f.plan();
    assert!(plan.actions.is_empty() && plan.notes.is_empty(), "{plan}");
    f.try_sync(false).unwrap();
    assert_eq!(
        fs::read(f.skills("codex").join("demo-skill/SKILL.md")).unwrap(),
        codex_copy
    );
    assert!(!f.skills("pi").join("demo-skill/a.md").exists());
    assert_eq!(f.ledger.list_items().unwrap().len(), 1);
}

#[test]
fn a_symlink_leading_outside_a_skill_is_skipped_and_reported() {
    #[cfg(unix)]
    {
        let f = fx();
        let d = f.skill("claude", "demo-skill", "body");
        fs::write(f.root.join("secret.txt"), "secret").unwrap();
        std::os::unix::fs::symlink(f.root.join("secret.txt"), d.join("leak.txt")).unwrap();
        f.sync();
        for a in &AGENTS[1..] {
            assert!(!f.skills(a).join("demo-skill/leak.txt").exists(), "{a}");
        }
        let report = f.with_adapters(|a| a[0].skill_report().unwrap());
        assert_eq!(report.skipped.len(), 1);
    }
}
