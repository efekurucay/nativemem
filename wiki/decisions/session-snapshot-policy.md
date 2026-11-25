# Decision: session continuation and native append

## Purpose

Record why chat sessions follow a snapshot policy while memory and instruction items replicate
faithfully.

## Scope

`Kind::Session`. Mechanics are in `wiki/kinds/sessions.md`.

## Decision

A session replica is created once per (original, target agent). A tracked replica can relay new
turns when its recorded history is an exact prefix and its turns are compatible with the original.
Untouched copies follow the longest compatible history. When two continuations diverge, sync
reports a conflict and preserves both. Codex and OpenCode append compatible turns to an idle
original or replica. Pi and Claude Code append compatible messages to the current native leaf.
All four retain the same native id.
Deleting a copy records a tombstone. A missing original leaves existing copies in place.
Session items are left out of the mass-delete total.

## Why

- Appending to the same transcript is normal resume behavior. Divergent turns represent distinct
  conversations and cannot be merged without changing what the user and each agent saw.
- Claude Code prunes transcripts by itself. If deletes propagated, pruning would erase history in
  every other agent.
- A user who deletes a copy wants it gone; recreating it on the next run would fight the user.
- OpenCode checks the history and idle state inside the SQLite transaction. Codex checks the
  stored history before and after its native append API call. Pi and Claude Code compare the
  existing bytes before one append and verify the resulting file. Close file-backed sessions
  before handing off.

## Consequences

- Replicas carry a marker (`nativemem_origin` key, `nativemem-origin` entry, import marker event) so a
  lost ledger cannot turn a copy into an original.
- Copies keep the original's title and use the modification time of the history they contain.
- CLI `delete` and `edit` refuse session ids with code `unsupported`.
- The ledger keeps the replica's own hash so a later edit stays visible; a hash left empty freezes a
  replica (tombstone, or a replica another program wrote).

## Source anchors

- `crates/core/src/engine.rs` (`plan_session`)
- `crates/core/src/session.rs`
- `crates/cli/src/main.rs` (`refuse_session`)
- Tests `a_continued_codex_copy_reaches_pi_without_rewriting_the_sources`,
  `independently_continued_histories_are_preserved_and_reported`,
  `deleted_replica_is_tombstoned_and_never_recreated` and `vanished_original_leaves_replicas_alone`

## Rules

- MUST: Never delete a session or propagate a session delete.
- MUST: Preserve existing turns and use native append only with an exact compatible prefix.
- MUST: Recheck the OpenCode session for native changes after taking the write transaction.

## Verification checklist

- [ ] The session tests above pass
- [ ] `wiki/kinds/sessions.md` table matches `plan_session`

## Related docs

- `wiki/kinds/sessions.md`
- `wiki/agents/claude-code.md`
- `wiki/decisions/codex-session-writes-gated.md`

## Last updated

2026-11-22
