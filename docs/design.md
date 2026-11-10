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
pub enum Kind { Memory, Instruction, Session } // more kinds later

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
  field of the first line that has one in any `projects/<dir>/*.jsonl`, ignoring lines that carry
  the session-replica marker `nativemem_origin` (a replica this tool wrote is not evidence that the
  folder exists). No jsonl, or no `cwd`:
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
  `NATIVEMEM_PI_HOME`, `NATIVEMEM_STATE_DIR`) and `--session-days` (`NATIVEMEM_SESSION_DAYS`), `--exclude-automated-sessions` and `--no-sessions`.
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

## Chat sessions

`Kind::Session`. One item is one chat session. Sessions are logs, not documents, so they follow a
different policy from memory and instruction items. Claude Code, Codex and Pi read and write
sessions.

- Neutral model (`core::session`): id, working directory, title, created and updated times, and an
  ordered list of turns (`user` or `assistant`, text, timestamp). Thinking blocks and provider
  signatures are dropped. Tool activity becomes one assistant text turn per call, call and result
  together, so every source renders the same way in every target: a line `Tool: <name>`, a fenced
  block with the essential input (the command for shell-like tools, compact JSON otherwise; cut at
  600 characters), then `Result:` (`Result (exit code N):` for a non-zero exit, `Result (error):`
  for a tool-reported error) with a fenced block of the output cut at 1500 characters. A cut adds a
  line `… (N more characters)`. Codex's output wrapper (`{"output": ..., "exit_code": ...}`) and
  JS-style calls such as `text(await tools.exec_command({cmd:"..."}))` are unwrapped to the command
  and the output. A result is merged into the turn of its call by call id; a result without a call
  stands alone, and empty text never makes a turn. Pi `bashExecution` messages render the same way.
  Images become `[external unsupported block: image]`.
- Engine item: kind `Session`, scope `Project(cwd)`, `name` = the session's own id, `description` =
  title, `body` = deterministic JSON of the neutral model. `native_id` is the path below the agent's
  session root (`<project dir>/<file>.jsonl`, `YYYY/MM/DD/rollout-....jsonl` for Codex). The content
  hash of a session item is SHA-256 over cwd, title and every turn's role and text: ids and
  timestamps are left out so a replica hashes like its original.
- Policy (snapshot): a replica is created once per (original, target agent) and refreshed (rewritten,
  old file backed up) only while it is unmodified (its current hash equals the ledger's) and the
  original's hash changed. A replica edited or continued in its own agent is left alone from then on,
  without a conflict note. A replica deleted by the user after it was written gets a tombstone, a
  replica ledger row with an empty hash (no schema change), and is never recreated; a replica file
  that exists but was not scanned is not treated as deleted. Edits and deletions never travel back to
  the original, a missing original never removes replicas (Claude Code prunes old transcripts by
  itself, which must not erase history elsewhere), and replicas are never originals, so a replica is
  never replicated again. Ledger rows adopt marked replicas after a lost ledger instead of writing.
  Session items are left out of the mass-delete total.
- Scope: every session of every age and kind is synced by default, so all agents list the same
  sessions in the same order. A session needs at least one user turn. Limits are opt-in:
  `--session-days <N>` (env `NATIVEMEM_SESSION_DAYS`) keeps only sessions whose file was modified in
  the last N days, applied to the file's mtime before anything is parsed; `--exclude-automated-sessions`
  leaves out automated sessions; `--no-sessions` turns session sync off. A replica gets the mtime of
  its original and the original's title, which keeps the relative order. Sources over 200 MB are
  skipped and listed by `status`, as are sessions without a resolvable cwd. A scan of about 700
  sources of 500 KB each takes about 2 s without changes, so there is no scan cache.
- Automated sessions (structural markers only; a missing or unknown marker counts as interactive):
  Claude Code when the first `user` / `assistant` line's `entrypoint` starts with `sdk` (`sdk-cli`,
  `sdk-ts`); Codex when `session_meta.source` is an object (sub-agent), `parent_thread_id` is set,
  `thread_source` is `subagent`, `source` is `exec`, or `originator` is `codex_exec` or `Claude Code`.
  Pi files carry no such marker. `status` counts them per agent either way.
