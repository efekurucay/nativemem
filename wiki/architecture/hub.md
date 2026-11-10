# MCP hub

## Purpose

What the hub is, where its configuration and secrets live, how it names and forwards tools, what
the `nativemem mcp` commands do and how it runs subagents, so the hub can be changed without
re-deriving its rules.

## Scope

The `nativemem-hub` crate and `nativemem mcp` in the CLI: configuration, secret storage,
proxying, OAuth sign-in, management commands, the import of the agents' MCP entries and the
registration of the hub in the agents, and the subagents it offers as tools. The hub is not a sync kind: it does not use the engine, the
ledger or the adapters. Agent formats for MCP entries are on `wiki/agents/*.md`; JSON shapes of the
commands are in `docs/desktop.md`; why the hub exists is in `wiki/decisions/mcp-hub.md`.

## What it is

One local MCP server, `nativemem mcp serve`, that an agent registers once. The real MCP servers
(stdio or streamable HTTP) and their secrets are configured in one place, the hub connects to
them, and the agent sees their tools behind one connection. The hub speaks MCP over stdio on its
own stdin and stdout; diagnostics go to stderr only. Built on `rmcp` (the official Rust MCP SDK)
with tokio. Anchors: `crates/hub/src/serve.rs`, `crates/hub/src/connect.rs`. `nativemem mcp install`
puts the hub into each agent's MCP config once ("Registration").

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
auth = "none"                  # "none" | "oauth" (http only; no Authorization header)
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
- An `oauth` server takes no `Authorization` header: the hub sends the stored bearer token.
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
| Sign-in missing | An `oauth` server without a usable sign-in is not connected; it exposes one tool, `<server>__sign_in_required`, that answers with the error result `Sign in again: nativemem mcp login <server>` | `SIGN_IN_TOOL`; test `a_server_that_starts_signed_out_exposes_the_sign_in_tool` |
| Subagent tools | `subagents_list` and `subagent_<name>` are answered by the hub itself, before any upstream is involved (see "Subagents") | `HubServer::call_tool` |
| `nativemem_status` | A tool with no arguments; its text result is `{"servers": [{"name", "status": "connected" \| "error" \| "needs_login" \| "disabled", "tools"?, "error"?}]}`; for `needs_login`, `error` is the sign-in message | `STATUS_TOOL` |
| Scope | Tools only: no resources and no prompts | `HubServer::get_info` |

Tests: `tools_are_prefixed_and_calls_are_forwarded_unchanged` (in-process), and
`serve_proxies_the_fake_server_over_a_child_process` in `crates/cli/tests/mcp.rs`, which starts
`nativemem mcp serve` as a child process and talks to it with an `rmcp` client. The upstream in both
is `nativemem-fake-mcp` (`crates/hub/src/bin/fake_mcp.rs`, tools `echo` and `env_matches`); an HTTP
upstream is covered by `http_upstream_receives_resolved_headers`.

## OAuth

A server with `auth = "oauth"` is signed in once by `nativemem mcp login <name>`; every agent's hub
process then uses the stored tokens. The flow is the MCP authorization flow, on `rmcp`'s `auth`
module (`crates/hub/src/oauth.rs`): OAuth 2.1 authorization code with PKCE.

| Step | Behaviour | Anchor |
|---|---|---|
| Probe | An `initialize` request without a token; the 401 `WWW-Authenticate` header names the protected resource metadata (RFC 9728). Without that header the well-known URLs are tried. A success answer is an error: the server needs no sign-in | `probe`, test `login_refuses_a_server_without_oauth` |
| Discovery | Protected resource metadata, then authorization server metadata (RFC 8414) | test `login_discovers_registers_stores_and_the_bearer_reaches_the_upstream` |
| Registration | Dynamic client registration (RFC 7591) once; the client (`client_id`, optional secret, redirect URI) is stored | `StoredClient` |
| Redirect | A loopback listener on `http://127.0.0.1:<port>/callback`. The port of the stored client is bound again when it is free, so its registration stays valid; otherwise a random port and a new registration | `bind_loopback`, tests `a_second_login_reuses_the_registered_client`, `a_taken_loopback_port_registers_a_new_client` |
| Browser | `open` on macOS, `xdg-open` elsewhere; the URL is printed to stderr too. The hidden flag `mcp login --browser <program>` replaces the opener (tests) | `open_browser` |
| Wait | Up to 5 minutes (`LOGIN_TIMEOUT`) for `/callback`; an authorization error is reported by its short code, never its free text | tests `login_gives_up_when_the_browser_never_comes_back`, `a_refused_authorization_is_reported_without_free_text` |
| Store | Access token, refresh token and expiry are stored in the secret store | `HubCredentials` |

