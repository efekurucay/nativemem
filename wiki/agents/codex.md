# Codex

## Purpose

What `nativemem` needs to know about the Codex CLI and desktop app to read its memory, instruction
sections and rollouts, and how session writes into Codex are guarded.

## Scope

Facts gathered read-only on a development machine running codex-cli 0.167.0 (rollouts from 0.149.0
to 0.167.0; 215 live rollouts, 5 archived, 641 rows in the `threads` table, 79 sessions imported from
Claude Code). SQLite files were inspected read-only (`immutable=1`, structure and non-content values). The `codex` binary is never driven by this repo.

## Status in nativemem

| Kind | Status | Adapter |
|---|---|---|
| Project memory | Originals read from generated `memories/MEMORY.md` (read-only, never edited or deleted); replicas written to a nativemem-owned store plus a routing block | `crates/core/src/adapters/codex.rs`, `memory_store.rs` |
| Instruction sections | Read and write in `AGENTS.md` | `codex.rs` through `sections.rs` |
| Sessions | Originals always read; replicas (with their `threads` row) written only when `NATIVEMEM_CODEX_SESSION_WRITES=on` and the schema probes pass | `codex_sessions.rs`, `codex_rollout.rs`, `codex_imports.rs`, `codex_threads.rs` |
| Skills (global) | Read and write in `<codex home>/skills/`; `.system` and directories without `SKILL.md` are left alone; a matching copy in `~/.agents/skills` counts as present | `crates/core/src/adapters/skills.rs`, `codex.rs` |
| MCP servers | Read by `mcp import`; never written | `crates/hub/src/import.rs` |

## Native locations

| What | Global | Project |
|---|---|---|
| Instructions | `<codex home>/AGENTS.md` | `<repo>/AGENTS.md` (never touched) |
| Memory | generated `<codex home>/memories/MEMORY.md` and `memory_summary.md`; replica store `<codex home>/memories/extensions/nativemem/resources/<project key>/` | blocks inside the generated file carry `applies_to: cwd=<path>` |
| Sessions | `<codex home>/sessions/YYYY/MM/DD/rollout-<ts>-<thread id>.jsonl`, `archived_sessions/` | |
| Session catalogue | `<codex home>/state_5.sqlite` table `threads` | |
| Import ledger | `<codex home>/external_agent_session_imports.json` | |
| MCP servers | `<codex home>/config.toml` tables `[mcp_servers.<name>]` | |
| Skills | `<codex home>/skills/<name>/` (`.system/` and container directories without `SKILL.md` are left alone), `~/.agents/skills` (read, never written), plugin caches | `<repo>/.agents/skills` |

The Codex home is `--codex-home`, else `NATIVEMEM_CODEX_HOME`, else `CODEX_HOME`, else `~/.codex`.

## Formats

- **Generated memory**: `memories/MEMORY.md` holds blocks that start with `# Task Group: <title>` and
  contain a line `applies_to: cwd=<path>; ...`. Each block is one item: `native_id`
  `<cwd>#<slug(title)>`, `name` the slug, `description` the title, `body` the block text. Blocks
  without `applies_to` are reported as unscoped and skipped.
- **Replica store**: described in `wiki/kinds/memory.md`. The `extensions/.../resources/<project>/scope.json`
  layout follows what Codex's own memory prompt describes; it is not officially documented. Loading
  is guaranteed by the routing block in `AGENTS.md`, not by that layout.
- **Rollout** (JSONL, `{timestamp, [ordinal], type, payload}`): line types `session_meta`,
  `turn_context`, `event_msg`, `response_item`, `token_usage_record`, `compacted` and others. User and
  assistant text appears twice, as `response_item` messages and as events (`item_completed`
  `UserMessage` / `AgentMessage` in the paginated history mode, `user_message` / `agent_message` in
  the other mode, which has no `ordinal` fields). Tool calls are `function_call` and
  `custom_tool_call` items with outputs matched by `call_id`. Reasoning is an opaque encrypted blob.
- **`threads` row**: `id`, absolute `rollout_path`, `created_at`, `updated_at`, `source`, `cwd`,
  `title`, `first_user_message`, `preview`, `archived`, `history_mode`, and many cache columns.
- **Importer ledger**: `{"records": [{source_path, content_sha256, imported_thread_id, imported_at,
  source_modified_at, connector_names, title}], "detected_connector_records": [...]}`.
  `content_sha256` is the SHA-256 of the raw bytes of the source transcript.
- **MCP entry**: `command`, `args`, `env`, `env_vars`, `cwd`, `url`, `http_headers`,
  `bearer_token_env_var`, per-tool approval settings. `mcp import` carries `command`, `args`, `env`, `cwd`,
  `url`, `http_headers` and `enabled`, and names the other keys in a warning.

