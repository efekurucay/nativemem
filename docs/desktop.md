# nativemem desktop app (macOS)

## Intent

A small native Mac app for looking at and tidying agent memory, and for running the sync
without a terminal. It is a window onto the `nativemem` CLI: every action in the app runs a CLI
command and shows the result. The app holds no logic of its own about agents or sync.

## What the user does with it

1. See every memory item, grouped by project, and read it.
2. See where each item lives (Claude Code, Codex).
3. Delete an item that is stale or wrong. The next sync removes it from the other agent too.
4. Create a memory entry or a global instruction section in any agent. The next sync copies it to
   the other agents.
5. Read the chat sessions of a project folder, in any agent, as a transcript.
6. Press Sync: see the plan in plain words, then apply it.

## Look and feel

- Native macOS: SwiftUI, system fonts, system colours, light and dark, standard controls.
  No custom chrome, no gradients, no web view.
- Calm and sparse. One short plain sentence of explanation where a screen needs it. No jargon:
  say "copy" not "replica", "Preview changes" not "dry run".
- UI text in English.

## Layout

Sidebar plus a detail area, minimum window 900 x 560. The detail area shows either the Overview
on its own, or an item list beside an item detail pane. There is never an empty placeholder
column next to the Overview.

Spacing uses one scale everywhere (4, 8, 12, 16, 20). Lists, sidebar rows, search fields,
toolbar items and grouped boxes are the system's own controls with their default insets; no
hand-drawn borders, dividers or backgrounds.

**Sidebar** (`List` with the sidebar style, native `.searchable` filter in the sidebar)
- "Overview" row at the top, an ordinary list row.
- Section "Projects": a folder tree built from the project paths.
  - The home directory prefix is shown as `~`. The deepest folder that contains every project
    is not a level of the tree: its children are the top level. If that folder itself holds
    items it appears as the first row, labelled with its display path (for example `~`), as a
    leaf.
  - A folder with no items of its own and exactly one child folder is merged with that child
    (`acme/apps`).
  - Each row: folder icon, name, and a trailing count badge of all items in that folder and
    below. Siblings are sorted by that count, largest first, then by name.
  - Top-level folders start expanded; deeper ones start collapsed.
  - The filter keeps folders whose path contains the text, with their ancestors, expanded.
- A "Global" row above the tree when global items exist.
- A "Skills" row below it when skills exist, with a count badge. Skills are listed there only,
  not under "Global" or a project.

**Item list** (a folder is selected)
- Items of that folder and everything below it. When they come from more than one project, the
  list is sectioned by project, each header being the project's path relative to the selection.
- One row each: name (primary), description (secondary, two lines max), a small category tag
  when present (`feedback`, `project`, `reference`, `user`).
- A search field filters by name, description and body.
- Each logical item appears once even though it exists in several agents: rows are originals;
  copies are folded into their original.

**Sessions** (a folder is selected)
- Above the list, a segmented control switches "Memory" and "Sessions (N)"; N counts the
  sessions of that folder and below. It is shown for project folders only; every other row lists
  memory. Chat sessions never appear in the memory list.
- The Sessions list covers the folder and below, sectioned by project like memory, newest first
  inside each section. A search field filters by title.
- One row each: title (two lines max), relative date and turn count, and a small label per agent
  that holds the session. Copies are folded into their original like memory rows.
- The detail pane shows the title, "Lives in" (original / copy) and the transcript read-only,
  loaded with `session show`: each turn is labelled "You" or "Assistant" and rendered with the
  same block rendering as memory bodies. There is no Edit and no Delete (sessions are snapshots;
  `delete` and `edit` refuse them). "Show in Finder" appears for the agents that keep one file per
  session (Claude Code, Pi, Codex) and is hidden for OpenCode, whose sessions live in a database.

