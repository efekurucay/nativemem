# Memory entries

## Purpose

How project memory entries are modelled, where each agent keeps them, how replicas are marked and
how edits and deletes travel.

## Scope

`Kind::Memory` with `Scope::Project(<cwd>)`. Instruction sections are in
`wiki/kinds/instruction-sections.md`, sessions in `wiki/kinds/sessions.md`. Global-scope memory is
not implemented: every adapter answers `supports(Memory, Global)` with false.

## Model

One memory entry is one item: `name` (short slug), `description` (one line), `category` (`user`,
`feedback`, `project`, `reference` or none), `body` (markdown, copied verbatim). Scope is the
absolute working directory of the project. Text is never rewritten.

## Where entries live

| Agent | Originals | Replicas |
|---|---|---|
| Claude Code | `<claude home>/projects/<dir>/memory/*.md` except `MEMORY.md`; `native_id` is `<dir>/<file>` | Same directory; frontmatter marker; a line in that project's `MEMORY.md` |
| Codex | Blocks of the generated `<codex home>/memories/MEMORY.md` that start with `# Task Group: <title>` and contain `applies_to: cwd=<path>; ...`; `native_id` is `<cwd>#<slug(title)>`. Read-only | `<codex home>/memories/extensions/nativemem/resources/<project key>/` plus a routing block in `<codex home>/AGENTS.md` |
| OpenCode | none | `<opencode home>/nativemem/memory/<project key>/` plus a routing block in `<opencode home>/AGENTS.md` |
| Pi | none | `<pi home>/nativemem/memory/<project key>/` plus a routing block in `<pi home>/AGENTS.md` |

Anchors: `crates/core/src/adapters/claude.rs`, `crates/core/src/adapters/codex.rs`,
`crates/core/src/adapters/agents_file.rs`, `crates/core/src/adapters/memory_store.rs`.

## Claude Code store

- Project directory: the directory name is a lossy encoding of the working directory (every
  non-alphanumeric character becomes `-`), so the real path is read from the `cwd` field of the first
  line that has one in any `projects/<dir>/*.jsonl`. Lines that carry the session marker
  `nativemem_origin` are ignored for this, because a replica written by this tool is not evidence
  that the folder exists. A directory without a usable `cwd` is resolved by decoding its name against
  real directories only when exactly one path matches; otherwise it is skipped and listed by
  `status`. Tests: `cwd_resolution_and_skipped_project`, `name_decoded_against_filesystem_when_no_jsonl`,
  `ambiguous_decoding_stays_skipped`, `replica_sessions_do_not_make_a_missing_folder_resolve`.
- Frontmatter: both variants parse. Nested: `name`, `description`, `metadata: { type, node_type,
  originSessionId, modified }`. Flat: `name`, `description`, `type`, `originSessionId`. `category` is
  `metadata.type` or `type`. Test: `scan_both_variants_and_quoted`.
- Replica marker: `metadata.nativemem_origin: "<agent>:<native_id>"`.
- `write_replica` writes `<slug>.md` with nested frontmatter and the body, and keeps the line
  `- [<name>](<file>) — <description>` in that project's `MEMORY.md` (created with `# Memory Index`
  when absent). A project directory that does not exist is created with the encoded name and a
  helper file `nativemem.jsonl` holding the `cwd`, so the directory resolves later. Tests:
  `write_replica_existing_project_and_index`, `write_replica_new_project`.
- `write_original` replaces `name`, `description`, `category` and body, and keeps every other
  frontmatter key (test `write_original_preserves_unknown_keys`). `remove` backs up and deletes the
  file and its index line (test `remove_deletes_file_and_index_line_with_backups`).

## Replica store (Codex, OpenCode, Pi)

Shared implementation in `crates/core/src/adapters/memory_store.rs`:

- `<base>/<project key>/scope.json` is `{"cwd": "<absolute path>"}`.
- `<base>/<project key>/MEMORY.md` is `# Memory Index` plus one line per item.
- `<base>/<project key>/<slug>.md`: first line `<!-- nativemem origin=<agent> id=<encoded native_id> -->`,
  then `# <name>`, a blank line, the description, a blank line, `Category: <category>` when present,
  a blank line, the body. A body that starts with `Category:` gets an empty `Category:` line so the
  parse stays unambiguous.
- Project key: slug of the last path component plus `-` and the first 8 hex characters of the
  SHA-256 of the full path (`project_key`; test `project_key_is_stable_and_distinct`).
- When the last item of a project goes, its directory and the empty parents below the agent home go
  with it (Codex keeps `resources`).
- An edit made to a replica file is written back to the original (tests
  `editing_a_pi_or_opencode_replica_writes_back_to_claude`, `replica_edit_updates_original_including_category`).

## Routing block

Agents without a native per-project memory get a block in their global instruction file between
`<!-- nativemem:begin -->` and `<!-- nativemem:end -->`. It carries the heading `## Project memory
(managed by nativemem)`, one sentence telling the agent to read the matching project's index first,
and one line per project: `- <cwd> → <index file>`. Content outside the markers is never changed;
the block is regenerated on every apply and removed when no project remains. Decision:
`wiki/decisions/routing-block-for-agents-without-memory.md`. Tests:
`routing_block_insert_update_remove_preserves_surroundings`,
`routing_block_created_when_agents_md_absent`.

## Propagation

Faithful replication in both directions: adds, edits and deletes propagate, edits on a replica are
written back to the original, and edits on both sides are conflicts left alone. The full decision
table is in `wiki/architecture/overview.md`. Codex originals are generated by Codex, so they are
read and replicated but never edited or deleted by `nativemem`; blocks without `applies_to` are
reported as unscoped and skipped (test `task_groups_parse_with_unscoped_and_duplicates`).

## Source anchors

- `crates/core/src/model.rs`
- `crates/core/src/adapters/claude.rs`
- `crates/core/src/adapters/codex.rs`
- `crates/core/src/adapters/memory_store.rs`
- `crates/core/src/adapters/agents_file.rs`
- `crates/core/src/engine.rs`
- `docs/design.md` (Claude Code adapter, Codex adapter)

## Rules

- MUST: Copy text verbatim; never summarise or rewrite an entry.
- MUST: Preserve every frontmatter key `nativemem` does not own when it edits a Claude original.
- MUST: Never edit or delete a Codex-generated original.
- MUST: Edit the routing block only between its markers.
- SHOULD: Keep the replica layout in `memory_store.rs` so one module owns it.

## Verification checklist

- [ ] Paths in the table match `layout` in `crates/core/src/adapters/codex.rs` and `MEMORY_DIR` in `crates/core/src/adapters/agents_file.rs`
- [ ] A new store layout has a round-trip test with tricky bodies
- [ ] A write never lands outside the declared base directory and the agent's `AGENTS.md`

## Related docs

- `wiki/kinds/instruction-sections.md`
- `wiki/agents/claude-code.md`
- `wiki/agents/codex.md`
- `wiki/decisions/routing-block-for-agents-without-memory.md`

## Last updated

2026-11-10