## What nativemem reads and writes today

- Reads `memories/MEMORY.md` blocks, `AGENTS.md` sections, `memories/extensions/nativemem/resources/`
  replicas and `sessions/` rollouts (directory depth at most 4 below `sessions/`).
- Writes replica memory items and the routing block, the instruction region at the end of
  `AGENTS.md`, and, only with `NATIVEMEM_CODEX_SESSION_WRITES=on` while the schema probes pass, rollout files, the importer-ledger record for
  Claude-origin sessions and the `threads` row of every replica rollout (below). The ledger file is parsed and rewritten whole (every other key
  and record kept, key order not), backed up first, created only when the Codex home exists, and left
  alone when it does not parse.
- A rollout written by this tool is shaped like Codex's own importer output: `session_meta`
  (`originator` `Codex Desktop` for a Claude origin, the origin agent's display name otherwise;
  `cli_version` and `source` as the importer writes them), one `external-import-turn-N` turn per user
  message with event and `response_item` pairs, the `<EXTERNAL SESSION IMPORTED>` marker event once,
  a `token_count` and the closing `task_complete`. `session_meta` also carries `nativemem_origin` and
  `nativemem_title`. The file mtime is the original's.
- Never touches `~/.codex/sqlite/`, `thread_history_1.sqlite`, `session_index.jsonl`, `history.jsonl`,
  `.codex-global-state.json`, `config.toml` or `hooks.json`, and in `state_5.sqlite` only the
  `threads` rows of its own replicas.

## Sessions: listing and resume

- Codex's list is driven by the `threads` table. A thread appears with a rollout file plus a row that
  has a non-empty `preview`, `archived = 0` and a valid absolute `rollout_path` (INFERRED from the
  data; `codex resume` was not run). Other places that hold an imported thread are caches or UI state
  (`session_index.jsonl`, `history.jsonl`, `thread_history_1.sqlite`, `sqlite/codex.db`,
  `.codex-global-state.json`); imports were present in `threads` and absent from several of them.
- A session imported by Codex itself has a row inserted by the importer directly (`originator` NULL),
  `cwd` equal to the first cwd in the Claude file, `title` from the Claude title priority, and
  `created_at` / `updated_at` equal to the first and last message times of the source.
- Codex's `backfill_state` is `complete`, so a rollout without a row is not picked up later
  (OBSERVED: 6 written rollouts had no row and were not listed).

## Thread row writer

`crates/core/src/adapters/codex_threads.rs`, called (only with the switch on) from `Adapter::finish` at the end of `sync --apply`.
The row mirrors what Codex's importer inserts for an imported Claude thread (OBSERVED on 73 imported
rows against 443 native ones) and is derived from the rollout file, so file and row agree:

| Column | Value |
|---|---|
| `id` | thread id of `session_meta.id` |
| `rollout_path` | absolute path of the rollout (importer rows are absolute) |
| `created_at`, `updated_at` | first `task_started.started_at` and last `task_complete.completed_at`, in seconds |
| `created_at_ms`, `updated_at_ms`, `recency_at_ms` | seconds times 1000; `recency_at` is `updated_at` |
| `cwd`, `title`, `name` | the session's cwd and title (importer rows have `name` equal to `title`) |
| `first_user_message`, `preview` | text of the first user turn (importer rows have them equal, untruncated) |
| `source`, `model_provider`, `cli_version`, `history_mode` | the values written into `session_meta` (`vscode`, `openai`, `CLI_VERSION`, `legacy`) |
| `sandbox_policy`, `approval_mode` | `{"type":"read-only"}`, `on-request` (importer values) |
| `tokens_used`, `has_user_event`, `archived` | 0 |
| everything else | default or NULL (`originator`, `model`, git columns, `thread_source`, project and section columns, `memory_mode` default `enabled`) |

- The five triggers on `threads` only fill `*_at_ms` and `recency_at*` when they are NULL or 0; every
  value is set explicitly, so none is relied on. No FTS table, view or other table needs a feed for
  the picker. `local_thread_catalog` (`sqlite/codex.db`) is filled by the app's own scan (49 of 73
  imported threads present), `session_index.jsonl` held none of the imported threads, and
  `thread_history_1.sqlite` holds turn projections for `paginated` threads only (none of the legacy
  rows), so none of them is written (INFERRED; the previous research said the same).
- Rows exist only for rollouts with `nativemem_origin`. A forced refresh (the engine rewrote the
  rollout) replaces the derived columns and keeps `name`, `archived`, `is_pinned`, `tokens_used` and
  everything else Codex or the user set. A rollout that lost its row is inserted on the next run, and
  an existing row is advanced only when the rollout's last message is newer than the row's
  `updated_at`, so a thread resumed in Codex is never reverted. Nothing is deleted.
