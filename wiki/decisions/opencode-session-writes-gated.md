# Decision: OpenCode session writes are opt-in, probe-guarded and marked inside the message JSON

## Purpose

Record when session copies are written into OpenCode's database and why they look the way they do.

## Scope

Session replicas whose target is OpenCode. Reading OpenCode sessions as originals is unaffected by the switch.

## Decision

OpenCode session writes are opt-in: the CLI enables them only when `NATIVEMEM_OPENCODE_SESSION_WRITES=on`,
because the user decides when nativemem may touch OpenCode's database. The agent takes part in session
sync only when `<data home>/opencode.db` exists (`--opencode-data-home`, `NATIVEMEM_OPENCODE_DATA_HOME`,
else `$XDG_DATA_HOME/opencode`, else `~/.local/share/opencode`). While the switch is off
`AgentsFileAdapter` answers `supports(Session, Project)` false, nothing is planned for OpenCode as a
target, sessions are still read as originals, and `status` and `doctor` show the notice `session writes
are off; set NATIVEMEM_OPENCODE_SESSION_WRITES=on to let nativemem write sessions into OpenCode`. With the
switch on, `supports` is true unless the compatibility watch pauses sessions: `opencode.session_schema`
(column signature of `session_v2`, `session_message`, `project`, `event_sequence`) or `opencode.version`
(major `2`) failing pauses OpenCode sessions, so nothing is read or written.

A replica is one `session_v2` row plus its `session_message` rows and `event_sequence` row, written in one
`BEGIN IMMEDIATE` transaction after the run's online backup (`wiki/architecture/safety.md`). The values:

| Row | Value and source |
|---|---|
| `session_v2.id` | `ses_` + 12 hex (complement of `(created ms << 12) mod 2^48`) + 14 base62 derived from the marker text, so a refresh finds the same row (OBSERVED id shape) |
| `project_id` | `opencode_project::resolve(cwd)`: the id cached in `<git common dir>/opencode`, else SHA-1 of `git-remote:<normalised origin>`, else the first root commit; no VCS marker or a missing directory: SHA-1 of `directory:<cwd>`; Mercurial and an underivable git id are skipped with a note (VERIFIED against the local rows; the cache file wins over the origin) |
| `directory`, `path` | the cwd; `''` at the project root, else the relative sub path (OBSERVED) |
| `title`, `time_created`, `time_updated` | the neutral session's title and times in ms; `time_updated` equals the original's so ordering matches the other agents |
| `version` | the installed OpenCode version from the install path, else `2.2.6` |
| `agent`, `model` | `build` (OpenCode's default agent), model NULL so OpenCode uses its own default |
| `time_idle`, `time_busy`, `idle_outcome` | updated, created, `succeeded`: the state of a finished session; `parent_id`, `time_archived`, `time_suspended`, `time_viewed` NULL |
| `slug` | two words derived from the marker (OBSERVED `adjective-noun`) |
| `session_message` | contiguous `seq` from 0: a `user` row per user turn, an `assistant` row per assistant turn with one text block, `agent` `build`, `finish` `stop`, zero cost and tokens, and the provider and model id of the most recently updated session that names one (the fallback `opencode/big-pickle` only for a database without any), an `idle` row after each assistant run and at the end; ids `msg_` + 12 hex of `(ms << 12) + index` + 14 base62 |
| `event_sequence.seq` | the last `seq` in use (OBSERVED: sessions OpenCode migrated have `seq` equal to their last message `seq`); only raised on a refresh |
| `project` | inserted with `sandboxes` `[]` and the times of the write only when the id has no row |

The marker is the extra key `nativemem_origin` in every row's `data` JSON, so a lost ledger still
recognises a replica and the key stays out of the interface. Tool activity is readable text, as for
every writer.

## Why

- OpenCode lists sessions from `session_v2` filtered by exact `directory` (`opencode -c`) or project and
  `path` (picker), newest `time_updated` first; the rows above are what those queries need
  (INFERRED from binary strings, `[UNVERIFIED]` until the staged check).
- The schema moves fast, so the writer refuses to run when a column differs and takes a backup before the
  first write.
- A running OpenCode may hold the database in WAL mode: the writer waits at most 5 seconds, never changes
  the journal mode and leaves a busy database for the next run.
- Replicas are rows, not files: a refresh rewrites the replica's own rows (surplus rows turn into `idle`
  rows) because rows are never deleted.

## Consequences

- OpenCode keeps no retention of aged sessions that nativemem could trip over: no cleanup of aged
  `session_v2` rows was found in the binary strings (INFERRED); archived sessions are not read.
- A deleted replica is not observed, its ledger row stays and it is never written again.
- Whether the running OpenCode service caches session lists and needs a restart to show new copies is
  `[UNVERIFIED]`; nativemem never runs or signals it.
- That `-c` and the picker list and open a copy, that the extra JSON key is ignored, and that the model
  field raises no restore warning are `[UNVERIFIED]` until `wiki/guides/verify-with-real-agents.md` is done.
- Linked git worktrees use the main repository's project id and a `path` relative to the worktree
  (`[UNVERIFIED]`).

## Source anchors

- `crates/cli/src/main.rs` (`Agents::open`, `resolve`)
- `crates/core/src/adapters/agents_file.rs` (`supports`, `write_replica`)
- `crates/core/src/adapters/opencode_sessions.rs`
- `crates/core/src/adapters/opencode_project.rs`
- `crates/core/src/adapters/opencode_probes.rs`
- `crates/core/src/adapters/sqlite_guard.rs`

## Rules

- MUST: Write OpenCode's database only through `opencode_sessions.rs`, only while the probes pass.
- MUST: Never change or delete a row OpenCode created.
- MUST: Bump `SESSION_WRITER_REVISION` in `opencode_sessions.rs` when the written rows change.
- SHOULD: Record the outcome of the staged check on `wiki/agents/opencode.md`.

## Verification checklist

- [ ] With a failing `opencode.session_schema`, `sync --apply` reads and writes no OpenCode session
- [ ] Without `NATIVEMEM_OPENCODE_SESSION_WRITES=on`, `opencode.db` is unchanged and no backup exists
- [ ] `README.md` names the switch and the guard

## Related docs

- `wiki/kinds/sessions.md`
- `wiki/agents/opencode.md`
- `wiki/architecture/compatibility.md`
- `wiki/decisions/codex-session-writes-gated.md`
- `wiki/decisions/session-snapshot-policy.md`

## Last updated

2026-11-11
