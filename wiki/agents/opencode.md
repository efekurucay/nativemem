# OpenCode

## Purpose

What `nativemem` needs to know about OpenCode to sync instruction sections, project memory and chat
sessions with it, and the facts the session reader and writer rest on.

## Scope

Facts gathered read-only on a development machine running OpenCode 2.2.6 (a Bun-compiled binary, so
claims come from embedded strings and the live database, cited as "binary strings"). Instruction
sections, project memory and sessions are implemented. The `opencode` binary is never driven by this
repo, and `opencode.json` and `service.json` (which hold secrets) are never opened.

## Status in nativemem

| Kind | Status | Adapter |
|---|---|---|
| Project memory | OpenCode has no native memory: the store holds replicas and originals created through `add`, announced by a routing block | `crates/core/src/adapters/agents_file.rs`, `memory_store.rs` |
| Instruction sections | Read and write in `AGENTS.md`, including a new section through `add` | `agents_file.rs` through `sections.rs` |
| Sessions | Always read as originals while `opencode.db` exists; copies written only with `NATIVEMEM_OPENCODE_SESSION_WRITES=on` while the session probes pass | `crates/core/src/adapters/opencode_sessions.rs`, `opencode_project.rs`, `agents_file.rs` |
| Skills (global) | Read and write in `<opencode home>/skills/` | `crates/core/src/adapters/skills.rs`, `agents_file.rs` |
| MCP servers | Not implemented | |

## Native locations

| What | Global | Project |
|---|---|---|
| Instructions | `<opencode home>/AGENTS.md`; the Claude file `~/.claude/CLAUDE.md` is read only while this file is absent | `AGENTS.md` found by an upward search from the cwd (to home when the cwd is under home, else to the project root), inside a project only |
| Extra instruction sources | config key `instructions: [...]` (absolute paths, `~/...`, globs, relative globs, http(s) URLs); local files are watched | |
| Memory | none native; replica store `<opencode home>/nativemem/memory/<project key>/` | |
| Sessions | SQLite `<data home>/opencode.db` (default `~/.local/share/opencode`; `--opencode-data-home`, `NATIVEMEM_OPENCODE_DATA_HOME`, `$XDG_DATA_HOME/opencode`) | |
| MCP servers | `<opencode home>/opencode.json` key `mcp.<name>` (`opencode.jsonc` is merged) | `<repo>/opencode.json(c)`, `<repo>/.opencode/opencode.json(c)` |
| Skills | `<opencode home>/skills/`; also scans `.agents`, `.claude`, `.opencode` roots | `.opencode/skills`, `.claude/skills`, `.agents/skills` |

The OpenCode home is `--opencode-home`, else `NATIVEMEM_OPENCODE_HOME`, else `OPENCODE_CONFIG_DIR`,
else `~/.config/opencode`.

## Formats

- **Instruction file and memory**: markdown; see `wiki/kinds/instruction-sections.md` and
  `wiki/kinds/memory.md`.
- **Config**: JSON or JSONC with `instructions`, `mcp`, `permissions` and provider keys. Edits made by
  OpenCode's own CLI preserve formatting. The file is watched and hot-reloaded.
- **MCP entry** (not implemented): `type` `local` with `command` as one array (executable plus args),
  `environment`, `cwd`; `type` `remote` with `url`, `headers`; `enabled`, `oauth`, `timeout`.
- **Session store** (OBSERVED from the live database structure and binary strings):
  - The session table that owns the `time_suspended` column is the one that lists sessions; a second,
    frozen session table receives no new sessions. Content rows are in `session_message` (columns
    `id`, `session_id`, `type`, `seq`, `time_created`, `time_updated`, `data` JSON). Types seen:
    `user`, `assistant`, `system`, `synthetic`, `idle`, `model-switched`. Assistant content blocks are
    `text`, `reasoning` and `tool`.
  - Session ids are `ses_` plus 12 hex plus 14 base62 characters (30 in all). Message ids use `msg_`.
    All timestamps are integer milliseconds.
  - A listed session needs a `project` row (foreign key), then a session row with `id`, `project_id`,
    `slug`, `directory`, `version`, `time_created`, `time_updated`. For the picker the `path` column
    must be the empty string for a session at the project root (not NULL), `parent_id` NULL and
    `time_archived` NULL. A per-session sequence row (`event_sequence`) exists for every native
    session.
  - Project id: SHA-1 of `git-remote:<normalised origin url>`, else the id cached in the git common
    directory file `opencode`, else the first sorted root commit; without a VCS directory it is SHA-1
    of `directory:<absolute path>`.
  - `opencode session export <id>` and `opencode session import <file|url>` exist; their schema was
    not inspected and the writer does not use them (`[UNVERIFIED]`).
  - `session_message.seq` is the session's event sequence: sessions OpenCode created natively start
    above 0 and have gaps, sessions it migrated are contiguous from 0; `event_sequence.seq` is the last
    sequence in use (equal to the last message `seq`, or one above it). `instruction_state` exists for
    natively created sessions only; migrated sessions list without it.
  - `session_v2.time_idle`, `time_busy` and `idle_outcome` are set on every native session
    (`succeeded`, `failed`, `interrupted`); `time_viewed` is NULL for sessions never opened; `model`
    and `agent` may be NULL.
  - Row ids: messages `msg_` + `((ms << 12) + counter) mod 2^48` as 12 hex + 14 base62; sessions
    use the complement of that value.
  - Project id: when `<git common dir>/opencode` exists its content is the id (it can differ from the
    origin hash, for example when it was written before a remote was added), else the origin hash, else
    the first root commit.

