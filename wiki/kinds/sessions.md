# Chat sessions

## Purpose

How chat sessions are read, converted and written between Claude Code, Codex and Pi, and why they
follow a snapshot policy instead of the replication policy of memory.

## Scope

`Kind::Session` with `Scope::Project(<cwd>)`. OpenCode and Hermes have no session adapter. Policy
rationale: `wiki/decisions/session-snapshot-policy.md`. Agent-side listing and resume behaviour:
`wiki/agents/`.

## Neutral model

`Session` in `crates/core/src/session.rs`: `id`, `cwd`, `title`, `created_ms`, `updated_ms` and an
ordered list of turns (`user` or `assistant`, text, timestamp).

- Thinking blocks and provider signatures are dropped; they are opaque and bound to a provider.
- Images become the text `[external unsupported block: image]`.
- Empty text never makes a turn. A session needs at least one user turn.
- Titles are single-line and at most 200 characters; without a title the text is `Untitled session`.
- Engine item: `kind` session, `scope` `Project(cwd)`, `name` the session's own id, `description` the
  title, `body` the deterministic JSON of the neutral model. `native_id` is the path below the
  agent's session root and ends in `.jsonl` (`is_session_id` classifies by that suffix).
- The content hash covers cwd, title and each turn's role and text; ids and timestamps are left out,
  so a replica hashes like its original.

## Tool-activity rendering

Tool activity becomes one assistant text turn per call, call and result together, so every source
renders the same way in every target (test `tool_activity_renders_identically_in_every_target`):

```
Tool: <name>
<fenced block: the command for shell-like tools, compact JSON otherwise; cut at 600 characters>
Result:                       (or "Result (exit code N):" for a non-zero exit, "Result (error):" for a tool-reported error)
<fenced block: the output, cut at 1500 characters>
… (N more characters)         (only when something was cut)
```

An empty result prints `Result: (no output)`. The fence grows when the text contains backticks. A
result is merged into the turn of its call by call id; a result without a call stands alone. Codex's
output wrapper (`{"output": ..., "exit_code": ...}`) and JS-style calls such as
`text(await tools.exec_command({cmd:"..."}))` are unwrapped to the command and the output. Pi
`bashExecution` messages render the same way. Anchors: `render_tool`, `TOOL_INPUT_LIMIT`,
`TOOL_RESULT_LIMIT` in `crates/core/src/session.rs`; tests `merged_tool_turn_reads_like_a_conversation`,
`long_input_and_output_are_cut_with_a_notice`, `js_style_call_and_json_wrapper_are_unwrapped`.

## Snapshot policy

Implemented in `plan_session` in `crates/core/src/engine.rs`.