**Skills** ("Skills" row selected)
- One row each: name, description and a small label per agent that holds the skill.
- The detail pane shows the `SKILL.md` text read-only (loaded with `skill show`), "Lives in", and
  Delete (the skill directory is removed with a backup; the next sync propagates it). Edit is
  disabled with the tooltip "Skills are edited as files: edit the skill files directly."

**Detail column** (an item is selected)
- Name as title, description below, category tag.
- "Lives in": one small label per agent that has it (`Claude Code`, `Codex`, `OpenCode`, `Pi`), the origin marked
  "original", others "copy".
- Body as selectable text with simple block rendering: headings, bullet and numbered lists,
  fenced code in a monospaced font, and paragraphs. Lines inside a paragraph are joined, so
  text hard-wrapped in the file flows to the pane width. Inline bold, italic, code and links
  are rendered.
- "Edit" button. The pane switches to an edit form: name and description text fields, a
  category picker (none, user, feedback, project, reference) and a monospaced text editor for
  the body, with "Cancel" and "Save". Save runs the edit command on the original, then
  reloads. Under the form: "Changes reach the other agents at the next sync." Leaving the item
  with unsaved changes asks before discarding. Edit is disabled, with a tooltip saying why,
  when the original cannot be edited (the CLI answers `unsupported`).
- "Show in Finder" button for the original's file.
- "Delete…" button (destructive). Confirmation dialog: "Delete this memory? It will be removed
  from Claude Code now and from the other agents at the next sync. A backup is kept in
  ~/.nativemem/backups." On confirm run the delete command, then reload.

**Overview** (selected in sidebar)
- Three figures: items, projects, agents in sync (an agent is in sync when it holds every item).
- Per agent: originals, copies.
- "Skipped" disclosure list: label + reason, as given by the CLI.
- If the CLI binary cannot be found: a clear message and a "Choose…" button to pick it.

**Compatibility** (Overview, above the figures; only when an agent is not `ok`)
- `action_required`: a native box (`GroupBox`, system colours) per agent with its name, "Updated
  to X", the paused kinds, a "Failing checks" disclosure (summary, expected, observed per check)
  and two buttons. "Copy prompt" puts the `prompt` of `doctor --json` on the pasteboard.
  "Mark as reviewed" asks for confirmation, runs `doctor --accept <agent>` and reloads.
- `updated`: one secondary-text line, "Codex updated 0.167.0 → 0.170.0, all checks pass".
- The Overview row of the sidebar carries a badge with the number of agents that need action.
- `doctor --json` runs on load and on Reload; when it fails, the notices are left out.

**Toolbar**
- Every toolbar button has a tooltip. "Sync" shows its title next to its icon; "Reload" is
  icon-only with the tooltip "Reload".
- "New" button (plus icon, tooltip "New memory"). Opens a sheet; disabled while an item is being
  edited. Fields:
  - "Where": Global, or Project. Project shows a picker of the known project paths plus "Choose
    folder…", which opens an `NSOpenPanel` for a directory. The sheet starts on the project of the
    selected sidebar folder, else on Global.
  - "Agent": the installed agents (those in `status`); the default is the agent with the most
    originals, the first on a tie.
  - Project: name, description, a category picker (none, user, feedback, project, reference) and a
    monospaced body editor. Create runs `add --kind memory`.
  - Global: a heading field and a monospaced body editor. Create runs `add --kind instruction`.
  - Footer: "It reaches the other agents at the next sync." Buttons "Cancel" and "Create" (disabled
    until the required fields are filled). Create reloads and selects the new item; a CLI error is
    shown inline in the sheet and nothing is closed.
- "Sync" button. Runs the preview command and opens a sheet:
  - headline: "12 to add, 3 to update, 1 to remove" or "Everything is in sync."
  - list of changes grouped by project; each line: sign (add / update / remove), item name,
    target agent.
  - conflicts and skipped items in a separate section with the CLI's message.
  - buttons: "Cancel", "Apply" (disabled when there is nothing to do). If the CLI refuses with
    the mass-delete error, show its message and an "Apply anyway" button that re-runs with
    `--allow-mass-delete`, behind a second confirmation.
  - after Apply: show the result summary, then reload everything.