## What nativemem reads and writes today

- Reads `AGENTS.md` sections, the replica store and, while `opencode.db` exists, root-level non-archived sessions (`wiki/kinds/sessions.md`).
- Writes session copies into `opencode.db` only as `wiki/decisions/opencode-session-writes-gated.md` describes.
- Writes instruction replicas in the region at the end of `AGENTS.md`, the replica memory store under
  `nativemem/memory/`, and the routing block in `AGENTS.md`.
- Never touches `opencode.json`, `opencode.jsonc`, any other file of the data directory, or a database row OpenCode created.
- The marker of a copy is the key `nativemem_origin` inside each message's `data` JSON (`wiki/kinds/sessions.md`).

## Sessions: listing and resume

- `opencode session list` lists top-level sessions of the current project, newest first.
- `opencode -c` takes the newest top-level, non-archived session whose `directory` equals the cwd
  exactly.
- The TUI picker filters by project and by `path` equal to the cwd's subpath, ordered by
  `time_updated` then `id`, hiding sessions with a `parent_id` or `time_archived`.
- None of this has been confirmed by running OpenCode; it comes from binary strings (INFERRED).
- Retention: no cleanup or expiry of aged sessions was found in the binary strings (INFERRED), so a
  copy is not deleted by OpenCode; only the user's archive flag hides a session.
- The OpenCode background service may hold the database open and cache session lists; whether a copy
  shows up without restarting it is `[UNVERIFIED]`, and `nativemem` never runs, signals or stops it.

## Quirks

- **OpenCode stops reading `~/.claude/CLAUDE.md` once its own global `AGENTS.md` exists.** Creating
  that file replaces the fallback, so the synced region must carry the Claude Code sections over.
  This is why the instruction replicas matter for OpenCode.
- **Skills are read from several roots**, including `~/.claude/skills` and `~/.agents/skills`, so
  skills copied into several directories appear more than once (duplicate-name policy
  `[UNVERIFIED]`). `nativemem` writes only `<opencode home>/skills/`.
- **Hot reload**: config documents and local instruction files are watched; no restart is needed.
- **The default read permission is allow for files outside the project** except `.env` patterns
  (OBSERVED in binary strings), so the routing block's absolute-path reads work. `~` expansion in the
  `read` tool is `[UNVERIFIED]`, so routing lines use absolute paths.
- **MCP key layout is ambiguous**: the file uses `mcp.<name>` while the CLI's `mcp add` writes
  `mcp.servers.<name>` in 2.2.6. Confirm with `opencode mcp list` after a first write
  (`[UNVERIFIED]`).
- **Schema moves fast**: many database migrations in six months; a session writer must be version
  pinned and re-verified on every upgrade.

## Version detection and compatibility probes

The version is the `<version>` component of a `Cellar/opencode/<version>` path: the first `opencode`
on `PATH` is located and its symlinks are resolved, never run (`version` in
`crates/core/src/adapters/opencode_probes.rs`); another install layout leaves the version unknown.
Session rows record the creating version in the `version` column.

The probes (`wiki/architecture/compatibility.md`) cover the global config directory and `AGENTS.md`,
and, while `opencode.db` exists, the session tables and the installed version. The database is opened
read-only, not `immutable`. They are created by `AgentsFileAdapter` for the agent id
`opencode`; Pi's instance of the same adapter has none.

