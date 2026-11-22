# Decision: instruction sections are items

## Purpose

Record why a global instruction file is synced as one item per `## ` section instead of as one file.

## Scope

`Kind::Instruction`. Mechanics are in `wiki/kinds/instruction-sections.md`.

## Decision

One item is one top-level section of a global instruction file: the text from a `## ` heading through
the line before the next `## ` heading. The heading is the item's identity. Everything before the
first section is preamble and is never an item. Replicas live as ordinary Markdown sections in the
receiving file. Their identity is recorded by the sync tool in its state directory.

## Why

- Each user's instruction files hold personal rules next to shared ones; whole-file sync would
  overwrite a file the user owns, and the files differ legitimately between agents.
- Sections give the three-way decision a unit small enough to avoid false conflicts: an edit to one
  section does not collide with an edit to another.
- Independent section identity lets the agent read a regular Markdown file. The sync tool can
  match edits using section fingerprints and apply changes to the intended section.
- Using the heading as identity means a renamed heading is a delete plus an add, and a replica's
  heading edit is restored while its body edit travels back.

## Consequences

- Duplicate headings in one file get numbered ids (`-2`, `-3`).
- Fences matter: `## ` lines inside code fences are not headings.
- The CLI `edit` changes the body only and answers `invalid` when the heading differs.
- Agents whose global file is shadowed by another (OpenCode's fallback to the Claude file, Pi's
  `AGENTS.override.md`) need the shadowing considered on their agent pages.

## Source anchors

- `crates/core/src/adapters/sections.rs`
- `crates/cli/src/main.rs` (`edit`)
- `docs/design.md` (Instruction sections)

## Rules

- MUST: Never modify the preamble.
- MUST: Treat the heading as identity.
- MUST: Keep replica identity in the state directory.

## Verification checklist

- [ ] Tests `replica_heading_edit_is_restored` and `rewrite_body_keeps_every_other_byte` pass
- [ ] Replica identity and ambiguity handling match `sections.rs`

## Related docs

- `wiki/kinds/instruction-sections.md`
- `wiki/agents/opencode.md`
- `wiki/agents/pi.md`

## Last updated

2026-11-10
