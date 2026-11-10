# Skills are mirrored between the agents' own directories

## Purpose

Records why skills use the mirror policy with real copies in each agent's own skills directory.

## Scope

`Kind::Skill`. Page: `wiki/kinds/skills.md`.

## Decision

Every agent behaves as if it were the only agent: a skill written in one agent appears as a native
skill in the skills directory of each other agent. Copies are real directories, because each agent
reads only its own directory (plus `~/.agents/skills` for Codex and Pi). No shared store, plugin or
configuration edit is introduced (`wiki/decisions/no-shared-store.md`). Policy is faithful
replication like memory: adds, edits and deletes propagate, edits on both sides are a conflict, and
every overwrite or removal backs up the whole directory. Identity is the directory name; copies have
no marker, so identical copies are adopted and different ones are conflicts.

## Source anchors

- `crates/core/src/engine.rs`
- `crates/core/src/adapters/skills.rs`

## Rules

- MUST: Keep skills out of any shared directory this tool owns.
- SHOULD: Treat a matching copy in a native read root as present.

## Verification checklist

- [ ] `wiki/kinds/skills.md` matches the code

## Related docs

- `wiki/kinds/skills.md`
- `wiki/decisions/no-shared-store.md`

## Last updated

2026-11-11
