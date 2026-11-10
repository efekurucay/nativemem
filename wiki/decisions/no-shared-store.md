# Decision: no shared store

## Purpose

Record why `nativemem` has no memory store of its own that agents read from.

## Scope

All resource kinds and all agents.

## Decision

Every agent keeps reading its own files, in its own format, from its own home directory.
`nativemem` is a translator that runs between them; the only state it owns is the ledger and the
backups in the state directory.

## Why

- A shared store would need every agent to be taught to read it, which means plugins, extensions or
  config edits (`wiki/decisions/native-flow-only.md`).
- Agents keep working when `nativemem` is absent or broken: their files are the source of truth, so
  removing the tool loses nothing.
- The ledger can stay small and safe: ids, hashes and paths, never entry text, so it holds no secrets
  and cannot drift into a second copy of the user's data.
- Text is copied verbatim and scope is preserved, which a neutral store would tempt a rewrite of.

## Consequences

- Each agent needs an adapter that reads and writes its native format.
- Agents without a native memory (OpenCode, Pi) receive replicas in a store inside their own home,
  announced by a routing block (`wiki/decisions/routing-block-for-agents-without-memory.md`).
- The ledger can be lost without losing data: marked replicas are adopted on the next run (test
  `lost_ledger_is_rebuilt_by_adoption`).

## Source anchors

- `README.md` (intro, How it works)
- `docs/design.md` (Intent, Anti-goals)
- `crates/core/src/ledger.rs`

## Rules

- MUST: Keep entry text out of the ledger.
- MUST: Leave every agent able to work with `nativemem` uninstalled.
- SHOULD: Express any new need as an adapter, not as a store.

## Verification checklist

- [ ] The ledger schema holds no text column
- [ ] No adapter requires the user to configure the agent to find synced data, beyond what the agent loads natively

## Related docs

- `wiki/decisions/native-flow-only.md`
- `wiki/architecture/overview.md`

## Last updated

2026-11-10