- "Reload" button.

## CLI contract

The app finds the binary in this order: path saved in app settings, `NATIVEMEM_BIN` environment
variable, a `nativemem` file next to the app executable, `~/.cargo/bin/nativemem`,
`/usr/local/bin/nativemem`, `/opt/homebrew/bin/nativemem`.

All commands print one JSON document on stdout when `--json` is given. On failure: non-zero
exit and `{"error": "<message>", "code": "<snake_case code>"}` on stdout. Exit codes: 0 ok,
2 ok with conflicts or skipped writes remaining, 3 `doctor` found an agent that needs action,
1 error.

`nativemem status --json`
```json
{ "agents": [
    { "id": "claude", "originals": 399, "replicas": 0, "sessions": 0, "automated_sessions": 0,
      "skipped": [ { "label": "-Users-x-old", "reason": "no cwd in jsonl ..." } ] },
    { "id": "codex", "originals": 0, "replicas": 399, "sessions": 0, "automated_sessions": 0,
      "skipped": [] } ],
  "ledger_items": 399 }
```

`status --json` also carries `"compat": { "action_required": 0, "updated": 0, "agents": [ { "id":
"claude", "status": "ok" } ] }`; the app ignores it.

`nativemem doctor --json` (exit 0, or 3 while an agent needs action; the document is complete
either way)
```json
{ "agents": [
    { "id": "codex", "installed": true, "version": "0.170.0", "previous_version": "0.167.0",
      "version_changed_ms": 1794304800000, "status": "action_required", "paused": ["session"],
      "notices": [],
      "checks": [
        { "id": "codex.threads_columns", "feature_kinds": ["session"],
          "summary": "threads table column signature (name, type, not null, default)",
          "expected": "43 columns, sha256 aaaaaaaaaaaa",
          "observed": "44 columns, sha256 bbbbbbbbbbbb; added: brand_new; removed or changed: none",
          "ok": false, "accepted": false,
          "sources": ["crates/core/src/adapters/codex_probes.rs"] } ],
      "prompt": "You are working in the nativemem repository at …" },
    { "id": "pi", "installed": false, "version": null, "previous_version": null,
      "version_changed_ms": null, "status": "not_installed", "paused": [], "checks": [] } ] }
```
One entry per agent in the order claude, codex, opencode, pi (`--agent <id>` keeps one).
`status` is `ok`, `updated`, `action_required` or `not_installed`; `paused` lists the kinds
(`memory`, `instruction`, `session`, `skill`) that failing checks stop. `prompt` is present only for
`action_required`. `notices` are informational lines (the app shows them as secondary text in the
agent's Overview box; they name skills whose own copy differs from a copy the agent also reads elsewhere, and sessions other agents hold that Claude Code would delete under its default retention); they never change `status`. `doctor --accept <agent> --json` records the failing checks' observed values
as reviewed, returns the document after that and adds
`"accepted": [ { "agent": "codex", "check": "codex.threads_columns", "observed": "…" } ]`.
An agent without a home directory fails with code `not_installed`.

`nativemem items --json`
```json
{ "items": [
    { "agent": "claude", "native_id": "-Users-x-app/feedback_tests.md", "kind": "memory",
      "path": "/Users/x/.claude/projects/-Users-x-app/memory/feedback_tests.md",
      "scope": { "type": "project", "path": "/Users/x/app" },
      "name": "feedback-tests", "description": "Run the fast suite first",
      "category": "feedback", "body": "…markdown…",
      "replica_of": null, "hash": "ab12…" },
    { "agent": "codex", "native_id": "app-1a2b3c4d/feedback-tests.md", "kind": "memory",
      "path": "/Users/x/.codex/memories/extensions/nativemem/resources/app-1a2b3c4d/feedback-tests.md",
      "scope": { "type": "project", "path": "/Users/x/app" },
      "name": "feedback-tests", "description": "Run the fast suite first",
      "category": "feedback", "body": "…markdown…",
      "replica_of": { "origin": "claude", "native_id": "-Users-x-app/feedback_tests.md" },
      "hash": "ab12…" } ] }
```
`kind` is `memory`, `instruction`, `skill` or `session`. A skill item is one directory with a top-level
`SKILL.md` in an agent's own skills directory: `native_id` is `skill:<directory name>`, `path` is
the skill directory, `scope` is global, `name` is the directory name, `description` is the
frontmatter `description` (empty when it cannot be read), `category` is `null` and `body` is always
`""` (file contents are never listed). The copies of one skill in several agents are linked
through `replica_of` the same way as memory; the origin is the agent the ledger records, or the
first of Claude Code, Codex, OpenCode, Pi that holds it. The skills list is also available with
`items --kind skill --json`. An instruction item is one `## ` section of a global
instruction file (`CLAUDE.md`, `AGENTS.md`): `native_id` is `<file>#<slug>`, `scope` is global,
`description` is the heading text and `category` is `null`.
`scope` is `{ "type": "global" }` for global items. `category` may be `null`.

Sessions are listed only with `items --kind session --json`. A session item has `native_id` ending
in `.jsonl`, `path` the file (OpenCode: the database), `scope` the project directory, `name` the
session's own id, `description` its title, `category` `null`, `body` always `""` (transcripts are
never listed), and an extra object, present on sessions only:
```json
{ "agent": "pi", "native_id": "--Users-x-app--/2026-11-10T10-00-00-000Z_0198a1b3.jsonl", "kind": "session",
  "path": "/Users/x/.pi/agent/sessions/--Users-x-app--/2026-11-10T10-00-00-000Z_0198a1b3.jsonl",
  "scope": { "type": "project", "path": "/Users/x/app" },
  "name": "0198a1b3", "description": "Fix login", "category": null, "body": "",
  "replica_of": { "origin": "claude", "native_id": "-Users-x-app/0198a1b2.jsonl" }, "hash": "ef56…",
  "session": { "created_ms": 1794304800000, "updated_ms": 1794308400000, "turns": 12 } }
```
`turns` is the number of turns.

