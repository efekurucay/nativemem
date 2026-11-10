# Codex

## Purpose

What `nativemem` needs to know about the Codex CLI and desktop app to read its memory, instruction
sections and rollouts, and why session writes into Codex are gated.

## Scope

Facts gathered read-only on a development machine running codex-cli 0.167.0 (rollouts from 0.149.0
to 0.167.0; 215 live rollouts, 5 archived, 641 rows in the `threads` table, 79 sessions imported from
Claude Code). SQLite files were inspected as copies. The `codex` binary is never driven by this repo.

## Status in nativemem

| Kind | Status | Adapter |
|---|---|---|
| Project memory | Originals read from generated `memories/MEMORY.md` (read-only, never edited or deleted); replicas written to a nativemem-owned store plus a routing block | `crates/core/src/adapters/codex.rs`, `memory_store.rs` |
| Instruction sections | Read and write in `AGENTS.md` | `codex.rs` through `sections.rs` |
| Sessions | Originals always read; replica writes only when `NATIVEMEM_CODEX_SESSION_WRITES=on` | `codex_sessions.rs`, `codex_rollout.rs`, `codex_imports.rs` |
| MCP servers, skills | Not implemented | |

## Native locations

| What | Global | Project |
|---|---|---|
| Instructions | `<codex home>/AGENTS.md` | `<repo>/AGENTS.md` (never touched) |
| Memory | generated `<codex home>/memories/MEMORY.md` and `memory_summary.md`; replica store `<codex home>/memories/extensions/nativemem/resources/<project key>/` | blocks inside the generated file carry `applies_to: cwd=<path>` |
| Sessions | `<codex home>/sessions/YYYY/MM/DD/rollout-<ts>-<thread id>.jsonl`, `archived_sessions/` | |
| Session catalogue | `<codex home>/state_5.sqlite` table `threads` | |
| Import ledger | `<codex home>/external_agent_session_imports.json` | |
| MCP servers | `<codex home>/config.toml` tables `[mcp_servers.<name>]` | |
| Skills | `<codex home>/skills/<name>/`, `~/.agents/skills`, plugin caches | `<repo>/.agents/skills` |

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
  `bearer_token_env_var`, per-tool approval settings (not implemented here).

## What nativemem reads and writes today

- Reads `memories/MEMORY.md` blocks, `AGENTS.md` sections, `memories/extensions/nativemem/resources/`
  replicas and `sessions/` rollouts (directory depth at most 4 below `sessions/`).
- Writes replica memory items and the routing block, the instruction region at the end of
  `AGENTS.md`, and, only with `NATIVEMEM_CODEX_SESSION_WRITES=on`, rollout files and the importer-ledger
  record for Claude-origin sessions. The ledger file is parsed and rewritten whole (every other key
  and record kept, key order not), backed up first, created only when the Codex home exists, and left
  alone when it does not parse.
- A rollout written by this tool is shaped like Codex's own importer output: `session_meta`
  (`originator` `Codex Desktop` for a Claude origin, the origin agent's display name otherwise;
  `cli_version` and `source` as the importer writes them), one `external-import-turn-N` turn per user
  message with event and `response_item` pairs, the `<EXTERNAL SESSION IMPORTED>` marker event once,
  a `token_count` and the closing `task_complete`. `session_meta` also carries `nativemem_origin` and
  `nativemem_title`. The file mtime is the original's.
- Never touches `state_5.sqlite`, `~/.codex/sqlite/`, `thread_history_1.sqlite`, `session_index.jsonl`,
  `history.jsonl`, `.codex-global-state.json`, `config.toml` or `hooks.json`.

## Sessions: listing and resume

- Codex's list is driven by the `threads` table. A thread appears with a rollout file plus a row that
  has a non-empty `preview`, `archived = 0` and a valid absolute `rollout_path` (INFERRED from the
  data; `codex resume` was not run). Other places that hold an imported thread are caches or UI state
  (`session_index.jsonl`, `history.jsonl`, `thread_history_1.sqlite`, `sqlite/codex.db`,
  `.codex-global-state.json`); imports were present in `threads` and absent from several of them.