| Situation | Action | Test |
|---|---|---|
| No replica in a target agent that supports sessions | Write one, once per (original, target agent) | `claude_session_reaches_pi_and_round_trips` |
| Original grew, replica unmodified (its hash equals the ledger's) | Rewrite the replica, backing up the file | `grown_original_refreshes_an_unmodified_replica_with_backup` |
| Replica edited or continued in its own agent | Left alone from then on, no conflict note | `modified_replica_is_left_alone_without_a_note` |
| Target agent does not accept the session (`Adapter::accepts`; Claude Code when the session is older than its retention minus a day) | Nothing: no write, no note, no tombstone, and a missing copy is not a deletion | `a_session_older_than_the_retention_is_planned_for_other_agents_but_not_claude`, `a_replica_pruned_by_the_agent_is_not_reported_as_deleted_but_a_young_one_is` in `crates/cli/tests/doctor.rs` |
| Replica deleted by the user after it was written | Tombstone: a replica ledger row with an empty hash; never recreated | `deleted_replica_is_tombstoned_and_never_recreated` |
| Replica file exists but was not scanned (outside the window, too large) | Not treated as deleted | `crates/core/src/engine.rs` (`plan_session`) |
| Original missing (the agent pruned it) | Replicas stay | `vanished_original_leaves_replicas_alone` |
| Replica is itself a candidate original | Never; replicas are never replicated again | `replicas_are_never_originals_for_a_third_agent` |
| Ledger lost, marked replica found | Adopt it into the ledger, write nothing | `lost_ledger_adopts_replicas_and_never_loops` |
| Writer revision of the adapter is higher than the replica's | Rewrite once while unmodified | `unmodified_replica_with_an_older_writer_revision_is_rewritten_once` |

Edits never flow back to the original. Session items are left out of the mass-delete total. The
reason deletes never propagate: Claude Code prunes transcripts by itself, and that must not
erase history in other agents. Session ids are refused by CLI `delete` and `edit` with code
`unsupported`.

## Markers

A marker survives a lost ledger, so a replica is never mistaken for an original.

| Agent | Marker | Anchor |
|---|---|---|
| Claude Code | Extra top-level key `nativemem_origin: "<agent>:<native id>"` on every line the tool writes. Claude reads lines by known keys, so the key is ignored and the file stays a valid transcript; a continued replica keeps it because Claude only appends | `MARKER_KEY` in `crates/core/src/adapters/claude_sessions.rs` |
| Pi | A `custom` entry with `customType: "nativemem-origin"` and `data.origin`. Pi keeps custom entries out of the model context | `MARKER_TYPE` in `crates/core/src/adapters/pi.rs` |
| Codex | `nativemem_origin` and `nativemem_title` in `session_meta`, plus Codex's own import marker event `<EXTERNAL SESSION IMPORTED>`. Any rollout containing that event is a replica whichever program wrote it | `ORIGIN_KEY`, `TITLE_KEY` in `crates/core/src/adapters/codex_rollout.rs`, `IMPORT_MARKER` in `crates/core/src/adapters/codex_sessions.rs` |

A rollout Codex imported itself maps to its Claude original through `external_agent_session_imports.json`
(`source_path` names the transcript); a marked rollout without a record is ignored (test
`imported_codex_sessions_are_not_originals`). When Claude has a record in that ledger, the engine
adopts the imported thread as the Codex replica with an empty hash, so it is never rewritten (test
`a_session_imported_by_codex_is_adopted_not_written_again`).

## Writers by agent

- **Claude Code**: `projects/<dir>/<session id>.jsonl`. Reading takes `user` and `assistant` lines in
  file order, drops `isMeta` and `isSidechain` lines, one turn per text block. Title is the last
  `custom-title`, else the last `ai-title`, else the last `last-prompt`, else the first user text.
  The cwd is the first `cwd` on any line. A replica goes to the project directory whose cwd matches
  (created with Claude's naming rule when absent) as `<uuid>.jsonl` with the uuid derived from the
  origin, so a refresh rewrites the same file: a `custom-title` line, one `user` or `assistant` line
  per turn chained by `parentUuid` with the minimal required fields, and a `last-prompt` line. The
  file's mtime is set to the original's. Claude deletes transcripts older than `cleanupPeriodDays` by
  mtime, so a session older than that retention minus a day is not written to Claude
  (`wiki/agents/claude-code.md`). Anchor: `crates/core/src/adapters/claude_sessions.rs`.
- **Pi**: `sessions/--<cwd>--/<ISO ts>_<uuid>.jsonl`, header version 3. Reading follows the path from
  the last entry to the root. A replica has the header, the marker entry, a `session_info` entry and
  one linear `message` entry per turn. Assistant messages carry `api: "pi-virtual"` and the replica
  has no `model_change` entry, so Pi opens it with its own default model and no model warning (test
  `written_pi_replica_names_no_placeholder_model`). Anchor: `crates/core/src/adapters/pi.rs`.
- **Codex**: originals are read from `sessions/YYYY/MM/DD/rollout-*.jsonl`: text turns from the event
  stream (which never carries the context Codex injects as user messages), tool calls and results from
  `function_call` and `custom_tool_call` items; the title is the first user text; the cwd comes from
  `session_meta`. A replica is a rollout `sessions/YYYY/MM/DD/rollout-<UTC creation time>-<thread id>.jsonl`
  shaped like Codex's own importer output (`wiki/agents/codex.md`). The thread id is a UUIDv7 of the
  creation time with the rest derived from the origin, so a refresh rewrites the same file. For a
  Claude origin the matching record is also added to `external_agent_session_imports.json` so Codex's
  importer does not import the session again. Anchors: `crates/core/src/adapters/codex_sessions.rs`,
  `codex_rollout.rs`, `codex_imports.rs`.

## Codex needs a thread row; writes are opt-in and follow the schema probes

Codex lists threads from its state database, so each replica rollout also gets one `threads` row
(`crates/core/src/adapters/codex_threads.rs`, called from `Adapter::finish` after the run's writes).
The row is derived from the rollout file: id, absolute rollout path, cwd, title, first user message,
first and last message time, and the importer's constants for the rest (`wiki/agents/codex.md`).
Rows are inserted or refreshed only for rollouts carrying `nativemem_origin`, a rollout without a row
gets it on the next run, and Codex's own rows are never touched or deleted. The writer runs only when
`codex.state_db` and `codex.threads_columns` pass; a failing check pauses sessions for Codex (neither
read nor written). Writes into Codex are opt-in (`NATIVEMEM_CODEX_SESSION_WRITES=on`; `status` and `doctor` show a notice while
they are off, and nothing is planned for Codex then); sessions always flow from Codex to Claude Code
and Pi. `--no-sessions` writes no row either. A locked database is skipped
with a `skipped` note and repaired on the next run. Decision:
`wiki/decisions/codex-session-writes-gated.md`.

## Scope flags

Every session of every age and kind is synced by default, so each agent's resume list shows the same
sessions in the same order (a replica takes its original's modification time and title). Limits are
opt-in:

| Flag | Effect |
|---|---|
| `--session-days <N>` (`NATIVEMEM_SESSION_DAYS`) | Only sessions whose file was modified in the last N days; checked on the file's mtime before anything is parsed |
| `--exclude-automated-sessions` | Leave out headless runs and sub-agent threads |
| `--no-sessions` | Session sync off |

Sources over 200 MB and sessions without a resolvable working directory are skipped and listed by
`status`, which also counts sessions and automated sessions per agent (`sessions`,
`automated_sessions` in `--json`). Anchors: `SessionOptions` in `crates/core/src/session.rs`; tests
`window_is_opt_in_and_limits_which_sessions_are_read`, `sessions_without_user_turns_or_cwd_are_not_synced`.

## Automated-session markers

Structural markers only; a missing or unknown marker counts as interactive.

| Agent | Automated when |
|---|---|
| Claude Code | The first `user` or `assistant` line's `entrypoint` starts with `sdk` (`sdk-cli`, `sdk-ts`) |
| Codex | `session_meta.source` is an object (sub-agent), `parent_thread_id` is set, `thread_source` is `subagent`, `source` is `exec`, or `originator` is `codex_exec` or `Claude Code` |
| Pi | No marker exists; every Pi session counts as interactive |

Test: `automated_claude_and_codex_sessions_are_counted_and_optionally_excluded`.

## Source anchors

- `crates/core/src/session.rs`
- `crates/core/src/engine.rs` (`plan_session`)
- `crates/core/src/adapters/claude_sessions.rs`
- `crates/core/src/adapters/codex_sessions.rs`
- `crates/core/src/adapters/codex_rollout.rs`
- `crates/core/src/adapters/codex_imports.rs`
- `crates/core/src/adapters/codex_threads.rs`
- `crates/core/src/adapters/pi.rs`
- `crates/cli/src/main.rs`
- `docs/design.md` (Chat sessions)

## Rules

- MUST: Sessions never delete, never flow back, and never make a replica an original.
- MUST: Every written session carries a marker that survives a lost ledger.
- MUST: Bump the adapter's `SESSION_WRITER_REVISION` when the written format changes.
- MUST: Render tool activity through `render_tool` so all targets read the same.
- MUST: Write Codex's SQLite database only through `codex_threads.rs`, only while the schema probes pass.
- MUST: Plan nothing for a session the target agent does not accept (`Adapter::accepts`).
- SHOULD: Set the replica's file mtime to the original's so resume lists keep the relative order.

## Verification checklist

- [ ] A new session writer round-trips through its own reader with the marker
- [ ] A refresh rewrites the same file (derived id), not a second one
- [ ] The resume list of the real agent shows the copy (`wiki/guides/verify-with-real-agents.md`)
- [ ] The tool rendering test still passes for every source and target

## Related docs

- `wiki/decisions/session-snapshot-policy.md`
- `wiki/decisions/codex-session-writes-gated.md`
- `wiki/agents/codex.md`
- `wiki/agents/pi.md`
- `wiki/agents/claude-code.md`

## Last updated

2026-11-11