Secret ids, derived from the server name: `oauth-<name>-client` (the registered client) and
`oauth-<name>-tokens` (the tokens with their receive time and expiry). Both hold JSON and never reach
`hub.toml`, output or logs; connection errors are scrubbed of the token values too.

Using the tokens:

- On connect, an `oauth` server needs stored tokens and a stored client, else it is `needs_login`
  without a network call. The token is read, and refreshed when it expires within 30 seconds, before
  the first request; every request carries `Authorization: Bearer <token>`.
- A 401 on a request triggers one refresh and one retry (`rmcp`'s `AuthClient`; test
  `a_token_the_server_rejects_is_refreshed_and_the_call_retried`).
- Every agent runs its own hub process, so a refresh takes the file lock `<state dir>/hub-oauth.lock`
  and reads the store again after taking it, so it uses the refresh token another process may have
  stored meanwhile. A refresh waits up to 30 seconds for the lock (test
  `a_refresh_waits_for_the_lock_and_reads_the_store_again`).
- When the authorization server rejects the refresh token for good (`invalid_grant`), the tokens are
  deleted, unless another process stored new ones, and the server is `needs_login`. A call to a tool
  of a server that lost its sign-in during a session answers with the error result
  `Sign in again: nativemem mcp login <name>` (test
  `the_hub_reports_needs_login_and_its_tools_answer_with_the_sign_in_message`). A transient refresh
  failure leaves the tokens alone.
- `mcp logout <name>` deletes the tokens and keeps the registered client. `mcp list` shows
  `auth_state`: `signed_in` (tokens are stored; the authorization server can still have revoked
  them), `needs_login` (an `oauth` server without tokens) or `none`.
- `mcp login` waits for the browser, so it takes only `hub-oauth.lock` (when it stores the tokens),
  not the state lock; `mcp logout` takes `hub-oauth.lock` likewise.
- To make the next login register a new client, delete the secret `oauth-<name>-client`
  (`mcp delete-secret`).

Tests use a fake authorization server and MCP server on 127.0.0.1 (`crates/hub/tests/common/fake_oauth.rs`)
and a browser function that follows the authorization URL itself. No real network is used.

## Registration

`nativemem mcp install [--agent <id>] [--json]` registers the hub in each agent's own MCP
configuration, `mcp uninstall [--agent <id>]` removes the entry and `mcp status [--agent <id>]`
reports it (`crates/hub/src/register.rs`). The decision and its trade-off are in
`wiki/decisions/mcp-hub.md`.

- One entry named `nativemem`. Its command is the absolute path of the running binary
  (`std::env::current_exe`, symlinks resolved) with the arguments `mcp serve`. `--claude-home`,
  `--codex-home`, `--opencode-home`, `--pi-home` and `--state-dir` follow, as absolute paths, only for
  roots given explicitly by flag or environment variable (`ExplicitRoots` in `crates/cli/src/main.rs`).
- Where each agent holds it:

| Agent | File | Entry |
|---|---|---|
| Claude Code | `claude_config_path` of the Claude home: `.claude.json` next to a home named `.claude`, else inside the home (a custom `CLAUDE_CONFIG_DIR` is the Claude home) | `mcpServers.nativemem` = `{type: "stdio", command, args, env: {}}` |
| Codex | `<codex home>/config.toml` | `[mcp_servers.nativemem]` with `command`, `args`, `tool_timeout_sec = 1800` (long subagent runs are not cut) and `startup_timeout_sec = 60` (the hub connects its upstreams before it answers) |
| OpenCode | `<opencode home>/opencode.json`; `opencode.jsonc` when that is the file present (the file that already holds the entry wins) | `mcp.nativemem` = `{type: "local", command: [binary, "mcp", "serve", ...], enabled: true}`; `mcp.servers.nativemem` when the file keeps its servers under `mcp.servers` |
| Pi | `<pi home>/mcp.json` | `mcpServers.nativemem` = `{command, args}` |

