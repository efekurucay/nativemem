# Decision: native flow only

## Purpose

Record why `nativemem` uses only files and formats the agents already read, with no plugins,
extensions, hooks or edits of agent configuration beyond one owned MCP entry.

## Scope

Every adapter and every future resource kind.

## Decision

An adapter writes only content an agent loads through its own native mechanism: a memory file in the
agent's memory directory, a section of its global instruction file, a transcript in its session
directory. `nativemem` installs no plugin or extension, registers no hook, and does not edit agent
configuration files (`config.toml`, `hooks.json`, `opencode.json`, `settings.json`, `config.yaml`,
`~/.claude.json`) or agent databases.

One exception, approved by the user: `nativemem mcp install` writes the entry named `nativemem` into
each agent's own MCP configuration, and `mcp uninstall` removes it
(`wiki/decisions/mcp-hub.md`). The edit touches that entry only; every other byte of the file stays.
It writes nothing else: no hook, no plugin, no other key, and no other file.

## Why

- Configuration files are rewritten by the agents themselves. Codex rewrites `config.toml` and
  hash-pins each hook's command text, so a changed hook becomes untrusted; Claude Code rewrites
  `~/.claude.json` constantly; Hermes re-dumps `config.yaml` and drops comments
  (`wiki/agents/codex.md`, `wiki/agents/claude-code.md`, `wiki/agents/hermes.md`). The one owned entry is
  edited with the procedure the agent's file allows: Claude by optimistic read-modify-write, Codex
  and OpenCode with format-preserving editors, Pi by a JSON merge.
- Plugins and extensions run code inside the agent, widen what the tool can break, and have to be
  installed and trusted per agent.
- A marked block in a user-owned file or a replica file in a directory the agent already reads is
  visible, reviewable and reversible from the backup directory. So is the `nativemem` MCP entry.
- Agent-owned databases (Codex SQLite) have schemas that belong to the agent.

## Consequences

- Agents without native per-project memory get a routing block in their global instruction file
  (`wiki/decisions/routing-block-for-agents-without-memory.md`).
- Session writes into Codex include the `threads` row Codex lists from and run only while the
  schema probes pass (`wiki/decisions/codex-session-writes-gated.md`).
- MCP servers are not copied between agent configs: the hub holds them and each agent has the one
  `nativemem` entry (`wiki/decisions/mcp-hub.md`).
- The one agent-adjacent JSON file the tool edits besides the MCP entries is Codex's importer ledger,
  and only with Codex session writes on.

## Source anchors

- `crates/hub/src/register.rs`
- `crates/core/src/adapters/memory_store.rs`
- `crates/core/src/adapters/codex_imports.rs`
- `wiki/architecture/safety.md`

## Rules

- MUST: Not add plugin, extension, hook or config-file edits beyond the `nativemem` MCP entry without a new decision record and the user's approval.
- MUST: Not touch agent-owned databases.
- MUST: Edit the `nativemem` MCP entry only through `crates/hub/src/register.rs` (backup, atomic write, state lock).
- SHOULD: Prefer a marked, regenerated region over an edit in place when a file is user-owned.

## Verification checklist

- [ ] A grep of `crates/` for writes to `config.toml`, `hooks.json`, `settings.json`, `opencode.json`, `config.yaml` or `.claude.json` finds them only in `crates/hub/src/register.rs`
- [ ] `crates/hub/src/register.rs` writes no key other than the `nativemem` entry (`crates/hub/tests/register.rs`)
- [ ] A new write path lists its files on the agent page

## Related docs

- `wiki/decisions/mcp-hub.md`
- `wiki/decisions/no-shared-store.md`
- `wiki/architecture/safety.md`

## Last updated

2026-11-11
