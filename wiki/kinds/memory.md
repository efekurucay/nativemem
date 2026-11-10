# Memory entries

## Purpose

How project memory entries are modelled, where each agent keeps them, how originals are created and
replicas are marked, and how edits and deletes travel.

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
| Codex | Blocks of the generated `<codex home>/memories/MEMORY.md` that start with `# Task Group: <title>` and contain `applies_to: cwd=<path>; ...`; `native_id` is `<cwd>#<slug(title)>`. Read-only. Entries created with `nativemem add` are store originals (below) | `<codex home>/memories/extensions/nativemem/resources/<project key>/` plus a routing block in `<codex home>/AGENTS.md` |
| OpenCode | Store originals: item files without a marker in the store below | `<opencode home>/nativemem/memory/<project key>/` plus a routing block in `<opencode home>/AGENTS.md` |
| Pi | Store originals: item files without a marker in the store below | `<pi home>/nativemem/memory/<project key>/` plus a routing block in `<pi home>/AGENTS.md` |

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
- `create_original` (`nativemem add`) writes `<slug(name)>.md` with nested frontmatter and no
  `nativemem_origin` marker, and the `MEMORY.md` index line. The project directory is found or
  created by the same function `write_replica` uses (`project_dir`). A file that already exists, or
  the name `memory` (the index), answers `Error::Exists`. Tests: `create_original_in_an_existing_project_has_no_marker_and_an_index_line`,
  `create_original_in_a_new_project_adds_the_helper_file_and_resolves`,
  `create_original_refuses_a_taken_file_and_the_index_name`.
- `write_original` replaces `name`, `description`, `category` and body, and keeps every other
  frontmatter key (test `write_original_preserves_unknown_keys`). `remove` backs up and deletes the
  file and its index line (test `remove_deletes_file_and_index_line_with_backups`).

## Store (Codex, OpenCode, Pi)

Shared implementation in `crates/core/src/adapters/memory_store.rs`. An item file with a marker line
is a replica of another agent's entry. An item file without one is an original of this agent: `scan`
returns it with no `replica_of`, so the engine treats it like any other original.

- `<base>/<project key>/scope.json` is `{"cwd": "<absolute path>"}`.
- `<base>/<project key>/MEMORY.md` is `# Memory Index` plus one line per item.
- `<base>/<project key>/<slug>.md`: first line `<!-- nativemem origin=<agent> id=<encoded native_id> -->`,
  then `# <name>`, a blank line, the description, a blank line, `Category: <category>` when present,
  a blank line, the body. A body that starts with `Category:` gets an empty `Category:` line so the
  parse stays unambiguous.
- An original has the same layout minus the marker line: `# <name>`, a blank line, the
  description, a blank line, `Category: <category>` when present, a blank line, the body.
- `create_original` (`nativemem add`) writes `scope.json`, the file `<slug(name)>.md`, the index
  line and the routing block; a file that already exists, or the name `memory`, answers
  `Error::Exists`. `write_original` rewrites a store original in place, backing the file up first,
  and refuses replicas and ids with `#` (Codex-generated originals stay read-only). `remove` backs
  up and deletes any item file of the store, original or replica. Tests in
  `crates/core/src/adapters/agents_file.rs`: `created_memory_is_an_original_with_index_scope_and_routing`,
  `created_memory_refuses_taken_and_reserved_names`,
  `store_original_is_rewritten_in_place_with_a_backup_and_replicas_are_not`,
  `store_original_is_removed_with_backup_and_its_routing`.
- Project key: slug of the last path component plus `-` and the first 8 hex characters of the
  SHA-256 of the full path (`project_key`; test `project_key_is_stable_and_distinct`).
- When the last item of a project goes, its directory and the empty parents below the agent home go
  with it (Codex keeps `resources`).
- An edit made to a replica file is written back to the original (tests
  `editing_a_pi_or_opencode_replica_writes_back_to_claude`, `replica_edit_updates_original_including_category`,
  `replica_edit_writes_back_to_a_created_original`). An original created in Pi, OpenCode or Codex
  replicates to the other agents (`created_memory_in_any_agent_reaches_the_others`) and its delete
  propagates (`deleting_a_created_original_removes_every_replica_and_routing`).

## Routing block

Agents without a native per-project memory get a block in their global instruction file between
`<!-- nativemem:begin -->` and `<!-- nativemem:end -->`. It carries the heading `## Project memory
(managed by nativemem)`, one sentence telling the agent to read the matching project's index first,
and one line per project: `- <cwd> → <index file>`. Content outside the markers is never changed;
the block is regenerated on every apply and removed when no project remains. Decision:
`wiki/decisions/routing-block-for-agents-without-memory.md`. Tests:
`routing_block_insert_update_remove_preserves_surroundings`,
`routing_block_created_when_agents_md_absent`. A project that holds only originals is listed too
(`routing_block_lists_a_project_that_only_has_originals`).

## Propagation

Faithful replication in both directions: adds, edits and deletes propagate, edits on a replica are
written back to the original, and edits on both sides are conflicts left alone. The full decision
table is in `wiki/architecture/overview.md`. Codex-generated originals come from Codex, so they are
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
- MUST: Never edit or delete a Codex-generated original (an id with `#`).
- MUST: Create an original only through `Adapter::create_original`, which refuses a taken file with `Error::Exists`.
- MUST: Edit the routing block only between its markers.
- SHOULD: Keep the item file layout in `memory_store.rs` so one module owns it.

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

2026-11-11
