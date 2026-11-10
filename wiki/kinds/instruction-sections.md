# Instruction sections

## Purpose

How a global instruction file becomes items, how replicas are kept inside it, and what is never
touched.

## Scope

`Kind::Instruction` with `Scope::Global`. Project-level instruction files live inside repositories
and are never read or written. Parsing and edits are in `crates/core/src/adapters/sections.rs`,
shared by all adapters.

## Files

| Agent | File |
|---|---|
| Claude Code | `<claude home>/CLAUDE.md` |
| Codex | `<codex home>/AGENTS.md` |
| OpenCode | `<opencode home>/AGENTS.md` |
| Pi | `<pi home>/AGENTS.md` |

A missing file means no items; the first replica creates it. All four adapters support instruction
originals: a section can originate in any agent.

## Items

- One item is one top-level section: the text from a `## ` heading through the line before the next
  `## ` heading. Deeper headings stay in the body.
- Splitting ignores `## ` lines inside fenced code blocks (backtick or tilde fences; a longer fence
  needs a matching close). Tests: `headings_inside_fences_do_not_split`,
  `longer_fence_needs_a_matching_close`.
- Everything before the first section (title, `@imports`, intro) is the preamble: never an item,
  never modified.
- `description` is the heading text, `name` is `slugify(heading)`, `body` is the text after the
  heading line with leading and trailing blank lines trimmed, `category` is none.
- `native_id` is `<file name>#<name>`, with `-2`, `-3` for duplicate headings (test
  `duplicate_headings_get_numbered_ids`).

## Replicas

- Replicas live in one region at the end of the file between `<!-- nativemem:instructions:begin -->`
  and `<!-- nativemem:instructions:end -->`.
- Each replica is a marker line `<!-- nativemem origin=<agent> id=<encoded native_id> -->`, then
  `## <heading>`, a blank line and the body. The id is percent-encoded for characters outside
  `[A-Za-z0-9._/-]` (test `replica_ids_with_odd_characters_round_trip`).
- Replicas are sorted by origin and native id. The region is created with the first replica and
  removed with the last, together with the blank line the tool added before it (test
  `region_is_created_sorted_updated_and_removed_with_its_blank_line`).
- Managed regions are excluded from original detection: the instruction region and the routing block
  (`<!-- nativemem:begin -->` to `<!-- nativemem:end -->`). A block's own `## ` heading is not a
  section (test `managed_regions_are_not_sections_and_end_a_section`). An unterminated region is an
  error, not a guess.
- Bytes outside the region are never changed by replica writes (test
  `rewrite_body_keeps_every_other_byte`).

## Edits and deletes

- `write_original` rewrites the section body in place and keeps the heading line; every other byte
  of the file stays. `remove` deletes the section. Both back up the file first.
- The heading is part of identity. A heading edited by hand is a delete plus an add. When a replica's
  heading is edited, only the body travels back, and the heading is restored (test
  `replica_heading_edit_is_restored`).
- CLI `edit` on an instruction item changes the body only and answers code `invalid` without writing
  when the submitted `name` or `description` differs from the current ones (test
  `edit_instruction_heading_is_invalid_and_writes_nothing`).
- Deletes propagate like memory entries (test `deleting_original_sections_removes_replicas_and_empty_region`).

## Interaction with agents' own loading

- OpenCode reads `~/.claude/CLAUDE.md` only while its own global `AGENTS.md` is absent. Creating that
  file replaces the fallback; the synced region carries the Claude Code sections over. See
  `wiki/agents/opencode.md`.
- Pi loads the first of `AGENTS.override.md`, `AGENTS.md`, `AGENTS.MD`, `CLAUDE.md`, `CLAUDE.MD` in
  each directory, so an `AGENTS.override.md` in its home shadows the synced `AGENTS.md`. See
  `wiki/agents/pi.md`.
- Hermes has no global instruction file equivalent and no adapter. See `wiki/agents/hermes.md`.

## Source anchors

- `crates/core/src/adapters/sections.rs`
- `crates/core/src/adapters/agents_file.rs`
- `crates/core/src/adapters/claude.rs`
- `crates/core/src/adapters/codex.rs`
- `crates/cli/src/main.rs` (`edit`)
- `docs/design.md` (Instruction sections)

## Rules

- MUST: Never modify the preamble or any byte outside the managed regions and the edited section.
- MUST: Keep the heading as identity; never rename a section through a replica.
- MUST: Back up the instruction file before every rewrite.
- SHOULD: Treat a parse error on an unterminated region as a stop, not something to repair.

## Verification checklist

- [ ] A new instruction file path is added to the table and to the agent page
- [ ] Round-trip and byte-preservation tests exist for a new adapter
- [ ] The marker format matches `marker` in `crates/core/src/adapters/sections.rs`

## Related docs

- `wiki/kinds/memory.md`
- `wiki/decisions/instruction-sections-as-items.md`
- `wiki/architecture/overview.md`

## Last updated

2026-11-11