- Edits keep every other byte: Codex with `toml_edit`, OpenCode with the `jsonc-parser` concrete syntax
  tree (comments and formatting stay), Claude and Pi with `serde_json` in file order and the file's
  indentation and trailing newline. A key of the entry the user added stays; `tool_timeout_sec` and `startup_timeout_sec` are set
  only when absent. Tests: `crates/hub/tests/register.rs`.
- Every file is read, transformed and written only when it is unchanged since the read; otherwise it is
  read again, up to 3 times, then the agent fails with code `busy` and nothing is written (Claude and
  Codex rewrite their files while running). The previous file is backed up first, the write is atomic.
  Tests: `claude_update_starts_over_when_the_file_changes_under_it`,
  `a_file_that_never_stops_changing_is_left_alone`.
- A second `install` changes nothing and takes no backup. A file that does not parse is never written:
  the agent's result has `status: "error"` and code `parse`. A missing file is created (`mcp.json`,
  `opencode.json`, `config.toml`, `.claude.json`); an agent whose home directory is missing is skipped as
  `not_installed`. The other agents are still handled; the exit code is 2 when any agent failed.
- Pi: an extension in `<pi home>/extensions/` (one level deep) or listed in `settings.json`
  `extensions` that registers `/mcp` replaces Pi's built-in MCP support and Pi stops reading
  `mcp.json`; `install` and `status` print a warning that names it (`pi_warnings`; test
  `pi_warns_when_an_extension_replaces_the_built_in_mcp_support`).
- `status` per agent: `registered`, `outdated` (the entry differs from what `install` would write now, for
  example after the binary moved), `missing`, `not_installed` or `error`.

## CLI

All commands take `--json` and use the error shape and codes of `docs/desktop.md`.

| Command | Does |
|---|---|
| `mcp serve` | Runs the hub on stdio |
| `mcp list` | Servers with transport, enabled, auth, `auth_state` and secret references |
| `mcp add` | Adds one server from a JSON document on stdin (`name` plus the fields of `hub.toml`); code `exists` for a duplicate name |
| `mcp edit <name>` | Replaces that server with the JSON document on stdin |
| `mcp remove <name>` | Removes the server; its secrets stay |
| `mcp enable <name>`, `mcp disable <name>` | Sets `enabled` |
| `mcp set-secret <id>` | Stores the value read from stdin (one trailing newline dropped) |
| `mcp delete-secret <id>` | Deletes it; code `not_found` when it does not exist |
| `mcp test <name>` | Connects, lists the tools, disconnects; exit code 2 when it failed (a server that needs a sign-in fails with the sign-in message) |
| `mcp login <name>` | Signs in to an `oauth` server in the browser ("OAuth") |
| `mcp logout <name>` | Deletes that server's tokens |
| `mcp install [--agent <id>]`, `mcp uninstall [--agent <id>]` | Registers the hub in the agents, or removes the entry ("Registration") |
| `mcp status [--agent <id>]` | Whether each agent has the entry |
| `mcp import [--apply]` | Reads the agents' MCP entries and prints the plan; `--apply` writes `hub.toml` and the secrets |

`nativemem subagent list|add|edit|remove|run` manages the subagent definitions ("Subagents"); the JSON
shapes are in `docs/desktop.md`. `add`, `edit` and `remove` run under the state lock
(`SubagentArgs::writes` in `crates/cli/src/subagent.rs`).

Every command except `serve`, `list`, `test`, `login`, `logout`, `status` and a dry-run `import` runs
under the state lock (`McpArgs::writes` in `crates/cli/src/mcp.rs`); `install` and `uninstall` do.

## Subagents

A subagent is a coding agent run headless with a chosen harness and model. It is defined once and every
agent that has the `nativemem` entry can call it. The hub offers it as a tool; it does not copy anything
into the agents.

Definitions live in `<state dir>/subagents.toml`, written by `SubagentConfig::save` with the same backup
(`<state dir>/backups/<stamp>/hub/subagents.toml`), atomic write and state lock as `hub.toml`
(`crates/hub/src/subagents.rs`; test `config_round_trips_and_backs_up_the_previous_file`).