- Before the first write of a run the database is copied with SQLite's online backup to
  `<state dir>/backups/<stamp>/codex/state_<n>.sqlite`; the 5 newest copies are kept. The write is one
  `BEGIN IMMEDIATE` transaction after `busy_timeout` of 5 s, read back afterwards. A locked database
  gives a `skipped` note and the same work happens on the next run. A home without a state database
  gets no rows until Codex creates it.
- The row says `history_mode` `legacy` like the rollout; Codex's importer writes `paginated` and
  builds `thread_history_1.sqlite` for it. Whether Codex lists and opens a `legacy` thread written
  this way is `[UNVERIFIED]`.

## Quirks

- **Codex hash-pins hooks and rewrites `config.toml`.** `[hooks.state.<id>]` tables hold
  `trusted_hash` values that pin each hook's command text; changing a hook without matching the hash
  disables it. Codex edits the file with a layout-preserving TOML editor and appends
  `[projects."<path>"] trust_level` and plugin tables itself. `nativemem` never edits it.
- **Generated memory is Codex's.** `memories/MEMORY.md` and `memory_summary.md` are regenerated, so
  `nativemem` never writes them; originals there are read-only (test `originals_are_read_only`).
- **Codex's own importer renders tool calls differently.** It writes `[external_agent_tool_call: <tool>]`
  pseudo-call blocks as assistant text; `nativemem` renders `Tool: <name>` blocks so all targets
  read the same (`wiki/kinds/sessions.md`).
- **Import dedup uses the transcript bytes.** After this tool writes a replica of a Claude session it
  adds the importer record so Codex does not import it again. A session Codex already imported
  counts as its copy (adopted with an empty hash, never rewritten).
- **Headers differ by Codex version.** Fields such as `history_mode`, `paginated` and `ordinal`
  changed between 0.164 and 0.167. The writer uses the history mode without `ordinal` fields, the
  simplest shape.
- **Sub-agent threads started by Claude Code** carry originator `Claude Code` and a
  `parent_thread_id`; they are automated sessions, not imports.
- **A live Codex app writes `state_5.sqlite` in WAL mode** and holds lock files. The row writer uses
  SQLite's own locking (5 s `busy_timeout`, `BEGIN IMMEDIATE`), never changes the journal mode and skips
  a locked database.
- **Context cap**: Codex loads project docs up to `project_doc_max_bytes = 32768` (binary strings,
  OBSERVED), which matters for the routing block size.

## Version detection and compatibility probes

The version is `latest_version` of `<codex home>/version.json` (the version Codex's update check
last saw), else `payload.cli_version` of the newest rollout (`version` in
`crates/core/src/adapters/codex_probes.rs`). The rollout writer pins `CLI_VERSION`
(`0.167.0-alpha.3.2`) and a short base-instructions text in
`crates/core/src/adapters/codex_rollout.rs`; a rollout records the writing version in
`session_meta.cli_version`.

The probes (`wiki/architecture/compatibility.md`) open `state_<n>.sqlite` read-only with
`immutable=1`, so no `-wal`, `-shm` or journal file is created and nothing is written. They read the
newest 50 rollouts by date path. A home without a state database, rollout or import ledger passes
these checks.

| Check id | Protects | Expected constant | Fails when |
|---|---|---|---|
| `codex.state_db` | session | `SUPPORTED_STATE_DB`: the highest `state_<n>.sqlite` is `5` | another number is the highest |
| `codex.threads_columns` | session | `THREADS_COLUMNS`: the `threads` table has these 43 columns, compared as a sorted `name:type:notnull:default` list and shown as a count plus the first 12 hex digits of its SHA-256 | a column is added, removed or changed (the observed value names them), or the table is unreadable |
| `codex.session_meta_keys` | session | `REQUIRED_META_KEYS`: the newest rollout's first line is `session_meta` with `cwd`, `id`, `source`, `timestamp` | the first line is another type or a key is missing |
| `codex.import_ledger_keys` | session | `LEDGER_RECORD_KEYS`: every `records[]` entry has exactly `connector_names`, `content_sha256`, `imported_at`, `imported_thread_id`, `source_modified_at`, `source_path`, `title` | a key is added or missing, or `records` is not an array |
| `codex.rollout_layout` | session | `ROLLOUT_LAYOUT`: `sessions/YYYY/MM/DD/rollout-YYYY-MM-DDTHH-MM-SS-<36 character id>.jsonl` | a sampled rollout path differs |
| `codex.skills_layout` | skill | `LAYOUT_EXPECTED`: the skills directory is absent, empty, or most of its directories hold a top-level `SKILL.md` | it is not a directory, or most directories lack `SKILL.md` |