`nativemem session show --agent <id> --id <native_id> --json`
```json
{ "agent": "claude", "native_id": "-Users-x-app/0198a1b2.jsonl",
  "title": "Fix login", "cwd": "/Users/x/app",
  "created_ms": 1794304800000, "updated_ms": 1794308400000,
  "turns": [ { "role": "user", "text": "how do I fix the login?", "ts_ms": 1794304800000 },
             { "role": "assistant", "text": "…", "ts_ms": 1794304860000 } ] }
```
Reads one session from that agent's own store; it takes no lock and writes nothing. `role` is
`user` or `assistant`; tool activity is already rendered into assistant text. Failures:
`unknown_agent`, `invalid` when the id is not a session id (it does not end in `.jsonl`),
`not_found` when the agent is not installed or holds no such session. Without `--json` it prints
the title and a readable transcript.

`nativemem skill show --agent <id> --name <skill> --json`
```json
{ "agent": "claude", "name": "demo-skill",
  "path": "/Users/x/.claude/skills/demo-skill/SKILL.md",
  "text": "---\nname: demo-skill\ndescription: Does demo-skill\n---\n…" }
```
Reads the `SKILL.md` of that skill in that agent's own skills directory. A skill the agent does
not hold fails with code `not_found`. `status --json` carries `"skills": <count>` per agent beside
`sessions`; skills are not counted in `originals` and `replicas`.

