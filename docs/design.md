# nativemem — design and build plan

## Intent

People who use several AI coding agents re-teach each one the same things. `nativemem` keeps
each agent's own native memory in sync with the others. There is no shared memory store: every
agent keeps reading its own files, in its own format, from its own home directory. The tool is
a background translator between them.

The long-term goal is full sync of every resource kind (memory, instruction files, MCP config,
skills, chat sessions) across many agents. This document covers the first release.

## First release scope

- Agents: Claude Code, Codex CLI, OpenCode and Pi.
- Resource kinds: learned memory items and global instruction sections.
- Behaviour: faithful replication. Adds, edits and deletes propagate in both directions.
  Text is never rewritten and scope (which project an item belongs to) is preserved.
- Deterministic. No LLM calls.
- Rust workspace: `core` library + `cli` binary. Runs on macOS, Linux, Windows.

## Non-goals for this release

Classification, promoting project items to global, summarising, stale cleanup, extracting
memory from session transcripts, desktop UI, other agents, other resource kinds.

## Anti-goals

- Writing anything inside a user's git repository.
- Touching agent-owned state: Codex SQLite files, `hooks.json`, `config.toml`, generated
  `MEMORY.md` / `memory_summary.md`; any file holding secrets.
- Deleting a file outright. Every delete or overwrite first copies the old file to the backup dir.
- Keeping item text in the tool's own state. The ledger stores ids, hashes and paths only.

## Architecture

```
crates/core   model, Adapter trait, ledger, sync engine, fs safety helpers
crates/cli    `nativemem` binary: status, sync
```

The engine knows nothing about agents. Each agent is an `Adapter`. Adding an agent or a
resource kind later means adding an adapter or a `Kind` variant; the engine does not change.

### Model (`core::model`)

```rust
pub struct AgentId(pub String);              // "claude", "codex", "opencode", "pi"
pub enum Scope { Global, Project(PathBuf) }  // Project = absolute working directory
pub enum Kind { Memory, Instruction }        // more kinds later

pub struct Item {
    pub kind: Kind,
    pub scope: Scope,
    pub name: String,          // short slug
    pub description: String,   // one line
    pub category: Option<String>, // user | feedback | project | reference
    pub body: String,          // markdown, verbatim
}
pub struct ItemKey { pub origin: AgentId, pub native_id: String }

/// One entry found on disk in an agent's native store.
pub struct Observed {
    pub agent: AgentId,
    pub native_id: String,           // stable id inside that agent
    pub path: PathBuf,
    pub item: Item,
    pub replica_of: Option<ItemKey>, // set when the entry carries a nativemem marker
}

pub trait Adapter {
    fn id(&self) -> AgentId;
    fn scan(&self) -> Result<Vec<Observed>>;
    /// Create or update the replica of `key` in this agent. Returns its native_id and path.
    fn write_replica(&self, key: &ItemKey, item: &Item) -> Result<(String, PathBuf)>;
    /// Update an entry that originated in this agent (an edit made on a replica elsewhere).
    fn write_original(&self, native_id: &str, item: &Item) -> Result<()>;
    /// Remove an entry (original or replica). Must back up first.
    fn remove(&self, native_id: &str) -> Result<()>;
}
```

`content_hash(item)` = SHA-256 over `name`, `description`, `category`, `body` after normalising
line endings to `\n` and trimming trailing whitespace. Scope is part of identity, not of the hash.

### Ledger (`core::ledger`, SQLite via `rusqlite` with the `bundled` feature)

Location: `<state_dir>/ledger.db`, default state dir `~/.nativemem`.

```
items(origin, native_id, kind, scope, hash, PRIMARY KEY(origin, native_id))
replicas(origin, native_id, agent, replica_native_id, path, hash,
         PRIMARY KEY(origin, native_id, agent))
```

`hash` is the content hash at the last successful sync.

### Sync engine (`core::engine`)

`plan(adapters, ledger) -> Plan` is pure: it scans, compares with the ledger and returns a list
of actions. `apply(plan)` executes them and updates the ledger. `sync --dry-run` prints the plan.

Per item, comparing current state with the ledger:

| Original | Replica in agent B | Action |
|---|---|---|
| new, not in ledger | — | write replica to every other agent, record |
| changed | unchanged | update replicas |
| unchanged | changed | write the edit back to the original and to other replicas |
| changed | changed | conflict: do nothing, report |
| missing | present | remove replicas, drop ledger rows |
| unchanged | missing | remove the original and other replicas, drop ledger rows |
| unchanged | never written (scope unsupported) | skip, report |

