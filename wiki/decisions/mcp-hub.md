# Decision: one MCP hub instead of copying MCP configs

## Purpose

Record why MCP servers are not synced between the agents as config entries, and what replaces the
copy: one hub that every agent registers once.

## Scope

The `nativemem` MCP hub (`wiki/architecture/hub.md`) and the one config entry `nativemem mcp install`
writes into each agent. Skills, memory and sessions are unaffected.

## Decision

`nativemem` does not copy MCP server entries from one agent's config to another. The MCP servers and
their secrets live once, in the hub (`<state dir>/hub.toml` plus the OS keychain), and each agent
gets exactly one entry named `nativemem` in its own MCP configuration. That entry starts
`<nativemem binary> mcp serve`; the agent sees every hub server's tools behind that connection.

The entry is the only agent configuration `nativemem` edits. It sits in:

| Agent | File | Key |
|---|---|---|
| Claude Code | `.claude.json` (`claude_config_path` in `crates/hub/src/import.rs`) | `mcpServers.nativemem` |
| Codex | `<codex home>/config.toml` | `[mcp_servers.nativemem]` |
| OpenCode | `<opencode home>/opencode.json`, or `opencode.jsonc` when that is the file present | `mcp.nativemem` (`mcp.servers.nativemem` when the file keeps its servers there) |
| Pi | `<pi home>/mcp.json` | `mcpServers.nativemem` |

## Why

- Copying entries cannot carry sign-in state. An OAuth token is bound to the client that registered
  with the authorization server, so a token one agent obtained is useless to another agent's client.
  The hub is one client with one registration: it signs in once, for all agents
  (`wiki/architecture/hub.md`, "OAuth").
- Secrets stay in one place. Entries copied between agent files would put the same token into four
  files with four formats and four ways of resolving `${VAR}` or `!command`.
- A server added to the hub reaches every agent that has the entry, with no write into an agent file.
  After `mcp install`, MCP changes touch `hub.toml` only.
- The entry is small, owned and reversible: `mcp uninstall` removes exactly that entry, and every
  write keeps the rest of the file byte for byte (`crates/hub/tests/register.rs`).

## Trade-off

While the hub cannot start (binary moved or deleted, state directory unreadable, keychain locked),
the agent has no MCP tools from the hub. `nativemem mcp status` checks the entry:
it reports `outdated` when the registered command differs from the binary that would be written now.
An agent starts the hub as its own process, so each agent runs its own hub and its own upstream
connections.

## Consequences

- `mcp install` writes agent config files, so it follows the config-edit procedure of
  `wiki/decisions/native-flow-only.md`: format-preserving edit, backup first, atomic write, state lock,
  and a file that fails to parse is never written (error code `parse`).
- Claude rewrites `.claude.json` constantly, so its edit is an optimistic read-modify-write: the file is
  read again just before the rename and the edit starts over, up to 3 times, when it changed.
- The entry carries `--claude-home`, `--codex-home`, `--opencode-home`, `--pi-home` and `--state-dir`
  only for roots the user chose explicitly, so a default install registers `mcp serve` alone.
- `mcp import` skips the entry named `nativemem`, and no hub server may take that name.

## Source anchors

- `crates/hub/src/register.rs`
- `crates/hub/tests/register.rs`
- `crates/cli/src/mcp.rs`
- `crates/cli/tests/mcp.rs`
- `wiki/architecture/hub.md`

## Rules

- MUST: Register the hub under the name `nativemem` and write no other entry into an agent's MCP config.
- MUST: Keep every other byte of an agent's config file as it was.
- MUST: Not copy MCP server entries between agent configs.
- SHOULD: Put new MCP servers into the hub (`mcp add` or `mcp import`), not into agent files.

## Verification checklist

- [ ] `crates/hub/tests/register.rs` passes: insert, idempotent second install, uninstall, byte preservation
- [ ] `nativemem mcp status` on temp roots reports `registered` after `mcp install`
- [ ] No other agent config key is written by `register.rs`

## Related docs

- `wiki/decisions/native-flow-only.md`
- `wiki/decisions/no-shared-store.md`
- `wiki/architecture/hub.md`
- `wiki/architecture/safety.md`

## Last updated

2026-11-11
