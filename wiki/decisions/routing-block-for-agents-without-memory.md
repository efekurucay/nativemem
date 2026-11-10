# Decision: routing block for agents without native project memory

## Purpose

Record how agents that have no per-project memory of their own still receive memory, without a
plugin, a config edit or a file inside a repository.

## Scope

Codex, OpenCode and Pi replicas of project memory. Claude Code has native per-project memory and
needs no block.

## Decision

Replica items go to a store inside the agent's own home (item files, a per-project `MEMORY.md` index
and a `scope.json`). A block in the agent's global `AGENTS.md`, between `<!-- nativemem:begin -->` and
`<!-- nativemem:end -->`, lists `<cwd> → <index file>` per project and tells the agent to read the
matching project's index first and open the item files it lists when relevant. The block is
regenerated on every apply and removed when no project remains. Content outside the markers is never
changed.

## Why

- These agents always load their global `AGENTS.md`, so an instruction there is deterministic, needs
  no configuration and works on every project.
- They can read absolute paths without approval (Pi's `read` tool; OpenCode's default permission for
  files outside the project except `.env` patterns), so the index files can sit in the agent home
  (`wiki/agents/pi.md`, `wiki/agents/opencode.md`).
- Nothing is written inside a repository, and no per-project `AGENTS.md` is created.
- The index is one line per item, so the agent reads little by default and opens items on demand.

## Consequences

- The block grows by one line per project in every session; projects with no items are dropped.
- The store layout is owned by one module (`crates/core/src/adapters/memory_store.rs`).
- Routing lines use absolute paths.
- Codex's loader caps project docs at 32768 bytes (`wiki/agents/codex.md`), which bounds the block.

## Source anchors

- `crates/core/src/adapters/memory_store.rs` (`routing_block`, `splice_block`)
- `crates/core/src/adapters/agents_file.rs`
- tests `routing_block_insert_update_remove_preserves_surroundings`, `routing_block_created_when_agents_md_absent`

## Rules

- MUST: Edit the routing block only between its markers.
- MUST: Remove the block when the last project goes.
- MUST: Keep the block before any instruction region in the file.

## Verification checklist

- [ ] The two tests above pass
- [ ] A synced project's agent reads the named index (`wiki/guides/verify-with-real-agents.md`)

## Related docs

- `wiki/kinds/memory.md`
- `wiki/decisions/native-flow-only.md`

## Last updated

2026-11-10