An entry observed with a marker but unknown to the ledger is adopted as a replica (ledger
rebuild), never treated as a new original.

Safety rules:
- Mass-delete guard: if a plan removes more than 10 entries, or more than 3 entries
  and more than 20% of ledger items, `apply` refuses unless `allow_mass_delete` is set.
- If an adapter's `scan` fails, the run aborts before planning. A failed scan must never look
  like "everything was deleted".
- Writes are atomic: write a temp file in the same directory, fsync, rename.
- Backups go to `<state_dir>/backups/<UTC timestamp>/<agent>/<relative path>`.

### Claude Code adapter (`core::adapters::claude`)

Root: `claude_home` (default `~/.claude`).

- Store: `projects/<dir>/memory/`. Each `*.md` except `MEMORY.md` is one item. Other files are ignored.
- Project path: the directory name is a lossy encoding, so read the real path from the `cwd`
  field of the first line that has one in any `projects/<dir>/*.jsonl`. No jsonl, or no `cwd`:
  skip the directory and report it.
- Frontmatter, two variants, both must parse:
  - nested: `name`, `description`, `metadata: { type, node_type, originSessionId, modified }`
  - flat: `name`, `description`, `type`, `originSessionId`
- `native_id` = `<dir>/<filename>`. `category` = `metadata.type` or `type`.
- Replica marker: `metadata.nativemem_origin: "<agent>:<native_id>"`.
- `write_replica`: file `<slug>.md` in the target project's memory dir with nested frontmatter
  (`name`, `description`, `metadata.type`, `metadata.nativemem_origin`), then body. Ensure
  `MEMORY.md` has the line `- [<name>](<file>) — <description>`; create the file with
  `# Memory Index` if absent. To find the target dir, match `Scope::Project(path)` against the
  cwd of existing project dirs; if none exists, create `projects/<encoded>/memory/` where
  encoded = path with every non-alphanumeric char replaced by `-`.
- `write_original`: replace `name`, `description`, body; keep every other frontmatter key as is.
  Update the index line.
- `remove`: back up and delete the file, remove its index line.
- `Scope::Global` is unsupported in this release (returns a typed `Unsupported` error).

### Codex adapter (`core::adapters::codex`)

Root: `codex_home` (default `~/.codex`, honour `CODEX_HOME`).

- Replica store: `memories/extensions/nativemem/resources/<project-key>/`
  - `scope.json`: `{"cwd": "<absolute path>"}`
  - `MEMORY.md`: `# Memory Index` + one line per item, same shape as Claude's
  - `<slug>.md`: first line `<!-- nativemem origin=<agent> id=<native_id> -->`, then
    `# <name>`, blank, `<description>`, blank, `Category: <category>` when present, blank, body
  - `project-key` = last path component, plus `-<8 hex of sha256(path)>`
- Routing block in `AGENTS.md` (global), between `<!-- nativemem:begin -->` and
  `<!-- nativemem:end -->`. Content outside the markers is never changed. The block says:
  when working in one of the listed directories, read that project's memory index first and
  open the item files it lists when relevant. One line per project: `- <cwd> → <index path>`.
  Regenerated on every apply; removed when no project remains.
- Originals: Codex-generated `memories/MEMORY.md`. Parse blocks that start with
  `# Task Group: <title>` and contain a line `applies_to: cwd=<path>; ...`. Each block is one
  item: `native_id` = `<cwd>#<slug(title)>`, scope `Project(cwd)`, `name` = slug(title),
  `description` = title, body = the block text. Blocks or content without `applies_to` are
  reported as unscoped and skipped.
- `write_original` / `remove` on a Codex original: unsupported in this release (generated
  state). The engine reports these as "cannot write back to codex" and leaves both sides alone.
- The `extensions/.../resources/<project>/scope.json` layout follows what Codex's own memory
  prompt describes; it is not officially documented. Loading is guaranteed by the `AGENTS.md`
  routing block, not by that layout. Keep the layout in one small module so it is easy to change.

### CLI (`crates/cli`, `clap`)

- `nativemem status` — per agent: items found, replicas, skipped (with reasons).
- `nativemem sync` — dry run: prints the plan. `nativemem sync --apply` executes it.
  `--allow-mass-delete` lifts the guard.