```toml
[agents.code-review]
harness = "claude"             # "claude" | "codex" | "opencode" | "pi"
model = "sonnet"               # a model id in that harness's syntax
description = "Reviews a diff for bugs"   # when to use it; the tool description
instructions = "Be terse."     # optional, put in front of every prompt (blank line between)
timeout_secs = 900             # default 900, at least 1
args = []                      # extra harness flags, placed before the prompt
```

- A name is 1-48 characters of `a-z 0-9 -`. Unknown keys are a parse error. `model` is not empty, does
  not start with `-` and holds no control characters; `description` is not empty (`SubagentSpec::validate`).
- The harness commands (`build` in `crates/hub/src/harness.rs`; the page of each agent in `wiki/agents/`
  has the evidence level of its flags):

| Harness | Command | Prompt | Answer |
|---|---|---|---|
| `claude` | `claude -p --output-format json --model M [args]` | stdin | `result` of the JSON document; `is_error: true` is a failure |
| `codex` | `codex exec --model M --skip-git-repo-check [args] -o <file> -` | stdin | the file, in a scratch directory removed after the run |
| `opencode` | `opencode run --model M --format json [args] "<prompt>"` | last argument | the `text` events of the JSON lines, joined by a blank line |
| `pi` | `pi -p --model M [args] -- "<prompt>"` | last argument | stdout |

- The executable is looked up in the `PATH` of the process, then `~/.local/bin`, `/opt/homebrew/bin`,
  `/usr/local/bin` and `~/.pi/agent/bin`, with symlinks resolved (`search_path` and `locate`, on
  `resolve_executable` of `crates/core/src/probe.rs`). Agents start the hub with a minimal `PATH`, and the
  child gets that search path as its `PATH` too, so a harness that is a script finds its interpreter. A
  missing executable is an error result; `subagent add` and `edit` refuse it with code `not_installed`.
- Tools: `subagents_list` (no arguments; `{"subagents": [{name, tool, harness, model, description}]}`)
  and one `subagent_<name>` per definition with the input `{prompt: string, cwd?: string}`. The tool
  description is the definition's description plus ` (harness <h>, model <m>)`. The definitions are read
  when the hub starts (`run_stdio`); a `subagents.toml` that does not load is reported on stderr and
  leaves the proxied servers working. A missing `prompt` or a `cwd` that is not a string is an
  invalid-params error.
- A run starts in `cwd` (an absolute path of an existing directory; the hub process's directory when
  omitted) in its own process group. At most 4 runs go at once per hub process; the others wait for a slot,
  and the wait does not count against the timeout. At `timeout_secs` the whole process group is killed
  (`GroupGuard` in `crates/hub/src/runner.rs`; so it is when the call is cancelled).
- A subagent cannot start subagents. The child gets `NATIVEMEM_SUBAGENT_DEPTH` = the parent's value plus one
  (0 when unset); a run whose level would reach 2 is refused with an error result and nothing is started.
  The hub that a subagent's own agent starts therefore lists the tools and refuses to run them. A value that
  is not a number counts as the highest level.
- Result: the answer, cut at 50 KB (on a character boundary) with the note `[output cut at 50 KB; N bytes
  omitted]`. A failure is an error result with the exit code (or the timeout, or the signal) and the last
  2 KB of stderr; stdout and the environment never appear in it. A program the harness
  left running that still holds its output pipes is waited for 2 seconds after the harness exits, then
  the answer is read from what arrived.
- The harness runs with its own permissions and settings. `args` carries what it needs for that (for
  example a permission mode of the harness), because nothing here adds flags that loosen them.

Tests: `crates/hub/tests/subagents.rs` runs every harness against fake shell scripts in a temp directory
that is the only place binaries are searched in (`claude_gets_its_prompt_on_stdin_and_answers_in_the_result_field`,
`codex_gets_its_prompt_on_stdin_and_answers_in_the_file`, `opencode_gets_its_prompt_as_an_argument_and_answers_in_text_events`,
`pi_gets_its_prompt_as_an_argument_and_answers_in_plain_text`, `a_failed_run_reports_the_exit_code_and_the_end_of_stderr`,
`a_timeout_kills_the_harness_and_everything_it_started`, `a_subagent_at_the_depth_limit_cannot_start_another`,
`a_long_answer_is_cut_with_a_note`, `at_most_four_runs_go_at_once`, `the_hub_lists_and_calls_subagent_tools`);
`crates/cli/tests/subagent.rs` covers the commands and `mcp serve` as a child process. No real agent is
started.

