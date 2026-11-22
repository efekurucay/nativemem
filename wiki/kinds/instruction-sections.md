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

A missing file means no items; the first replica or the first created section creates it. All four adapters support instruction
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

- A replica is ordinary Markdown: a `## <heading>`, a blank line and its body. New copies are
  appended to the receiving agent's global file. Identity stays in
  `<state dir>/instruction-origins/<agent>/<profile hash>.json`. The state contains section
  fingerprints and source ids, and stores no instruction text.
- The scanner matches unchanged sections by heading and body fingerprints, then matches a unique
  heading or body to recognize an edit. An ambiguous match stops the scan before planning any
  deletion. Identical sections can use their relative order when the entire matching group has
  the same size.
- Full apply adopts replica identities from inline synchronization comments, backs up the native
  file, and removes the comments while preserving the preamble, headings and bodies. The dry run
  and the project-scoped session-only apply leave the native instructions alone.
- Before replacing the file, the writer records the file hashes and identities for both sides.
  The file hash selects the correct identities if a run stops during the write.
- Explicit edits and deletes touch the selected section. Other rule text stays in place.

## Creating a section

- `InstructionFile::create_original` (CLI `add --kind instruction`) appends `## <description>`, a
  blank line and the body as a new user section, before the first replica or at the end of the file. One blank line separates
  it from what precedes and follows it; every byte outside the inserted text stays identical. A
  missing file is created. Tests: `insert_section_goes_before_the_first_managed_region_and_keeps_every_other_byte`,
  `insert_section_without_a_region_appends_with_one_blank_line`.
- `native_id` is `<file>#<slug(description)>`. When that id exists (a duplicate heading counts, as
  its numbered ids keep the plain one), the call fails with `Error::Exists` and writes nothing (test
  `insert_section_refuses_an_id_that_is_taken`).
- A heading is one non-empty line. A body that would start another `## ` section outside a fence, or
  holds a `<!-- nativemem` marker, would not read back as one section, so `section_problem` rejects it
  (CLI code `invalid`; test `section_problem_rejects_what_would_not_read_back`).
- The section reaches the other agents at the next sync (test `created_instruction_in_any_agent_reaches_the_others`).

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
  file replaces the fallback; the synced Markdown sections carry the Claude Code sections over. See
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
- `crates/cli/src/main.rs` (`add`, `edit`)
- `docs/design.md` (Instruction sections)

## Rules

- MUST: Never modify the preamble or any byte outside the edited section and its separator.
- MUST: Keep the heading as identity; never rename a section through a replica.
- MUST: Back up the instruction file before every rewrite.
- MUST: Create a section only through `create_original`, which refuses a taken id with `Error::Exists`.
- SHOULD: Stop on ambiguous replica identity or incomplete synchronization markup.

## Verification checklist

- [ ] A new instruction file path is added to the table and to the agent page
- [ ] Round-trip and byte-preservation tests exist for a new adapter
- [ ] Native files contain ordinary instruction sections and replica identity stays in the state directory

## Related docs

- `wiki/kinds/memory.md`
- `wiki/decisions/instruction-sections-as-items.md`
- `wiki/architecture/overview.md`

## Last updated

2026-11-22