- Global flags / env for roots: `--claude-home`, `--codex-home`, `--opencode-home`, `--pi-home`,
  `--state-dir` (`NATIVEMEM_CLAUDE_HOME`, `NATIVEMEM_CODEX_HOME`, `NATIVEMEM_OPENCODE_HOME`,
  `NATIVEMEM_PI_HOME`, `NATIVEMEM_STATE_DIR`).
- Exit code 0 on success, 2 when conflicts or skipped writes remain, 1 on error.

## Testing rules (hard requirement)

Tests and manual runs during development use temp directories only (`tempfile`). Nothing in the
build may read or write the real `~/.claude`, `~/.codex` or `~/.nativemem`. Adapters take their
root as a constructor argument; only the CLI resolves defaults.

## Build tasks

1. **Foundation.** Workspace, `core::model`, `content_hash`, `Adapter` trait, `core::fsx`
   (atomic write, backup), `core::ledger`, and `core::testkit` (builders that create a fake
   Claude home and a fake Codex home in a temp dir). Unit tests for hash, ledger, fsx.
2. **Claude adapter** with tests on fixtures: both frontmatter variants, cwd resolution,
   replica write + index line, write_original preserving unknown keys, remove with backup.
3. **Codex adapter** with tests: replica store layout, routing block insert / update / removal
   without touching surrounding text, MEMORY.md task-group parsing, unsupported operations.
4. **Engine + CLI.** `plan` / `apply` for every row of the table, both safety guards, ledger
   adoption of marked entries, and an end-to-end test over fake homes: add, edit on each side,
   delete on each side, conflict. CLI commands and exit codes.

Tasks 2 and 3 are independent and touch different files.

## Left to the builder

Error type design, module-internal structure, slug function details, output formatting.

## Instruction sections (global scope)

Capabilities: `Adapter::supports(kind, scope)` says what an agent can store (default: everything).
Claude and Codex store memory in `Project` scope and instructions in `Global` scope; OpenCode and
Pi store global instructions only. The engine leaves agents that do not support an item out of its
replica targets, write-back targets, absent sets and adoption, so they never produce actions,
skipped notes or counts for it. `Error::Unsupported` remains a runtime fallback.

`Kind::Instruction`. One item is one top-level section of a global instruction file: the text from a
`## ` heading through the line before the next `## ` heading. Files: Claude `<claude_home>/CLAUDE.md`,
Codex `<codex_home>/AGENTS.md`, OpenCode `<opencode_home>/AGENTS.md` (default `~/.config/opencode`,
honour `OPENCODE_CONFIG_DIR`), Pi `<pi_home>/AGENTS.md` (default `~/.pi/agent`, honour
`PI_CODING_AGENT_DIR`). OpenCode and Pi use one generic adapter (`core::adapters::agents_file`)
that only handles instruction sections; a missing file means no items and the first replica
creates it. An agent takes part in a run only when its home directory exists. Scope is always `Global`; project-level instruction files live inside
repositories and are never read or written.

- Parsing: split at lines starting with `## ` that are outside fenced code blocks (``` or ~~~).
  Everything before the first section (title, `@imports`, intro) is preamble: never an item, never
  modified. `description` = heading text, `name` = `slugify(heading)`, `body` = text after the heading
  line with leading and trailing blank lines trimmed, `category` = None. `native_id` =
  `<file name>#<name>`, with `-2`, `-3` for duplicate headings.
- Managed regions are excluded from original detection: the instruction region below and the Codex
  routing block (`<!-- nativemem:begin -->` .. `<!-- nativemem:end -->`).
- Replicas live in one region at the end of the file, between `<!-- nativemem:instructions:begin -->`
  and `<!-- nativemem:instructions:end -->`. Each replica is a marker line
  `<!-- nativemem origin=<agent> id=<encoded native_id> -->` (same encoding as memory markers), then
  `## <heading>`, a blank line and the body. Replicas are sorted by (origin, native_id). The region is
  created with the first replica and removed with the last; the blank line the tool adds before it goes
  with it. Bytes outside the region are never changed by replica writes.
- `write_original` on an instruction section rewrites its body in place and keeps the heading line;
  everything else in the file stays byte-for-byte. `remove` deletes the section. Both back up the file
  first. All adapters support this for instruction originals; Codex-generated memory originals stay
  unsupported.
- The heading is part of identity. Editing a heading by hand is a delete plus an add. The CLI `edit`
  command changes the body only for instruction items and answers code `invalid` when the submitted
  `name` or `description` differs from the current ones.
- The `items --json` contract gains a `"kind"` field (`"memory"` or `"instruction"`) on every entry.