## Desktop app

The "MCP & Integrations" screen of the app runs these commands with `--json` and holds no MCP logic
(`apps/macos/Sources/NativememKit/McpModel.swift`; screen described in `docs/desktop.md`).

- State of a server: `disabled` when it is not enabled; `needs sign-in` when `auth_state` is `needs_login`
  or the test fails with the `Sign in again` message; `error` for any other failed test; `connected`
  when the test passes (`McpServerState.derive`). The sidebar badge counts `needs sign-in` and `error`.
  `mcp test` runs in the background for each enabled server, except those with `needs_login`.
- `mcp list` never returns a secret value or a literal one, so the Edit sheet cannot show plain
  values: they are typed again, and a secret row with an empty value keeps its `secret:<id>`.
- A server name or secret id can begin with `-`, so the app passes it after `--`.
- The app stores secrets with `mcp set-secret <id>` (value on stdin) before `mcp add` / `mcp edit`.
- `mcp` documents are decoded without the snake-case key strategy, because env and header names are
  dictionary keys.

Tests: `McpContractTests`, `McpStateTests`, `McpDraftTests` in `apps/macos/Tests/NativememKitTests/`.

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
- `crates/hub/src/oauth.rs`
- `crates/hub/src/register.rs`
- `crates/hub/src/subagents.rs`
- `crates/hub/src/harness.rs`
- `crates/hub/src/runner.rs`
- `crates/hub/src/bin/fake_mcp.rs`
- `crates/hub/tests/hub.rs`
- `crates/hub/tests/import.rs`
- `crates/hub/tests/oauth.rs`
- `crates/hub/tests/register.rs`
- `crates/hub/tests/subagents.rs`
- `crates/hub/tests/common/fake_oauth.rs`
- `crates/cli/src/mcp.rs`
- `crates/cli/src/subagent.rs`
- `crates/cli/tests/mcp.rs`
- `crates/cli/tests/subagent.rs`
- `docs/desktop.md`
- `apps/macos/Sources/NativememKit/McpModel.swift`
- `apps/macos/Sources/NativememKit/McpContract.swift`

## Rules

- MUST: Never print, log or store a secret value outside the secret store; show `secret:<id>`.
- MUST: Write `hub.toml` only through `HubConfig::save` (backup, atomic write) under the state lock.
- MUST: Keep stdout of `mcp serve` for the protocol; diagnostics go to stderr.
- MUST: Run tests with `--secrets-dir` and temp roots; never the keychain, never a real agent home.
- MUST: Write an agent's config file only to add, update or remove the `nativemem` entry, through `crates/hub/src/register.rs`; import reads those files only.
- MUST: Hold `<state dir>/hub-oauth.lock` while tokens are refreshed, stored or deleted, and read the store again after taking it.
- MUST: Never print a token; scrub it from connection errors.
- MUST: Write `subagents.toml` only through `SubagentConfig::save` (backup, atomic write) under the state lock.
- MUST: Start a harness only from `crates/hub/src/runner.rs`, in its own process group, with the timeout, the slot limit and the depth guard.
- MUST: Test subagents only with fake harness scripts in a temp directory that is the whole search path; never let a test find a real agent binary.
- SHOULD: Put secrets behind `secret:<id>`; only env and header values are moved by import.

## Verification checklist

- [ ] Every repo path in backticks exists
- [ ] `cargo test --workspace` passes, including `crates/hub/tests` and `crates/cli/tests/mcp.rs`
- [ ] A new `mcp` command with JSON output is described in `docs/desktop.md`
- [ ] `crates/hub/tests/oauth.rs` and `crates/hub/tests/register.rs` pass without network or real agent homes
- [ ] `crates/hub/tests/subagents.rs` and `crates/cli/tests/subagent.rs` pass without a real agent binary
- [ ] No test or fixture contains a real token

## Related docs

- `wiki/architecture/overview.md`
- `wiki/architecture/safety.md`
- `wiki/architecture/file-structure.md`
- `wiki/decisions/mcp-hub.md`
- `wiki/guides/add-a-resource-kind.md`

## Last updated

2026-11-11
