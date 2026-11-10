# Decision: Codex session writes are gated on thread rows

## Purpose

Record why session copies are not written into Codex by default.

## Scope

Session replicas whose target is Codex. Reading Codex rollouts as originals is unaffected.

## Decision

`CodexAdapter` answers `supports(Session, Project)` only when session writes are enabled, and the CLI
enables them only when the environment variable `NATIVEMEM_CODEX_SESSION_WRITES` equals `on`. With
the variable unset, sessions flow from Codex to the other agents, and no session is written into or
adopted for Codex.

## Why

- Codex lists threads from its state database: a thread appears with a rollout file plus a row in
  `threads` (INFERRED from the data; see `wiki/agents/codex.md`).
- `nativemem` writes only the rollout and the importer-ledger record. The database schema belongs to
  Codex, a live Codex app writes it in WAL mode, and editing it is outside the native-flow rule
  (`wiki/decisions/native-flow-only.md`).
- Whether Codex backfills a missing row from rollout files is `[UNVERIFIED]`. A copy that Codex does
  not list would be invisible clutter on disk.

## Consequences

- Tests and early adopters switch writes on explicitly: `run` in `crates/cli/tests/cli.rs` sets
  `NATIVEMEM_CODEX_SESSION_WRITES=on`.
- The gate is lifted by a verified thread-row writer behind a schema-version check, or by a staged
  check that shows Codex lists a rollout without a row (`wiki/guides/verify-with-real-agents.md`),
  and the change updates `README.md`, `wiki/kinds/sessions.md`, `wiki/agents/codex.md` and this page.
- Codex's own import of a Claude session is respected: with writes on, the importer ledger record is
  added so Codex does not import the session twice.

## Source anchors

- `crates/cli/src/main.rs` (`Agents::open`)
- `crates/core/src/adapters/codex.rs` (`with_session_writes`, `supports`)
- `crates/core/src/adapters/codex_rollout.rs`

## Rules

- MUST: Keep session writes into Codex off by default.
- MUST: Never write Codex SQLite files.
- SHOULD: Record the outcome of the staged check on `wiki/agents/codex.md` before changing the gate.

## Verification checklist

- [ ] With the variable unset, a dry-run plan holds no session write for Codex
- [ ] `README.md` names the variable and the gate

## Related docs

- `wiki/kinds/sessions.md`
- `wiki/agents/codex.md`
- `wiki/decisions/session-snapshot-policy.md`

## Last updated

2026-11-10