Evidence (read-only statistics on the development machine, Codex 0.167.0): `state_5.sqlite` holds
the 43-column `threads` table; every one of the newest 40 rollouts starts with `session_meta` and the
four required keys (optional keys such as `git`, `thread_source` and `multi_agent_version` vary, so
they are not required); all 84 ledger records carry the 7 keys; all 212 rollout paths match the
layout.

## Facts and evidence

| Fact | Evidence level | Source |
|---|---|---|
| Rollout path `sessions/YYYY/MM/DD/rollout-<ts>-<thread id>.jsonl`, thread id UUIDv7 | OBSERVED | rollout corpus |
| `content_sha256` is the SHA-256 of the raw source bytes; `source_modified_at` is the mtime in ns | VERIFIED (76 of 78 existing sources) | importer ledger check; `record_for` in `codex_imports.rs` |
| Codex's import mapping reproduces user and assistant text one-for-one | VERIFIED (76 of 76 unchanged sources) | read-only converter check |
| Importer drops thinking, ignores `isMeta`, turns tool calls and results into assistant text | VERIFIED | same check |
| Listing needs a `threads` row with non-empty `preview` | INFERRED | 311 `session_index` lines vs 641 threads; `codex resume` not run |
| A rollout without a row is not listed | OBSERVED | 6 written rollouts, no rows, not in the picker |
| The written row has the importer's shape | VERIFIED against the fixture, OBSERVED against 73 imported rows | `crates/core/tests/codex_threads.rs` |
| `hooks.state` hashes pin hook commands | OBSERVED | `config.toml` structure and binary strings |
| The five probes pass on the fixtures; a `threads` column, a `state_6.sqlite`, an extra ledger key, a missing `session_meta` key and a flat rollout path each fail one check, and the database file is never written | VERIFIED | `crates/core/tests/probes.rs` (`codex_*`) |
| MCP `enabled` key and `type` handling | `[UNVERIFIED]` | not testable read-only |
| A nativemem-written rollout is read back as a replica | VERIFIED | test `claude_session_becomes_a_codex_rollout_with_an_import_record` |
| Codex lists a nativemem-written rollout plus row in `codex resume` and opens it | `[UNVERIFIED]` | needs the staged check |
| Codex accepts `history_mode` `legacy` for a row written this way | `[UNVERIFIED]` | needs the staged check |

## Source anchors

- `crates/core/src/adapters/codex.rs`
- `crates/core/src/adapters/codex_sessions.rs`
- `crates/core/src/adapters/codex_rollout.rs`
- `crates/core/src/adapters/codex_imports.rs`
- `crates/core/src/adapters/codex_probes.rs`
- `crates/core/src/adapters/codex_threads.rs`
- `crates/core/src/adapters/memory_store.rs`
- `crates/cli/src/main.rs` (`NATIVEMEM_CODEX_SESSION_WRITES`)
- `crates/core/src/testkit.rs` (`CodexHome`, `write_state_db`) and `testkit_codex_state.sql` (structure of the real `threads` table)
- `crates/core/tests/probes.rs`
- `crates/core/tests/codex_threads.rs`, `crates/cli/tests/codex_threads.rs`
- `docs/design.md` (Codex adapter, Chat sessions)

## Rules

- MUST: Write Codex's state database only through `codex_threads.rs` (rows of this tool's replicas, never a delete); never write its other SQLite files, `config.toml`, `hooks.json` or generated memory files. The probes open the state database read-only with `immutable=1`.
- MUST: Treat a rollout containing `<EXTERNAL SESSION IMPORTED>` as a replica, never an original.
- MUST: Keep session writes behind the `codex.state_db` and `codex.threads_columns` probes.
- MUST: Back up `external_agent_session_imports.json` before changing it; do not overwrite an
  unparsable one.
- MUST: Update the probe constants in `codex_probes.rs` and the table above together with the adapter.
- SHOULD: Re-read the header shape on a Codex upgrade before bumping `CLI_VERSION`.

## Verification checklist

- [ ] Without `NATIVEMEM_CODEX_SESSION_WRITES=on`, or with a failing schema probe, `sync` plans no session write for Codex
- [ ] The staged check (`codex resume` lists and opens a copy) is recorded here
- [ ] The README support table and this page agree on session writes

## Related docs

- `wiki/architecture/compatibility.md`
- `wiki/runbooks/agent-update-action-required.md`
- `wiki/kinds/sessions.md`
- `wiki/decisions/codex-session-writes-gated.md`
- `wiki/guides/verify-with-real-agents.md`
- `wiki/agents/claude-code.md`

## Last updated

2026-11-11