`nativemem sync --json` (preview) and `nativemem sync --apply --json`
```json
{ "applied": false,
  "actions": [
    { "op": "add", "agent": "codex", "name": "feedback-tests",
      "scope": { "type": "project", "path": "/Users/x/app" },
      "origin": { "origin": "claude", "native_id": "-Users-x-app/feedback_tests.md" } } ],
  "notes": [ { "kind": "conflict", "message": "…" } ],
  "summary": { "add": 1, "update": 0, "remove": 0, "conflicts": 0, "skipped": 0 } }
```
`op` is one of `add`, `update`, `remove`. `kind` is one of `conflict`, `skipped`.
With `--apply`, `applied` is true and `summary` counts what was done. Error code for the
mass-delete guard: `mass_delete`.

`nativemem delete --agent <id> --id <native_id> --json`
```json
{ "deleted": { "agent": "claude", "native_id": "-Users-x-app/feedback_tests.md" },
  "backup_dir": "/Users/x/.nativemem/backups/20261110T062319Z" }
```
Removes that one entry (with backup). It does not sync; the next sync propagates the removal.

`nativemem add --agent <id> --kind memory|instruction [--project <abs path>] --json`, with the
content as one JSON document on stdin (the same document `edit` reads):
```json
{ "name": "feedback-tests", "description": "Run the fast suite first",
  "category": "feedback", "body": "…markdown…" }
```
Output:
```json
{ "agent": "pi", "native_id": "app-1a2b3c4d/feedback-tests.md",
  "path": "/Users/x/.pi/agent/nativemem/memory/app-1a2b3c4d/feedback-tests.md",
  "hash": "ef56…", "backup_dir": "/Users/x/.nativemem/backups/20261110T062319Z" }
```
Creates a new original in that agent's own store; it does not sync, and the next sync copies it to
the other agents. `--kind memory` needs `--project`, an absolute path without `..` (a trailing slash
is dropped); `name`, `description` and `category` (an empty one means none) must be single lines
and `name`, `description` non-empty. `--kind instruction` takes no `--project`: `description` is the
heading of a new `## ` section in the agent's global instruction file, `name` is derived from it and
`category` is ignored; a body that would start another `## ` section outside a code fence fails.
Failures: `invalid` for bad input, `exists` when the entry's file or section id is already there,
`not_installed` when the agent has no home directory, `unsupported` when the agent cannot hold the
kind, `unknown_agent`, `parse` for a bad stdin document.

`nativemem edit --agent <id> --id <native_id> --json`, with the new content as one JSON
document on stdin:
```json
{ "name": "feedback-tests", "description": "Run the fast suite first",
  "category": "feedback", "body": "…markdown…" }
```
`category` may be `null`. Output:
```json
{ "edited": { "agent": "claude", "native_id": "-Users-x-app/feedback_tests.md" },
  "hash": "cd34…", "backup_dir": "/Users/x/.nativemem/backups/20261110T062319Z" }
```
Rewrites that original entry in place (with backup). It does not sync; the next sync carries
the change to the copies. Editing an entry the agent regenerates itself fails with code
`unsupported`; so does editing a skill (skills are edited as files in their directory). For an
instruction item only `body` changes; if `name` or `description` differs
from the current values the command writes nothing and fails with code `invalid`.

### MCP hub

`nativemem mcp ...` manages the hub: one local MCP server (`nativemem mcp serve`, stdio, no JSON
document) that holds the real MCP servers and their secrets (`wiki/architecture/hub.md`). Besides
the error codes above, `exists` is returned when a server name is already taken; `invalid` covers a
bad server spec, name or secret id; `not_found` covers an unknown server or secret. A secret value is
never printed: `secret:<id>` references are shown, and a literal env or header value is shown as
`"<literal>"`. OAuth tokens are never printed either.

