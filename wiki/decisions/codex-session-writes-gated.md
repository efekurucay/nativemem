# Decision: Codex session writes are guarded by the schema probes

## Purpose

Record when session copies are written into Codex and why a thread row is part of every write.

## Scope

Session replicas whose target is Codex. Reading Codex rollouts as originals is unaffected.

## Decision

Codex session writes (rollout files, `threads` rows and importer-ledger records) are opt-in: the CLI
enables them only when `NATIVEMEM_CODEX_SESSION_WRITES=on`, because the user decides when nativemem
may touch Codex's data (the desktop app's bundled CLI would otherwise run the writer from its Sync
button). While the switch is off `CodexAdapter` answers `supports(Session, Project)` false, nothing is
planned for Codex sessions and `status` and `doctor` show the notice `session writes are off; set
NATIVEMEM_CODEX_SESSION_WRITES=on to let nativemem write sessions into Codex`. With the switch on,
`supports` is true unless the compatibility watch pauses sessions for Codex. The watch pauses them when the
checks `codex.state_db` or `codex.threads_columns` (or any other Codex session check) fail, so
nothing is read or written for Codex sessions while the database does not have the shape the row
writer was written against. Every replica rollout gets a `threads` row in `state_<n>.sqlite`
(`crates/core/src/adapters/codex_threads.rs`); the row is the only thing besides the rollout file
that Codex's resume list needs.

## Why

- Codex lists threads from its state database: a thread appears with a rollout file plus a row in
  `threads` (INFERRED from the data; see `wiki/agents/codex.md`). A rollout alone would be invisible
  clutter.
- The database schema belongs to Codex, so the writer refuses to run when the column signature
  differs and takes a backup with SQLite's online backup API before the first write of a run.
- A live Codex app holds the database in WAL mode, so the writer waits at most 5 seconds for a lock,
  never changes the journal mode and leaves a busy database for the next run.

## Consequences

- No row is deleted, and a row Codex or the user changed keeps those columns.
- Reading Codex sessions as originals, adopting threads Codex imported, and memory and instruction
  sync are unaffected by the switch.
- A rollout that lost its row (skipped run, busy database) gets it on the next run.
- Codex's own import of a Claude session is respected: the importer-ledger record is added and a
  thread Codex imported is adopted, so a thread id appears once in the database.
- That Codex lists and opens a written copy is `[UNVERIFIED]` until the staged check in
  `wiki/guides/verify-with-real-agents.md` is done.

## Source anchors

- `crates/cli/src/main.rs` (`Agents::open`)
- `crates/core/src/adapters/codex.rs` (`supports`, `finish`)
- `crates/core/src/adapters/codex_threads.rs`
- `crates/core/src/adapters/codex_probes.rs`

## Rules

- MUST: Write Codex's database only through `codex_threads.rs`, only while the schema probes pass.
- MUST: Never delete a row and never change a row of a thread this tool did not write.
- SHOULD: Record the outcome of the staged check on `wiki/agents/codex.md`.

## Verification checklist

- [ ] With a failing `codex.threads_columns`, `sync --apply` writes no Codex rollout and no row
- [ ] Without `NATIVEMEM_CODEX_SESSION_WRITES=on`, no Codex session is written, planned or noted as skipped
- [ ] `README.md` names the switch and the guard

## Related docs

- `wiki/kinds/sessions.md`
- `wiki/agents/codex.md`
- `wiki/architecture/compatibility.md`
- `wiki/decisions/session-snapshot-policy.md`

## Last updated

2026-11-11
