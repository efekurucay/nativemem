# Decision: sessions are snapshots

## Purpose

Record why chat sessions follow a snapshot policy while memory and instruction items replicate
faithfully.

## Scope

`Kind::Session`. Mechanics are in `wiki/kinds/sessions.md`.

## Decision

A session replica is created once per (original, target agent) and refreshed only while it is
unmodified and the original has grown. A replica the user edits or continues is left alone from then
on, without a conflict note. A replica the user deletes gets a tombstone and is never recreated.
Edits and deletions never travel back to the original, a missing original never removes replicas, and
a replica is never replicated again. Session items are left out of the mass-delete total.

## Why

- Sessions are logs, not documents: continuing one in its own agent is normal use, not a conflict.
- Claude Code prunes transcripts by itself. If deletes propagated, pruning would erase history in
  every other agent.
- A user who deletes a copy wants it gone; recreating it on the next run would fight the user.
- Writing back would mean editing a transcript the original agent is still appending to.

## Consequences

- Replicas carry a marker (`nativemem_origin` key, `nativemem-origin` entry, import marker event) so a
  lost ledger cannot turn a copy into an original.
- Copies take the original's modification time and title so every resume list keeps the same order.
- CLI `delete` and `edit` refuse session ids with code `unsupported`.
- The ledger keeps the replica's own hash so a later edit stays visible; a hash left empty freezes a
  replica (tombstone, or a replica another program wrote).

## Source anchors

- `crates/core/src/engine.rs` (`plan_session`)
- `crates/core/src/session.rs`
- `crates/cli/src/main.rs` (`refuse_session`)
- tests `modified_replica_is_left_alone_without_a_note`, `deleted_replica_is_tombstoned_and_never_recreated`, `vanished_original_leaves_replicas_alone`

## Rules

- MUST: Never delete a session or propagate a session delete.
- MUST: Never write a replica's edits back to its original.
- MUST: Leave a modified replica alone.

## Verification checklist

- [ ] The three tests above pass
- [ ] `wiki/kinds/sessions.md` table matches `plan_session`

## Related docs

- `wiki/kinds/sessions.md`
- `wiki/agents/claude-code.md`
- `wiki/decisions/codex-session-writes-gated.md`

## Last updated

2026-11-10