`nativemem mcp list --json`
```json
{ "servers": [
    { "name": "github", "transport": "stdio", "enabled": true, "auth": "none",
      "auth_state": "none", "command": "npx", "args": ["-y", "pkg"], "cwd": "/Users/x/app",
      "env": { "GITHUB_TOKEN": "secret:github-GITHUB_TOKEN", "LOG": "<literal>" },
      "headers": {}, "secrets": ["github-GITHUB_TOKEN"] },
    { "name": "docs", "transport": "http", "enabled": false, "auth": "oauth",
      "auth_state": "needs_login",
      "url": "https://mcp.example.com/mcp", "args": [], "env": {},
      "headers": { "X-Api-Key": "secret:docs-X-Api-Key" },
      "secrets": ["docs-X-Api-Key"] } ] }
```
`command` and `cwd` appear for stdio servers, `url` for http servers. `transport` is `stdio` or `http`,
`auth` is `none` or `oauth`. `auth_state` is `none` for a server without OAuth, `signed_in` when an
`oauth` server has stored tokens and `needs_login` when it has none. An `oauth` server takes no
`Authorization` header (`invalid`).

`nativemem mcp add --json`, with one server as a JSON document on stdin:
```json
{ "name": "github", "transport": "stdio", "command": "npx", "args": ["-y", "pkg"],
  "env": { "GITHUB_TOKEN": "secret:github-GITHUB_TOKEN" }, "cwd": "/Users/x/app",
  "auth": "none", "enabled": true }
```
or `{"name", "transport": "http", "url", "headers", "auth", "enabled"}`. `auth` defaults to `none` and
`enabled` to `true`. Output `{ "added": "github" }`. A duplicate name fails with code `exists`.

`nativemem mcp edit <name> --json` replaces the server with the document on stdin (the same fields;
`name`, when present, must equal `<name>`). Output `{ "edited": "github" }`.

`nativemem mcp remove <name> --json` prints `{ "removed": "github" }`; its secrets are kept.
`nativemem mcp enable <name> --json` and `disable <name>` print `{ "name": "github", "enabled": true }`.

`nativemem mcp set-secret <id> --json` reads the value from stdin (one trailing newline is dropped; an
empty value fails with code `invalid`) and prints `{ "secret": "<id>" }`.
`nativemem mcp delete-secret <id> --json` prints `{ "deleted": "<id>" }`; an unknown id fails with
code `not_found`.

`nativemem mcp test <name> --json`
```json
{ "ok": true, "tools": ["echo", "env_matches"], "error": null }
```
`tools` are the server's own tool names. On a connection failure `ok` is false, `tools` is empty,
`error` is a message without secret values or tokens, and the exit code is 2. An `oauth` server without a
usable sign-in fails with `error` `Sign in again: nativemem mcp login <name>`.

`nativemem mcp login <name> --json` signs in to an `oauth` server: it opens the authorization page in the
browser (the URL is also printed to stderr), waits up to 5 minutes for the redirect to
`http://127.0.0.1:<port>/callback`, stores the registered client and the tokens in the secret store, and
prints `{ "signed_in": "<name>" }`. A server without `auth` `oauth` fails with code `invalid`; so does a
sign-in that is refused, times out or cannot reach the server. It does not take the state lock.
`nativemem mcp logout <name> --json` deletes the tokens, keeps the registered client and prints
`{ "signed_out": "<name>", "was_signed_in": true }`.

