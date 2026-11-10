# Add a resource kind

## Purpose

A checklist for adding a new kind of resource (for example MCP server entries or skills) so that it
flows through the engine with a deliberate policy, safe writes and complete docs.

## Scope

A new `Kind` variant and the adapters that store it. A new agent is in
`wiki/guides/add-an-agent.md`. MCP servers are research only today: their locations and
schemas are on the agent pages, no code exists. Skills are implemented (`wiki/kinds/skills.md`) and
are the reference for a kind without a marker and with a directory as its unit.

## Checklist

### 1. Research and policy

- [ ] Per agent, record where the kind lives, its format, who else rewrites that file, and whether it
  can hold secrets. Evidence levels on the agent pages.
- [ ] Choose the policy and write a decision record in `wiki/decisions/`: faithful replication (adds,
  edits, deletes, write-back), snapshot (copy once, never delete), or something new. State the item
  granularity ("one `## ` section is one item") and what is identity versus content.
- [ ] Decide the scope of an item (`Global` or `Project`) and which agents can store which scope.
- [ ] If values can be secrets, decide how they are carried without being printed or stored in the
  ledger. The ledger stores ids, hashes and paths only.
- [ ] If the kind lives in a file the agent rewrites itself (a config file), decide the edit
  procedure first: format-preserving edit of only the owned subtree, same-directory temp file and
  rename, backup, re-read before replace. Files with hash-pinned content must stay byte-identical
  (`wiki/agents/codex.md`).

### 2. Model

- [ ] Add the variant to `Kind` in `crates/core/src/model.rs` and to `as_str` and `parse`; the test
  `kind_text_round_trips` iterates the variants.
- [ ] Define how an item maps to `name`, `description`, `category`, `body`, and what `content_hash`
  covers. Anything the agent regenerates on each save must be outside the hash, as session ids and
  timestamps are.
- [ ] Define `native_id` so it is stable inside one agent and distinguishes the kind (sessions end in
  `.jsonl`, memory in `.md`, instructions contain `#`; `is_session_id` relies on that).

### 3. Engine

- [ ] Replication kinds go through `plan_key` in `crates/core/src/engine.rs` unchanged; check the
  `WriteBack` branch that decides which fields travel back (the heading of an instruction section is
  identity and does not).
- [ ] A kind with its own policy gets its own planner like `plan_session`, selected at the top of
  `plan_key`.
- [ ] Decide whether the kind counts toward the mass-delete total (`apply` excludes sessions).
- [ ] `item_label` and the plan display lines cover the new kind.

### 4. Adapters

- [ ] Each adapter's `supports(kind, scope)` returns true only for the agents and scopes that can
  hold it; add `writer_revision` for the kind if the written format may change.
- [ ] `scan`, `write_replica`, `write_original`, `remove` handle the kind; every write is atomic and
  backed up; text outside managed regions is byte-identical.
- [ ] Replicas carry a marker that survives a lost ledger.

### 5. CLI and app

- [ ] `items --kind` value parser lists the kind (`crates/cli/src/main.rs`); decide whether the kind is
  listed by default or only when asked for (sessions are only listed when asked for).
- [ ] `delete` and `edit` behaviour for the kind: allowed, restricted (instruction `edit` changes the
  body only), or refused with code `unsupported`.
- [ ] `status` counts the kind if it should be reported apart from entries.
- [ ] JSON contract: add `kind` values to `docs/desktop.md` only as a reported drift unless the user
  asked for a spec edit; add constants and a decoding test in
  `apps/macos/Sources/NativememKit/Contract.swift` and `apps/macos/Tests/NativememKitTests/`.
  Decide whether the app shows or filters the kind (sessions are filtered out of the memory views in `MemoryIndex.swift` and listed by `SessionIndex.swift`).

### 6. Tests

- [ ] Adapter tests per agent, including tricky bodies and odd characters in ids.
- [ ] Engine tests: add, edit on each side, delete on each side, conflict, lost ledger adoption, and
  `supports` filtering (see `memory_is_planned_only_for_agents_that_support_it`).
- [ ] CLI tests with explicit temp roots and redirected `HOME`.
- [ ] `cargo test --workspace`, clippy, fmt, and `swift test` in `apps/macos`.

### 7. Docs and rollout

- [ ] `wiki/kinds/<kind>.md`, the policy-per-kind table in `wiki/architecture/overview.md`, the kind
  row in every `wiki/agents/*.md` status table, `README.md`, `wiki/README.md`.
- [ ] Staged rollout (`wiki/guides/verify-with-real-agents.md`) with a check per agent.

## Source anchors

- `crates/core/src/model.rs`
- `crates/core/src/engine.rs`
- `crates/core/src/adapters/sections.rs`
- `crates/cli/src/main.rs`
- `apps/macos/Sources/NativememKit/Contract.swift`
- `wiki/decisions/instruction-sections-as-items.md`
- `wiki/decisions/session-snapshot-policy.md`

## Rules

- MUST: Write the policy decision before the code.
- MUST: Keep the engine format-agnostic; formats live in adapters.
- MUST: Never print or store secret values; the ledger holds ids, hashes and paths.
- MUST: Edit agent-owned config files only through a format-preserving, atomic, backed-up procedure.
- SHOULD: Start with one agent and one scope, then widen with `supports`.

## Verification checklist

- [ ] The decision record exists and is linked from the kind page
- [ ] Every adapter declares the kind in `supports`
- [ ] The wiki routing table row "New resource kind" in `wiki/WIKI-CREATOR.md` was followed

## Related docs

- `wiki/guides/add-an-agent.md`
- `wiki/architecture/overview.md`
- `wiki/decisions/native-flow-only.md`
- `wiki/WIKI-CREATOR.md`

## Last updated

2026-11-11