| Check id | Protects | Expected constant | Fails when |
|---|---|---|---|
| `opencode.config_dir` | instruction, memory | the OpenCode home is a directory | it is not |
| `opencode.agents_md` | instruction, memory | `AGENTS.md` in the home is absent or a UTF-8 text file | it is a directory, another non-file, or not UTF-8 |
| `opencode.session_schema` | session | `SESSION_COLUMNS`: `table.name:type:notnull:default:pk` of every column of `session_v2`, `session_message`, `project`, `event_sequence` | a column is added, removed or changed, a table is missing, or the file is not readable SQLite (a busy database passes) |
| `opencode.version` | session | `SUPPORTED_MAJOR` `2` | the installed version (Cellar path) has another major version; an unknown version passes |
| `opencode.skills_layout` | skill | `LAYOUT_EXPECTED`: the skills directory is absent, empty, or most of its directories hold a top-level `SKILL.md` | it is not a directory, or most directories lack `SKILL.md` |

Evidence: the two checks restate what `crates/core/src/adapters/sections.rs` and `memory_store.rs`
need to read and write `AGENTS.md` (test `opencode_probes_cover_the_config_dir_and_agents_file`).

## Facts and evidence

| Fact | Evidence level | Source |
|---|---|---|
| Sessions are listed from the table that owns `time_suspended` | OBSERVED | live database rows and binary strings |
| `path` must be `''` for root sessions | INFERRED | picker query in binary strings |
| `-c` needs `directory` equal to the cwd | INFERRED | binary strings |
| Project id derivation | VERIFIED against local repositories (SHA-1 of the normalised remote; root commit; `directory:` hash) | read-only recomputation |
| Without a global `AGENTS.md` OpenCode falls back to `~/.claude/CLAUDE.md` | OBSERVED | `README.md` Notes, binary strings |
| `session export` / `import` as the safest writer path | `[UNVERIFIED]`, not used | help text only |
| Rows written as in the decision page are listed by `opencode -c` and the picker and open | `[UNVERIFIED]` | needs the staged check |
| Excess keys in a message's `data` JSON are ignored | `[UNVERIFIED]` | not testable read-only |
| A restart of the OpenCode service is needed to see copies | `[UNVERIFIED]` | not testable read-only |
| Project id: cache file, then origin hash, then first root commit | VERIFIED against local rows | read-only recomputation |
| No structural marker for headless `opencode run` sessions | OBSERVED | no source-like column or key in the rows |
| No retention of aged sessions | INFERRED | binary strings |
| Probes pass on the real table structure and fail on added, renamed or missing columns | VERIFIED | `crates/core/tests/opencode_sessions.rs` |
| The two probes pass on the fixtures and fail on an `AGENTS.md` directory | VERIFIED | `crates/core/tests/probes.rs` |
| Excess keys in `mcp` entries are ignored | `[UNVERIFIED]` | not testable read-only |

## Source anchors

- `crates/core/src/adapters/agents_file.rs`
- `crates/core/src/adapters/opencode_probes.rs`
- `crates/core/src/adapters/opencode_sessions.rs`
- `crates/core/src/adapters/opencode_project.rs`
- `crates/core/tests/opencode_sessions.rs`
- `crates/core/src/adapters/memory_store.rs`
- `crates/core/tests/probes.rs`
- `crates/cli/src/main.rs` (`opencode` root and adapter)
- `README.md` (Notes)

## Rules

- MUST: Create or edit only `AGENTS.md` and `nativemem/memory/` inside the OpenCode home.
- MUST: Use absolute paths in routing lines.
- MUST: Write only the session rows described in `wiki/decisions/opencode-session-writes-gated.md`, through `opencode_sessions.rs`.
- MUST: Keep the session probes equal to the tables the reader and writer use.
- MUST: Roll a change of the written rows out in stages (`wiki/guides/verify-with-real-agents.md`).

## Verification checklist

- [ ] After a first sync the user confirms OpenCode loads the synced sections from its own `AGENTS.md`
- [ ] The README support table row for OpenCode matches this page
- [ ] Any new session fact is added to the table with its evidence level
- [ ] After the staged check, the `[UNVERIFIED]` session facts carry their result

## Related docs

- `wiki/architecture/compatibility.md`
- `wiki/runbooks/agent-update-action-required.md`
- `wiki/kinds/instruction-sections.md`
- `wiki/kinds/memory.md`
- `wiki/guides/add-an-agent.md`
- `wiki/agents/pi.md`
- `wiki/decisions/opencode-session-writes-gated.md`
- `wiki/kinds/sessions.md`

## Last updated

2026-11-11