`nativemem mcp install --json`, `nativemem mcp uninstall --json` and `nativemem mcp status --json`, each
with an optional `--agent claude|codex|opencode|pi`:
```json
{ "command": "/Users/x/.cargo/bin/nativemem",
  "args": ["mcp", "serve", "--state-dir", "/Users/x/state"],
  "agents": [
    { "agent": "claude", "status": "installed", "path": "/Users/x/.claude.json", "warnings": [] },
    { "agent": "codex", "status": "error", "path": "/Users/x/.codex/config.toml", "warnings": [],
      "code": "parse", "message": "cannot parse /Users/x/.codex/config.toml: ..." },
    { "agent": "opencode", "status": "not_installed", "path": null, "warnings": [] },
    { "agent": "pi", "status": "installed", "path": "/Users/x/.pi/agent/mcp.json",
      "warnings": ["the Pi extension /Users/x/.pi/agent/extensions/mcp.ts registers /mcp and replaces Pi's built-in MCP support: Pi does not read mcp.json while it is installed"] } ] }
```
`install` registers one entry named `nativemem` in each agent's own MCP configuration; `uninstall`
removes it. `command` is the absolute path of the running binary and `args` start with `mcp serve`, then
`--claude-home`, `--codex-home`, `--opencode-home`, `--pi-home` and `--state-dir` (absolute paths) for the
roots that were given explicitly. `command` and `args` are present for `install` and `status`, not for
`uninstall`. `status` per agent is `installed` or `unchanged` (install), `removed` or `absent`
(uninstall), `registered`, `outdated` (the entry differs from what `install` would write now) or
`missing` (status), `not_installed` (the agent's home directory does not exist; nothing is written) or
`error` (with `code` and `message`: `parse` for a file that does not parse, which is never written, `busy`
when the file kept changing, `io`). `path` is the file holding or receiving the entry. An unknown
`--agent` fails with code `unknown_agent`. The exit code is 2 when any agent has `error`. `install` and
`uninstall` take the state lock and back up each file they replace.

`nativemem mcp import --json` (preview) and `nativemem mcp import --apply --json`
```json
{ "applied": false,
  "servers": [
    { "name": "shared", "transport": "stdio", "status": "add",
      "agents": ["claude", "codex", "pi"], "secrets": ["shared-API_KEY"] },
    { "name": "clash", "transport": "stdio", "status": "conflict", "agents": ["claude", "codex"],
      "secrets": [] } ],
  "skipped": [ { "agent": "claude", "name": "sse-server", "reason": "sse transport is not supported by the hub" } ],
  "warnings": ["codex shared: keys not carried over: startup_timeout_sec"],
  "summary": { "add": 1, "present": 0, "conflicts": 1, "skipped": 1 } }
```
`status` is `add`, `present` (the hub already holds this exact entry) or `conflict` (the name has a
different config in another agent or in the hub; `agents` then includes `hub`). Nothing is imported
for a conflict. The agents' MCP files are only read. With `--apply`, `applied` is true, the `add`
entries are written to `hub.toml` and their env and header values are stored as the listed secrets.
Exit code 2 when a conflict remains.

## App structure

Swift package at `apps/macos` (SwiftUI app, macOS 14+, Swift 6, no third-party dependencies).

- `NativememCLI`: locates the binary, runs a command with `Process`, decodes JSON into
  `Codable` types that mirror the contract above. All calls are `async` and off the main actor.
- `AppModel` (`@Observable`): loaded items, status, selection, loading and error state.
  Groups items by project, folds copies into originals.
- `SessionIndex` (from `items --kind session --json`) folds copies into originals and sections the
  sessions of a folder like memory; `AppModel.itemMode` and `showsSessions` choose what the list
  shows; `SessionDetailView` loads the transcript with `session show`.
- `DoctorDocument` (decoded from `doctor --json`) feeds the Overview notices and the sidebar badge.
- `NewEntryDraft` (what the New sheet collects: validation and the `add` document) and
  `AppModel.add`, which runs `add`, reloads and selects the new item.
- Views: `SidebarView`, `ItemListView`, `ItemDetailView`, `SessionDetailView`, `OverviewView`, `SyncSheet`, `NewEntrySheet`.
- `scripts/bundle.sh`: builds release and assembles `build/nativemem.app` (Info.plist, the
  executable, and the `nativemem` CLI binary copied next to it when
  `../../target/release/nativemem` exists).
- Tests: decoding of each contract document from fixture JSON; grouping and folding logic; the New
  entry draft.
