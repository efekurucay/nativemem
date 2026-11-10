# MCP hub

## Purpose

What the hub is, where its configuration and secrets live, how it names and forwards tools, and
what the `nativemem mcp` commands do, so the hub can be changed without re-deriving its rules.

## Scope

The `nativemem-hub` crate and `nativemem mcp` in the CLI: configuration, secret storage,
proxying, management commands and the import of the agents' MCP entries. Registering the hub in
the agents and OAuth login are not built; `auth = "oauth"` is stored and has no effect. The hub is
not a sync kind: it does not use the engine, the ledger or the adapters. Agent formats for MCP
entries are on `wiki/agents/*.md`; JSON shapes of the commands are in `docs/desktop.md`.

## What it is

One local MCP server, `nativemem mcp serve`, that an agent registers once. The real MCP servers
(stdio or streamable HTTP) and their secrets are configured in one place, the hub connects to
them, and the agent sees their tools behind one connection. The hub speaks MCP over stdio on its
own stdin and stdout; diagnostics go to stderr only. Built on `rmcp` (the official Rust MCP SDK)
with tokio. Anchors: `crates/hub/src/serve.rs`, `crates/hub/src/connect.rs`.

## Configuration

`<state dir>/hub.toml`, read by every command and written by the management commands under the
state lock, with a backup of the previous file at
`<state dir>/backups/<stamp>/hub/hub.toml` and an atomic write (`HubConfig::save` in
`crates/hub/src/config.rs`; test `config_round_trips_and_backs_up_the_previous_file`).

```toml
[servers.github]
transport = "stdio"            # "stdio" | "http"
command = "npx"                # stdio
args = ["-y", "pkg"]
cwd = "/abs/dir"               # absolute
auth = "none"                  # "none" | "oauth" (stored only; stdio takes "none")
enabled = true

[servers.github.env]
GITHUB_TOKEN = "secret:github-GITHUB_TOKEN"

[servers.docs]
transport = "http"
url = "https://mcp.example.com/mcp"

[servers.docs.headers]
Authorization = "secret:docs-Authorization"
```

- A stdio server takes `command`, `args`, `env`, `cwd`; an http server takes `url`, `headers`;
  the other group is rejected (`ServerSpec::validate`).
- Unknown keys are a parse error (`deny_unknown_fields`).
- A server name is 1-32 characters of `A-Za-z0-9_-`, because it prefixes tool names. The name
  `nativemem` is refused: it is the name the hub is registered under, so such a server would be
  the hub calling itself.
- Every string value (command, args, env and header values, cwd, url) may be `secret:<id>`.

## Secrets

Values behind `secret:<id>` live in a `SecretStore` (`crates/hub/src/secrets.rs`):

| Store | Where | Used by |
|---|---|---|
| `KeychainStore` | the OS keychain through the `keyring` crate, service `nativemem`, account `<id>` | production |
| `FileStore` | `<dir>/secrets.json` | tests, selected by the hidden flag `nativemem mcp --secrets-dir <dir>`; not backed up, because a backup would copy the values into the backup tree |

- A secret id is 1-128 characters of `A-Za-z0-9-_.`.
- Values are resolved only when connecting (`resolve` in `crates/hub/src/connect.rs`) and passed to
  the child's command line and environment or to the HTTP request. A reference to a missing secret
  fails that server with `secret "<id>" is not set`.
- Connection error text is scrubbed: every resolved secret value is replaced by `***` before the
  message is stored or printed (`scrub`; test `scrub_hides_secret_values`).
- `mcp list` shows `secret:<id>` references. A literal env or header value is shown as
  `<literal>`, because a literal may itself be a secret (test `add_list_edit_enable_disable_remove`).
  Put secrets behind a reference: command, args, url and cwd are printed as stored.
- An HTTP `Authorization` header must be a `Bearer` token: the transport sends that header itself
  and takes the token without its scheme. Other headers are sent as given.
- The hub never writes a secret value into `hub.toml`, the ledger, a backup, an error or a log.
  Tests: `secret_references_reach_the_child_and_never_the_output`,
  `secrets_are_stored_listed_by_reference_and_deleted`.

## Serving

| Step | Behaviour | Anchor |
|---|---|---|
| Connect | On `initialize` (or on the first tool request, whichever comes first) the enabled servers are connected concurrently, each with a 20 s timeout for connecting and listing its tools | `connect_all`, `CONNECT_TIMEOUT` |
| Tool names | `<server>__<tool>` with every character outside `A-Za-z0-9_-` replaced by `_`, cut at 64 characters; a collision after sanitising gets a numeric suffix `_2`, `_3`, ... within the 64 | `exposed_name`, `unique_name` |
| Descriptions | `[<server>] ` is put in front of the upstream description | `connect_all` |
| Calls | The request is forwarded to the owning server with its original tool name; the response, including an MCP error, goes back unchanged. A transport failure becomes an error result that names the server | `HubServer::call_tool` |
| Failures | A server that fails to connect is left out of the tool list | test `failing_and_disabled_upstreams_are_reported_and_left_out` |
| `nativemem_status` | A tool with no arguments; its text result is `{"servers": [{"name", "status": "connected" \| "error" \| "disabled", "tools"?, "error"?}]}` | `STATUS_TOOL` |
| Scope | Tools only: no resources and no prompts | `HubServer::get_info` |