- Codex: originals come from `sessions/YYYY/MM/DD/rollout-*.jsonl`: text turns from the event stream
  (`item_completed` `UserMessage` / `AgentMessage`, or `user_message` / `agent_message`), which never
  carries the context Codex injects as user messages, tool calls and results from `function_call`,
  `custom_tool_call` and their outputs; title = first user text; cwd from `session_meta`; sub-agent
  threads are skipped as automated. A replica is written as `sessions/YYYY/MM/DD/rollout-<UTC ts of
  the session's creation>-<thread id>.jsonl` shaped like the output of Codex's own importer for a
  Claude session (`core::adapters::codex_rollout`): `session_meta` (originator `Codex Desktop` for a
  Claude origin, the origin agent's display name otherwise; `cli_version`, `source`,
  `model_provider` as the importer writes them; `history_mode` `legacy`, so no `ordinal` fields),
  one `external-import-turn-N` turn per user message with event and response-item pairs, the
  `<EXTERNAL SESSION IMPORTED>` marker event once, a `token_count` and the closing `task_complete`.
  The thread id is a UUIDv7 of the creation time with the rest derived from the origin, so a refresh
  rewrites the same file; the file mtime is the original's. `session_meta` also carries
  `nativemem_origin` and `nativemem_title`, which map a re-read back to the original and keep the
  title. Only the rollout is written: Codex's `threads` table, `session_index.jsonl`,
  `history.jsonl` and the other SQLite files are not touched, because their schema belongs to Codex.
  Whether Codex lists a rollout without a `threads` row (it backfills from rollout files) is observed
  on real use; a later slice can add the row behind a schema-version check in the same module split.
- Any rollout containing the `<EXTERNAL SESSION IMPORTED>` marker is a replica, never an original,
  whichever program wrote it. Replicas written by this tool map to their origin through
  `nativemem_origin`; rollouts imported by Codex itself map through
  `external_agent_session_imports.json` (`source_path` names the Claude transcript), and a marked
  rollout without a record is ignored. Adoption: when a Claude session has a record in that ledger,
  the engine records the imported thread as the Codex replica (ledger rows, no write) with an empty
  hash, so it is never rewritten. After this tool writes a replica of a Claude session it adds the
  equivalent record (`source_path`, `content_sha256` of the transcript bytes, `imported_thread_id`,
  `imported_at`, `source_modified_at`, `title`) to that file, replacing an earlier record of the same
  thread, so Codex's importer does not import the session again. The file is parsed and rewritten
  whole (every other key and record is kept, key order is not), backed up first, created only when
  the Codex home exists, and left alone when it does not parse. Sessions of other origins have no
  record.
- Claude Code: `projects/<dir>/<session id>.jsonl`. Reading takes `user` and
  `assistant` lines in file order (the order Codex's importer uses), `isMeta` and `isSidechain` lines
  dropped, one turn per text block; `tool_use` / `tool_result` blocks are merged as above. Title = last `custom-title`, else last
  `ai-title`, else last `last-prompt`, else the first user text. The cwd is the first `cwd` on any
  line. A replica is written to the project dir whose cwd matches (created with Claude's naming rule
  when absent) as `<uuid>.jsonl`, with the uuid derived from the origin so a refresh rewrites the
  same file: a `custom-title` line, one `user` / `assistant` line per turn chained by `parentUuid`
  (minimal required fields, `version`, `cwd`, `sessionId`, ISO timestamps), and a `last-prompt` line.
  The file mtime is set to the original's.
- Pi: `sessions/--<cwd>--/<ISO ts>_<uuid>.jsonl` (header version 3). Reading takes the path from the
  last entry to the root; `message` entries become turns (`toolCall` / `toolResult` / `bashExecution`
  merged as above, thinking dropped); title = last `session_info` name, else the first user text. A replica has the
  header, the marker entry, a `session_info` entry and one linear `message` entry per turn; the file
  name carries the creation time and a uuid derived from the origin. Pi restores the model a session
  recorded from the last `model_change` entry or the last assistant message that is not virtual, and
  warns when it does not exist. A replica has no `model_change` or thinking-level entry, and its
  assistant messages carry `api: "pi-virtual"`, which Pi skips when it looks for the recorded model,
  so the replica opens with Pi's own default model and no warning.
- Writer revision: each adapter has a revision per kind (`Adapter::writer_revision`, 0 unless the
  adapter says otherwise; Pi sessions are 1). The ledger records the revision a replica was written
  with in the `replicas.revision` column, added to older ledgers on open, where existing rows read as
  0. A session replica written with a lower revision is rewritten (old file backed up) while it is
  unmodified; a modified one is left alone. Adopted replicas are recorded at the current revision.
- Replica markers (survive a lost ledger, so a replica is never mistaken for an original):
  - Claude: an extra top-level key `nativemem_origin: "<agent>:<native id>"` on every line the tool
    writes. Claude reads lines by their known keys, so the key is ignored, and the file stays a
    valid transcript; a continued replica keeps it because Claude only appends.
  - Pi: a `custom` entry with `customType: "nativemem-origin"` and `data.origin`; Pi keeps custom
    entries out of the model context.
- `items` leaves sessions out unless `--kind session` is passed and never prints their bodies;
  `delete` and `edit` answer code `unsupported` for session ids; `status` counts sessions per agent
  apart from memory and instruction entries (`sessions` in `--json`).
