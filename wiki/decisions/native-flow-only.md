# Decision: native flow only

## Purpose

Record why `nativemem` uses only files and formats the agents already read, with no plugins,
extensions or edits of agent configuration.

## Scope

Every adapter and every future resource kind.

## Decision

An adapter writes only content an agent loads through its own native mechanism: a memory file in the
agent's memory directory, a section of its global instruction file, a transcript in its session
directory. `nativemem` installs no plugin or extension, registers no hook, and does not edit agent
configuration files (`config.toml`, `hooks.json`, `opencode.json`, `settings.json`, `config.yaml`,
`~/.claude.json`). Agent databases are written in exactly two places, with the user's approval: the
`threads` rows of Codex's state database for the rollouts this tool wrote, and the session rows of
OpenCode's `opencode.db` for the replicas this tool wrote. Both run only while the compatibility probes
pass and only after a database backup; no other database file is touched.

## Why

- Configuration files are rewritten by the agents themselves. Codex rewrites `config.toml` and
  hash-pins each hook's command text, so a changed hook becomes untrusted; Claude Code rewrites
  `~/.claude.json` constantly; Hermes re-dumps `config.yaml` and drops comments
  (`wiki/agents/codex.md`, `wiki/agents/claude-code.md`, `wiki/agents/hermes.md`).
- Plugins and extensions run code inside the agent, widen what the tool can break, and have to be
  installed and trusted per agent.
- A marked block in a user-owned file or a replica file in a directory the agent already reads is
  visible, reviewable and reversible from the backup directory.
- Agent-owned databases have schemas that belong to the agent, so the two writers refuse to run when
  the schema differs from the probed shape and never change or delete a row the agent made.

## Consequences

- Agents without native per-project memory get a routing block in their global instruction file
  (`wiki/decisions/routing-block-for-agents-without-memory.md`).
- Session writes into Codex include the `threads` row Codex lists from, and session writes into
  OpenCode are rows of `opencode.db`; both run only while the schema probes pass
  (`wiki/decisions/codex-session-writes-gated.md`, `wiki/decisions/opencode-session-writes-gated.md`).
- MCP servers and skills would require config edits for some agents and are not implemented; the
  placement research on the agent pages names the config-free options first.
- The one agent-adjacent JSON file the tool edits is Codex's importer ledger, and only for the Codex
  session replicas it writes.

## Source anchors

- `docs/design.md` (Anti-goals)
- `crates/core/src/adapters/memory_store.rs`
- `crates/core/src/adapters/codex_imports.rs`
- `crates/core/src/adapters/codex_threads.rs`
- `crates/core/src/adapters/opencode_sessions.rs`
- `wiki/architecture/safety.md`

## Rules

- MUST: Not add plugin, extension, hook or config-file edits without a new decision record and the user's approval.
- MUST: Not touch agent-owned databases except the Codex `threads` rows (`codex_threads.rs`) and the
  OpenCode session rows (`opencode_sessions.rs`), under probe gating with a database backup.
- SHOULD: Prefer a marked, regenerated region over an edit in place when a file is user-owned.

## Verification checklist

- [ ] A grep of `crates/` finds no write to `config.toml`, `hooks.json`, `settings.json`, `opencode.json`, `config.yaml` or `.claude.json`
- [ ] A new write path lists its files on the agent page
- [ ] A grep of `crates/` finds database writes only in `codex_threads.rs` and `opencode_sessions.rs`

## Related docs

- `wiki/decisions/no-shared-store.md`
- `wiki/architecture/safety.md`

## Last updated

2026-11-11
