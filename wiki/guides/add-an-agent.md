# Add an agent

## Purpose

An end-to-end checklist an engineer or coding agent can follow to add support for a new coding
agent, from read-only research to a verified real-agent rollout.

## Scope

A new `AgentId`. Adding a resource kind is in `wiki/guides/add-a-resource-kind.md`; the staged
rollout recipe is in `wiki/guides/verify-with-real-agents.md`. OpenCode (generic `AGENTS.md`
adapter) and Claude Code (full adapter with sessions) are the two reference implementations.

## Checklist

### 1. Research read-only first

- [ ] Copy `wiki/_templates/AGENT.md` to `wiki/agents/<agent>.md`.
- [ ] Find the native locations (global and project) of instructions, memory, sessions, MCP servers
  and skills. Read files and installed sources only. Never run `nativemem` or tests against a real
  home; copy SQLite files with their `-wal` and `-shm` files to a scratch directory before opening.
- [ ] Record the agent's version and how it is detected. Record every fact with an evidence level
  (VERIFIED, OBSERVED, INFERRED, `[UNVERIFIED]`) as in `wiki/architecture/context-quality.md`.
- [ ] Answer, per kind: does the agent have a native store, what file or table format, what marks a
  file as ours, which files does the agent rewrite itself, how does it list and resume sessions.
- [ ] Never copy secrets or message text into the page; structure only, placeholder paths.
- [ ] If the agent needs no config edit, plugin or extension for the chosen placement, say so on the
  page (`wiki/decisions/native-flow-only.md`). If it does, stop and decide with the user.

### 2. Decide capabilities

- [ ] Memory: native originals, or replicas only (then use the routing block,
  `wiki/decisions/routing-block-for-agents-without-memory.md`).
- [ ] Instruction sections: which global file, does the agent actually load it (and does something
  shadow it, as OpenCode's fallback or Pi's `AGENTS.override.md`).
- [ ] Sessions: reader only, writer, or neither. A session writer needs a marker that survives a lost
  ledger, a derived stable id so a refresh rewrites the same file, and a way to appear in the agent's
  own resume list.

### 3. Adapter and `supports`

- [ ] Add `crates/core/src/adapters/<agent>.rs` and list it in `crates/core/src/adapters/mod.rs`. An
  agent that only reads a global `AGENTS.md` reuses `AgentsFileAdapter::new("<id>", dir, state_dir,
  backups)` as OpenCode does; otherwise implement `Adapter` (`id`, `version`, `probes`, `supports`,
  `writer_revision`, `is_foreign_replica`, `scan`, `write_replica`, `write_original`, `remove`).
- [ ] `supports(kind, scope)` returns true only for what the agent can store, and false for the kinds
  in a `with_blocked_kinds(...)` list (a builder every adapter has). The engine then plans nothing else
  for it.
- [ ] Probes are mandatory (`wiki/architecture/compatibility.md`). Add `crates/core/src/adapters/<agent>_probes.rs`
  with `version(...)` and `probes(home)`: read-only, fast checks that compare structural descriptors of the
  agent's files with expected constants for every format the adapter reads or writes (key sets, type sets,
  column signatures, directory encoding, layout), each tagged with the kinds it protects and the repo paths
  of its adapter files. Wire them into `Adapter::version` and `Adapter::probes`, and list the expected
  constants with their evidence on the agent's page.
- [ ] Every write goes through `fsx::atomic_write`; every delete or overwrite calls `fsx::backup`
  first. Content outside the managed regions stays byte-identical.
- [ ] Replicas carry a marker that survives a lost ledger (`wiki/kinds/memory.md`,
  `wiki/kinds/sessions.md`).
- [ ] `is_session_id` classifies session ids by the `.jsonl` suffix (`crates/core/src/session.rs`). A
  database-backed agent whose native ids do not end in `.jsonl` needs that rule extended, or its
  sessions would be treated as memory ids by `delete` and `edit`.
- [ ] Session writer: set `writer_revision` for `Kind::Session`, bump the constant whenever the
  written format changes, set the replica file's mtime to the original's, derive ids from the origin.
- [ ] A transcript reader maps into the neutral `Session` through `TurnBuilder` so tool activity
  renders like every other source.

### 4. Roots, flags and environment in the CLI (`crates/cli/src/main.rs`)

- [ ] Global flag `--<agent>-home` with env `NATIVEMEM_<AGENT>_HOME` on `Cli`.
- [ ] Field on `Roots`; resolution in `resolve` through `pick`, with the agent's own environment
  variable as fallback and the default directory under the user's home.