Tests: `tools_are_prefixed_and_calls_are_forwarded_unchanged` (in-process), and
`serve_proxies_the_fake_server_over_a_child_process` in `crates/cli/tests/mcp.rs`, which starts
`nativemem mcp serve` as a child process and talks to it with an `rmcp` client. The upstream in both
is `nativemem-fake-mcp` (`crates/hub/src/bin/fake_mcp.rs`, tools `echo` and `env_matches`); an HTTP
upstream is covered by `http_upstream_receives_resolved_headers`.

## CLI

All commands take `--json` and use the error shape and codes of `docs/desktop.md`.

| Command | Does |
|---|---|
| `mcp serve` | Runs the hub on stdio |
| `mcp list` | Servers with transport, enabled, auth and secret references |
| `mcp add` | Adds one server from a JSON document on stdin (`name` plus the fields of `hub.toml`); code `exists` for a duplicate name |
| `mcp edit <name>` | Replaces that server with the JSON document on stdin |
| `mcp remove <name>` | Removes the server; its secrets stay |
| `mcp enable <name>`, `mcp disable <name>` | Sets `enabled` |
| `mcp set-secret <id>` | Stores the value read from stdin (one trailing newline dropped) |
| `mcp delete-secret <id>` | Deletes it; code `not_found` when it does not exist |
| `mcp test <name>` | Connects, lists the tools, disconnects; exit code 2 when it failed |
| `mcp import [--apply]` | Reads the agents' MCP entries and prints the plan; `--apply` writes `hub.toml` and the secrets |

Every command except `serve`, `list`, `test` and a dry-run `import` runs under the state lock
(`McpArgs::writes` in `crates/cli/src/mcp.rs`).

## Import

`mcp import` reads, read-only, the global MCP entries of the four agents at the roots the CLI
resolved (`crates/hub/src/import.rs`):

| Agent | File | Key |
|---|---|---|
| Claude Code | `.claude.json` (next to `~/.claude`, or inside a custom config directory) | `mcpServers` |
| Codex | `<codex home>/config.toml` | `[mcp_servers.<name>]` |
| OpenCode | `<opencode home>/opencode.json`, then `opencode.jsonc` (comments and trailing commas allowed) | `mcp.<name>` |
| Pi | `<pi home>/mcp.json` | `mcpServers` |

- Entries become hub entries. OpenCode's `command` array is split into `command` and `args`.
- Identical entries from several agents are one entry. The same name with a different config is a
  `conflict`: nothing is imported for it, and the agents involved are listed. A name the hub already
  holds with a different config is a conflict with `hub` among its agents. An identical hub entry is
  `present`.
- Every env and header value moves into the secret store as `secret:<server>-<key>` (characters
  outside `A-Za-z0-9-_.` become `_`). A value that an agent resolves itself, `${VAR}` or `!command`,
  cannot be evaluated by the hub: its secret is created unset and a warning names the id.
- Skipped: the entry named `nativemem`, `sse` entries, entries without a command or url, names the
  hub does not accept, and project-scoped entries (only the global ones are read).
- Keys the hub cannot carry (for example Codex `bearer_token_env_var`) are named in a warning, never
  their values. An unreadable file is a warning that names the path.
- A dry run prints the plan and writes nothing. Exit code 2 when a conflict remains.

Tests: `crates/hub/tests/import.rs`, `import_previews_then_applies_from_the_four_agents`.

## Source anchors

- `crates/hub/src/config.rs`
- `crates/hub/src/secrets.rs`
- `crates/hub/src/connect.rs`
- `crates/hub/src/serve.rs`
- `crates/hub/src/import.rs`
- `crates/hub/src/bin/fake_mcp.rs`
- `crates/hub/tests/hub.rs`
- `crates/hub/tests/import.rs`
- `crates/cli/src/mcp.rs`
- `crates/cli/tests/mcp.rs`
- `docs/desktop.md`

## Rules

- MUST: Never print, log or store a secret value outside the secret store; show `secret:<id>`.
- MUST: Write `hub.toml` only through `HubConfig::save` (backup, atomic write) under the state lock.
- MUST: Keep stdout of `mcp serve` for the protocol; diagnostics go to stderr.
- MUST: Run tests with `--secrets-dir` and temp roots; never the keychain, never a real agent home.
- MUST: Leave the agents' own config files unwritten; import reads them only.
- SHOULD: Put secrets behind `secret:<id>`; only env and header values are moved by import.

## Verification checklist

- [ ] Every repo path in backticks exists
- [ ] `cargo test --workspace` passes, including `crates/hub/tests` and `crates/cli/tests/mcp.rs`
- [ ] A new `mcp` command with JSON output is described in `docs/desktop.md`
- [ ] No test or fixture contains a real token

## Related docs

- `wiki/architecture/overview.md`
- `wiki/architecture/safety.md`
- `wiki/architecture/file-structure.md`
- `wiki/guides/add-a-resource-kind.md`

## Last updated

2026-11-11
