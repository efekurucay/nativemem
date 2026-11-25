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
| Codex | Blocks of the generated `MEMORY.md` in the selected Codex memory store that start with `# Task Group: <title>` and contain `applies_to: cwd=<path>; ...`; `native_id` is `<cwd>#<slug(title)>`. Read-only. Entries created with `nativemem add` are native ad-hoc inputs | `<selected Codex memory store>/extensions/ad_hoc/notes/` |
| OpenCode | Store originals: item files without a marker in the store below | `<state dir>/memory/opencode/<profile key>/<project key>/` |
| Pi | Store originals: item files without a marker in the store below | `<state dir>/memory/pi/<profile key>/<project key>/` |

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
- Replica identity is stored by profile in `<state dir>/claude-memory-origins/`, keyed by the
  Claude project directory and note filename. A full apply adopts inline origin metadata into
  that state, backs up the note, and removes the metadata line while preserving the other bytes.
  Removing a replica also clears its identity.
- `write_replica` writes `<slug>.md` with nested frontmatter and the body, and keeps the line
  `- [<name>](<file>) — <description>` in that project's `MEMORY.md` (created with `# Memory Index`
  when absent). A project directory that does not exist is created with the encoded name and a
  helper file `nativemem.jsonl` holding the `cwd`, so the directory resolves later. Tests:
  `write_replica_existing_project_and_index`, `write_replica_new_project`.
- `create_original` (`nativemem add`) writes `<slug(name)>.md` with native frontmatter and
  the `MEMORY.md` index line. The project directory is found or
  created by the same function `write_replica` uses (`project_dir`). A file that already exists, or
  the name `memory` (the index), answers `Error::Exists`. Tests: `create_original_in_an_existing_project_has_no_marker_and_an_index_line`,
  `create_original_in_a_new_project_adds_the_helper_file_and_resolves`,
  `create_original_refuses_a_taken_file_and_the_index_name`.
- `write_original` replaces `name`, `description`, `category` and body, and keeps every other
  frontmatter key (test `write_original_preserves_unknown_keys`). `remove` backs up and deletes the
  file and its index line (test `remove_deletes_file_and_index_line_with_backups`).

### Index repair

`repair-memory-index --project-dir=<native directory> --json` previews the note links it can
add or remove. Pass the preview's token as `--apply=<token>` to apply under the state lock.
The token covers the current index and the proposed text. The operation preserves unrelated text,
backs up an existing index, and refuses symlinked project directories or indexes. The desktop
opens this preview from Review in the existing memory list.

## App-managed notes (OpenCode, Pi)

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
  line. A file that already exists, or the name `memory`, answers
  `Error::Exists`. `write_original` rewrites a store original in place, backing the file up first,
  and refuses replicas and ids with `#` (Codex-generated originals stay read-only). `remove` backs
  up and deletes any item file of the store, original or replica. Tests in
  `crates/core/src/adapters/agents_file.rs`: `created_memory_is_an_original_with_index_and_scope`,
  `created_memory_refuses_taken_and_reserved_names`,
  `store_original_is_rewritten_in_place_with_a_backup_and_replicas_are_not`,
  `store_original_is_removed_with_backup`.
- Project key: slug of the last path component plus `-` and the first 8 hex characters of the
  SHA-256 of the full path (`project_key`; test `project_key_is_stable_and_distinct`).
- The profile key is the SHA-256 of the agent home path. A full apply or explicit note mutation
  copies recognized notes from an existing agent-home store into the application state. Files
  retain their IDs and bodies. The source files are preserved and the app reads the state copy
  after adoption.
- Removing the final note also removes the empty project directory.
- An edit made to a replica file is written back to the original (tests
  `editing_a_pi_or_opencode_replica_writes_back_to_claude`, `replica_edit_updates_original_including_category`,
  `replica_edit_writes_back_to_a_created_original`). An original created in Pi, OpenCode or Codex
  replicates to the other agents (`created_memory_in_any_agent_reaches_the_others`). Explicit
  removal preserves unselected copies (`deleting_a_created_original_preserves_every_replica`).

## Global instructions

Memory writes are independent of instruction files. Full apply removes the generated project-memory
block from the global instruction files of Codex, OpenCode and Pi with a backup. The auxiliary
store remains available in the manager. Its entries do not automatically become native agent
memory. Decision: `wiki/decisions/routing-block-for-agents-without-memory.md`.

On OpenCode 2.3.2 and Pi 1.6.0, fresh-session checks with isolated profiles and a local model
endpoint confirmed that both clients load their `AGENTS.md` control rule. Standalone
`MEMORY.md` fixtures in plausible project and profile locations did not enter the same requests.
The per-agent evidence and tested locations are in [OpenCode](../agents/opencode.md#facts-and-evidence)
and [Pi](../agents/pi.md#facts-and-evidence).

## Native memory in Codex

Synced conversations are eligible inputs for Codex's own extraction and consolidation.
Standalone notes go into `extensions/ad_hoc/notes/` under the selected Codex memory store.
The note is Markdown with the project path followed by its title, description and body. The
Nativemem state directory records the logical id, origin and filename, so Codex's input contains
the note itself. Full apply adopts readable manager entries in place logically, keeps their source
files, and writes their Codex inputs once. Session-only apply handles conversations only.

Codex generates its own summary and loads it into eligible later sessions. Nativemem can edit
its own ad-hoc inputs after a backup. Explicit removal writes a request for Codex to forget the
selected note and backs up the entire input first. Generated memory remains read-only.

## Propagation

Adds and edits replicate in both directions. Edits on both sides are reported as conflicts.
An absent memory original or replica is reported for review while other copies exist.
Remove a selected copy explicitly with `delete` or use Review Cards to select the source or all
linked copies. The engine can discard its ledger entry when all copies are gone.
Instruction and skill removal follows the engine's separate synchronization rules.

## Source anchors

- `crates/core/src/model.rs`
- `crates/core/src/adapters/claude.rs`
- `crates/core/src/adapters/codex.rs`
- `crates/core/src/adapters/codex_memory.rs`
- `crates/core/src/adapters/memory_store.rs`
- `crates/core/src/adapters/agents_file.rs`
- `crates/core/src/engine.rs`
- `docs/design.md` (Claude Code adapter, Codex adapter)

## Rules

- MUST: Copy text verbatim; never summarise or rewrite an entry.
- MUST: Preserve every frontmatter key `nativemem` does not own when it edits a Claude original.
- MUST: Never edit or delete a Codex-generated original (an id with `#`).
- MUST: Create an original only through `Adapter::create_original`, which refuses a taken file with `Error::Exists`.
- MUST: Preserve other instruction text when removing a generated memory-routing block.
- SHOULD: Keep the item file layout in `memory_store.rs` so one module owns it.

## Verification checklist

- [ ] Paths in the table match `codex_memory.rs` and `MEMORY_DIR` in `crates/core/src/adapters/agents_file.rs`
- [ ] A new store layout has a round-trip test with tricky bodies
- [ ] A write never lands outside the declared base directory and the agent's `AGENTS.md`

## Related docs

- `wiki/kinds/instruction-sections.md`
- `wiki/agents/claude-code.md`
- `wiki/agents/codex.md`
- `wiki/decisions/routing-block-for-agents-without-memory.md`

## Last updated

2026-11-25