- [ ] Field on `Agents`; constructed in `Agents::open` only when the home exists (`present(...)`), so
  an agent without a home takes no part in a run, with `.with_blocked_kinds(&paused("<id>"))`. Add it to
  `all()`, `named()` and `AGENT_IDS`.
- [ ] The `UnknownAgent` message and the `--agent` help text of `delete` and `edit` list the new id.
- [ ] `status`: add the agent to the session-report match when it has a session reader.
- [ ] `crates/cli/src/schedule.rs`: add the root to `ExplicitRoots`, to `run_schedule` and to the flag
  list passed to the job, so a scheduled run uses the same root.
- [ ] The macOS app: add the display name in `agentDisplayName`
  (`apps/macos/Sources/NativememKit/MemoryIndex.swift`) and a decoding fixture or test if the JSON
  contract changes.

### 5. Capability table and docs

- [ ] Update the capability table in `wiki/architecture/overview.md` and the check list in
  `wiki/architecture/compatibility.md`.
- [ ] Update the support table in `README.md` and the flags and variables line in its Usage section.
- [ ] Fill `wiki/agents/<agent>.md` from the research; add it to `wiki/README.md` and
  `wiki/architecture/file-structure.md`.
- [ ] Add the per-agent checks to `wiki/guides/verify-with-real-agents.md`.

### 6. Tests with fixtures in temp dirs

- [ ] Add a home builder to `crates/core/src/testkit.rs` (like `ClaudeHome`, `CodexHome`, `PiHome`).
- [ ] Probe tests in `crates/core/tests/probes.rs`: every probe passes on the new testkit home, each drift
  (an unknown type, a changed key set, a changed version marker) fails exactly its check, and
  `with_blocked_kinds` makes `supports` false for exactly those kinds. A CLI drift test in
  `crates/cli/tests/doctor.rs` shows the other agents keep syncing.
- [ ] Adapter tests: scan of every format variant, `write_replica` round trip, `write_original`
  preserving unknown content, `remove` with backup, unsupported scopes.
- [ ] Engine tests: the new agent in a fixture with the other agents (see `tri` in
  `crates/core/src/engine.rs`): add, edit on each side, delete on each side, conflict.
- [ ] CLI test in `crates/cli/tests/cli.rs`: pass every root explicitly and redirect `HOME` as the
  `run` helper does; cover "agent without a home takes no part".
- [ ] Run `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo fmt --all -- --check`, and `swift test` in `apps/macos`.

### 7. Staged apply, verified by the user in the real agent

- [ ] Follow `wiki/guides/verify-with-real-agents.md`: preview, a narrow slice, the user checks the
  agent's own listing and opens an item, then everything.
- [ ] Record the outcome on the agent page and raise the evidence levels of the facts it confirmed.
- [ ] Anything that writes into a store the agent builds its list from, and cannot be verified, stays
  behind an explicit switch until it can (`wiki/decisions/codex-session-writes-gated.md`).

## Source anchors

- `crates/core/src/model.rs` (`Adapter`)
- `crates/core/src/adapters/agents_file.rs`
- `crates/core/src/adapters/claude.rs`
- `crates/core/src/adapters/claude_probes.rs`
- `crates/core/src/testkit.rs`
- `crates/cli/src/main.rs`
- `crates/cli/src/schedule.rs`
- `apps/macos/Sources/NativememKit/MemoryIndex.swift`

## Rules

- MUST: Research read-only before writing code, and record evidence levels.
- MUST: Declare capabilities in `supports`; take part in a run only when the home exists.
- MUST: Ship probes with the adapter (`wiki/architecture/compatibility.md`).
- MUST: Back up and write atomically on every write path.
- MUST: Test against temp directories with explicit roots only.
- MUST: Update README, wiki and the Swift display name in the same task.
- SHOULD: Reuse `AgentsFileAdapter` and `memory_store.rs` before writing a new store.

## Verification checklist

- [ ] Every box in the Checklist section is ticked or consciously skipped with a reason on the agent page
- [ ] `cargo test --workspace` passes without touching any real home
- [ ] `nativemem doctor --agent <id>` on temp roots reports every probe ok
- [ ] The staged check outcome is recorded on `wiki/agents/<agent>.md`

## Related docs

- `wiki/agents/opencode.md`
- `wiki/agents/claude-code.md`
- `wiki/guides/verify-with-real-agents.md`
- `wiki/guides/add-a-resource-kind.md`
- `wiki/architecture/overview.md`
- `wiki/architecture/compatibility.md`

## Last updated

2026-11-11
