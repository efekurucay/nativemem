# nativemem desktop app (macOS)

## Intent

A small native Mac app for looking at and tidying agent memory, and for running the sync
without a terminal. It is a window onto the `nativemem` CLI: every action in the app runs a CLI
command and shows the result. The app holds no logic of its own about agents or sync.

## What the user does with it

1. See every memory item, grouped by project, and read it.
2. See where each item lives (Claude Code, Codex).
3. Delete an item that is stale or wrong. The next sync removes it from the other agent too.
4. Press Sync: see the plan in plain words, then apply it.

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

**Item list** (a folder is selected)
- Items of that folder and everything below it. When they come from more than one project, the
  list is sectioned by project, each header being the project's path relative to the selection.
- One row each: name (primary), description (secondary, two lines max), a small category tag
  when present (`feedback`, `project`, `reference`, `user`).
- A search field filters by name, description and body.
- Each logical item appears once even though it exists in two agents: rows are originals;
  copies are folded into their original.

**Detail column** (an item is selected)
- Name as title, description below, category tag.
- "Lives in": one small label per agent that has it (`Claude Code`, `Codex`), the origin marked
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
- Three figures: items, projects, agents in sync.
- Per agent: originals, copies.
- "Skipped" disclosure list: label + reason, as given by the CLI.
- If the CLI binary cannot be found: a clear message and a "Choose…" button to pick it.

**Toolbar**
- Every toolbar button has a tooltip. "Sync" shows its title next to its icon; "Reload" is
  icon-only with the tooltip "Reload".
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
2 ok with conflicts or skipped writes remaining, 1 error.

`nativemem status --json`
```json
{ "agents": [
    { "id": "claude", "originals": 399, "replicas": 0,
      "skipped": [ { "label": "-Users-x-old", "reason": "no cwd in jsonl ..." } ] },
    { "id": "codex", "originals": 0, "replicas": 399, "skipped": [] } ],
  "ledger_items": 399 }
```

`nativemem items --json`
```json
{ "items": [
    { "agent": "claude", "native_id": "-Users-x-app/feedback_tests.md",
      "path": "/Users/x/.claude/projects/-Users-x-app/memory/feedback_tests.md",
      "scope": { "type": "project", "path": "/Users/x/app" },
      "name": "feedback-tests", "description": "Run the fast suite first",
      "category": "feedback", "body": "…markdown…",
      "replica_of": null, "hash": "ab12…" },
    { "agent": "codex", "native_id": "app-1a2b3c4d/feedback-tests.md",
      "path": "/Users/x/.codex/memories/extensions/nativemem/resources/app-1a2b3c4d/feedback-tests.md",
      "scope": { "type": "project", "path": "/Users/x/app" },
      "name": "feedback-tests", "description": "Run the fast suite first",
      "category": "feedback", "body": "…markdown…",
      "replica_of": { "origin": "claude", "native_id": "-Users-x-app/feedback_tests.md" },
      "hash": "ab12…" } ] }
```
`scope` is `{ "type": "global" }` for global items. `category` may be `null`.

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
`op` is one of `add`, `update`, `remove`. `kind` is one of `conflict`, `skipped`, `orphan`.
With `--apply`, `applied` is true and `summary` counts what was done. Error code for the
mass-delete guard: `mass_delete`.

`nativemem delete --agent <id> --id <native_id> --json`
```json
{ "deleted": { "agent": "claude", "native_id": "-Users-x-app/feedback_tests.md" },
  "backup_dir": "/Users/x/.nativemem/backups/20261110T062319Z" }
```
Removes that one entry (with backup). It does not sync; the next sync propagates the removal.

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
`unsupported`.

## App structure

Swift package at `apps/macos` (SwiftUI app, macOS 14+, Swift 6, no third-party dependencies).

- `NativememCLI`: locates the binary, runs a command with `Process`, decodes JSON into
  `Codable` types that mirror the contract above. All calls are `async` and off the main actor.
- `AppModel` (`@Observable`): loaded items, status, selection, loading and error state.
  Groups items by project, folds copies into originals.
- Views: `SidebarView`, `ItemListView`, `ItemDetailView`, `OverviewView`, `SyncSheet`.
- `scripts/bundle.sh`: builds release and assembles `build/nativemem.app` (Info.plist, the
  executable, and the `nativemem` CLI binary copied next to it when
  `../../target/release/nativemem` exists).
- Tests: decoding of each contract document from fixture JSON; grouping and folding logic.