- A session imported by Codex itself has a row inserted by the importer directly (`originator` NULL),
  `cwd` equal to the first cwd in the Claude file, `title` from the Claude title priority, and
  `created_at` / `updated_at` equal to the first and last message times of the source.
- `nativemem` writes no row. Whether Codex backfills a missing row from rollout files is
  `[UNVERIFIED]`. This is why session writes into Codex are off by default
  (`wiki/decisions/codex-session-writes-gated.md`).

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
- **A live Codex app writes `state_5.sqlite` in WAL mode** and holds lock files. Writing the database
  while the app runs risks corruption; any future row writer must coordinate with Codex.
- **Context cap**: Codex loads project docs up to `project_doc_max_bytes = 32768` (binary strings,
  OBSERVED), which matters for the routing block size.

## Version detection

`nativemem` does not detect the Codex version. The rollout writer pins `CLI_VERSION`
(`0.167.0-alpha.3.2`) and a short base-instructions text in
`crates/core/src/adapters/codex_rollout.rs`. A rollout records the writing version in
`session_meta.cli_version`; `~/.codex/version.json` holds `latest_version`; the installed version
comes from the agent's own `--version`, run by the engineer, never by tests.

## Facts and evidence

| Fact | Evidence level | Source |
|---|---|---|
| Rollout path `sessions/YYYY/MM/DD/rollout-<ts>-<thread id>.jsonl`, thread id UUIDv7 | OBSERVED | rollout corpus |
| `content_sha256` is the SHA-256 of the raw source bytes; `source_modified_at` is the mtime in ns | VERIFIED (76 of 78 existing sources) | importer ledger check; `record_for` in `codex_imports.rs` |
| Codex's import mapping reproduces user and assistant text one-for-one | VERIFIED (76 of 76 unchanged sources) | read-only converter check |
| Importer drops thinking, ignores `isMeta`, turns tool calls and results into assistant text | VERIFIED | same check |
| Listing needs a `threads` row with non-empty `preview` | INFERRED | 311 `session_index` lines vs 641 threads; `codex resume` not run |
| Rollout alone is listed after a backfill | `[UNVERIFIED]` | `backfill_state` tables exist; behaviour not tested |
| `hooks.state` hashes pin hook commands | OBSERVED | `config.toml` structure and binary strings |
| MCP `enabled` key and `type` handling | `[UNVERIFIED]` | not testable read-only |
| A nativemem-written rollout is read back as a replica | VERIFIED | test `claude_session_becomes_a_codex_rollout_with_an_import_record` |
| Codex lists a nativemem-written rollout in its resume list | `[UNVERIFIED]` | needs the staged check |

## Source anchors

- `crates/core/src/adapters/codex.rs`
- `crates/core/src/adapters/codex_sessions.rs`
- `crates/core/src/adapters/codex_rollout.rs`
- `crates/core/src/adapters/codex_imports.rs`
- `crates/core/src/adapters/memory_store.rs`
- `crates/cli/src/main.rs` (`NATIVEMEM_CODEX_SESSION_WRITES`)
- `crates/core/src/testkit.rs` (`CodexHome`)
- `docs/design.md` (Codex adapter, Chat sessions)

## Rules

- MUST: Never write Codex's SQLite files, `config.toml`, `hooks.json` or generated memory files.
- MUST: Treat a rollout containing `<EXTERNAL SESSION IMPORTED>` as a replica, never an original.
- MUST: Keep session writes gated until the thread-row writer exists and is verified.
- MUST: Back up `external_agent_session_imports.json` before changing it; do not overwrite an
  unparsable one.
- SHOULD: Re-read the header shape on a Codex upgrade before bumping `CLI_VERSION`.

## Verification checklist

- [ ] With `NATIVEMEM_CODEX_SESSION_WRITES` unset, `sync` plans no session write for Codex
- [ ] The staged check (resume list, open a session) is recorded here before the gate is lifted
- [ ] The README support table and this page agree on session writes

## Related docs

- `wiki/kinds/sessions.md`
- `wiki/decisions/codex-session-writes-gated.md`
- `wiki/guides/verify-with-real-agents.md`
- `wiki/agents/claude-code.md`

## Last updated

2026-11-10
