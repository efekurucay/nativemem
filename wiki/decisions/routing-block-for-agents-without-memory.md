# Decision: project memory is independent of global instructions

## Purpose

Keep global instructions focused on user-authored rules and instruction replicas.

## Scope

Codex, OpenCode and Pi. Claude Code loads its native project memory directly.

## Decision

The memory store does not write instructions. A full apply removes the generated project-memory
block from each supported global instruction file, after backing up the file. It recognizes the
owned markers and heading. Other text and the memory files are preserved by this cleanup.

## Consequences

- The manager can still enumerate and edit entries in the auxiliary store.
- Codex receives eligible default-profile Claude project memories through its native importer.
- Availability in the manager does not guarantee a fresh agent session loads the note.
- Session-only project apply leaves instruction files alone.

## Source anchors

- `crates/core/src/adapters/sections.rs` (`remove_memory_routing`)
- `crates/core/src/adapters/memory_store.rs`
- `crates/core/src/adapters/codex_memory.rs`
- `crates/cli/tests/cli.rs` (`a_full_apply_removes_memory_routing_without_removing_notes`)

## Rules

- MUST: Back up the instruction file before removing the generated block.
- MUST: Preserve other instruction text and memory files during cleanup.

## Verification checklist

- [ ] Full apply removes the generated block and preserves its backup
- [ ] A session-only project apply leaves the block alone

## Related docs

- `wiki/kinds/memory.md`
- `wiki/decisions/native-flow-only.md`

## Last updated

2026-11-22
