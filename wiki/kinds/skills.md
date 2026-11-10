# Skills

## Purpose

How skills are modelled, where each agent keeps them, how copies are identified without a marker and
how additions, edits and deletes travel.

## Scope

`Kind::Skill` with `Scope::Global`, for Claude Code, Codex, OpenCode and Pi. Project-level skill
directories (inside repositories), plugin caches and agent-managed skills are not read or written.
Policy decision: `wiki/decisions/skills-mirror-without-shared-store.md`.

## Model

One item is one directory with a top-level `SKILL.md` directly inside the agent's own skills
directory. `name` is the directory name (the identity), `native_id` is `skill:<name>`,
`description` is the frontmatter `description` (best effort, empty when unparsable), `category` is
none, scope is global.

The item `body` is a manifest, never file contents: one line per regular file, sorted by relative
path, `<x|-> <size> <sha256> <relative path>`. `content_hash` therefore covers every path, the
executable bit and every byte. `Item::source_dir` points the writers at the real directory and is
not hashed. Ignored: `.DS_Store`, `__pycache__/`, `*.pyc`, `.git/`, `node_modules/`. A symlink inside
a skill is followed only when it resolves inside the same skill directory; otherwise it is skipped
and listed by `status`. A skill over 50 MB is skipped and listed by `status`
(`MAX_SKILL_BYTES`). Tests: `hash_ignores_noise_and_location_but_sees_content_and_exec_bit`,
`symlinks_inside_a_skill_follow_only_inside` in `crates/core/src/adapters/skills.rs`.

## Where skills live

| Agent | Skills directory | Also reads (never written) |
|---|---|---|
| Claude Code | `<claude home>/skills/<name>/` | |
| Codex | `<codex home>/skills/<name>/` | `~/.agents/skills` |
| OpenCode | `<opencode home>/skills/<name>/` | |
| Pi | `<pi home>/skills/<name>/` | `~/.agents/skills` |

Excluded from the unit: dot-directories (`.system`), `synced` (managed by Claude Code itself),
directories without a top-level `SKILL.md`, anything outside the agent's own skills directory.
A symlinked skill directory counts as a skill; its content is read through the link.
Test: `excluded_directories_are_never_read_or_touched` in `crates/core/tests/skills.rs`.

## Identity across agents

Copies carry no marker (skills are copied verbatim). `link_skills` in `crates/core/src/engine.rs`
makes the copies of one name one logical item: the origin is the origin the ledger records, else
the first agent in adapter order (claude, codex, opencode, pi) that holds it; the other copies are
its replicas. `items` applies the same linking, so `replica_of` is set for skills.

## Propagation

Mirror policy, the decision table of `wiki/architecture/overview.md` with these differences:

- A copy no ledger row accounts for is adopted only when its hash equals the original's; the same
  name with other content is a conflict note and nothing is written
  (`same_name_with_different_content_is_a_conflict_and_nothing_is_written`).
- First run on a machine where identical copies exist: adoption only, zero writes
  (`identical_copies_everywhere_change_nothing_on_disk`).
- A write-back from a replica does not rewrite the editing agent's own directory.
- Writes copy the tree into `.<name>.nativemem-tmp-<pid>` next to the target, verify its manifest,
  then swap by rename (`SkillStore::write`). The old directory is backed up whole first
  (`fsx::backup_dir`). Executable bits are kept. No temporary directory remains
  (`no_temporary_directory_is_left_behind`).
- A symlinked skill directory is never written through. When its content must change, the link is
  replaced by a real directory; the backup holds a text file `symlink -> <target>`. Removing it
  removes the link only (`a_symlinked_skill_counts_as_present_and_is_replaced_never_written_through`).
- Removals count toward the mass-delete guard (`the_mass_delete_guard_covers_skill_removals`).

## Visible roots

Codex and Pi read `~/.agents/skills` natively. The CLI passes `<user home>/.agents/skills` as the
visible root of both (`with_visible_roots`). For an agent without its own copy, the engine asks
`Adapter::visible_copy`: an identical copy there counts as present and nothing is written; a copy
with other content is not written over and the plan carries a skipped note
(`a_matching_copy_in_a_visible_root_means_no_copy_is_written`,
`a_visible_copy_with_other_content_is_not_written_over_and_is_reported`). When an agent's own copy
differs from a visible copy, `doctor` prints a notice. Pi also reads paths from its `settings.json`;
those are not visible roots, so a skill Pi can reach only that way is still copied into Pi's own
directory `[UNVERIFIED: duplicate-name behaviour]`.

## CLI and app

`items --kind skill`, `status` (`skills` per agent), `sync`, `delete --id skill:<name>`,
`skill show --agent <id> --name <skill>` (prints `SKILL.md`). `edit` answers `unsupported`. The app
lists skills on a "Skills" sidebar row (`docs/desktop.md`).

## Source anchors

- `crates/core/src/adapters/skills.rs`
- `crates/core/src/engine.rs` (`link_skills`, `plan_key`)
- `crates/core/src/fsx.rs` (`backup_dir`)
- `crates/core/src/model.rs` (`Kind::Skill`, `Item::source_dir`, `visible_copy`, `skill_report`)
- `crates/core/tests/skills.rs`
- `crates/cli/src/main.rs`

## Rules

- MUST: Copy skills as whole trees, never rewrite or translate a file.
- MUST: Never write through a symlink; back up the whole directory before an overwrite or removal.
- MUST: Never touch dot-directories, `synced` or directories without a top-level `SKILL.md`.
- MUST: Never write into a visible root (`~/.agents/skills`).
- SHOULD: Keep skill ids prefixed `skill:` so no other kind's id is mistaken for one.

## Verification checklist

- [ ] `crates/core/tests/skills.rs` passes
- [ ] `status` lists skipped skills and files with a reason
- [ ] The probe `<agent>.skills_layout` passes on the agent's directory

## Related docs

- `wiki/architecture/overview.md`
- `wiki/architecture/safety.md`
- `wiki/decisions/skills-mirror-without-shared-store.md`
- `wiki/guides/add-a-resource-kind.md`

## Last updated

2026-11-11
