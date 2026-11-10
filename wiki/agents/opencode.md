# OpenCode

## Purpose

What `nativemem` needs to know about OpenCode to sync instruction sections and project memory with
it, and the verified facts a future session adapter would build on.

## Scope

Facts gathered read-only on a development machine running OpenCode 2.2.6 (a Bun-compiled binary, so
claims come from embedded strings and the live database, cited as "binary strings"). Instruction
sections and project memory are implemented. Sessions are not: no session adapter exists. The
`opencode` binary is never driven by this repo.

## Status in nativemem

| Kind | Status | Adapter |
|---|---|---|
| Project memory | Replicas only (OpenCode has no native memory): store plus routing block | `crates/core/src/adapters/agents_file.rs`, `memory_store.rs` |
| Instruction sections | Read and write in `AGENTS.md` | `agents_file.rs` through `sections.rs` |
| Sessions | Not implemented; `supports(Session, ...)` is false | |
| MCP servers, skills | Not implemented | |

## Native locations

| What | Global | Project |
|---|---|---|
| Instructions | `<opencode home>/AGENTS.md`; the Claude file `~/.claude/CLAUDE.md` is read only while this file is absent | `AGENTS.md` found by an upward search from the cwd (to home when the cwd is under home, else to the project root), inside a project only |
| Extra instruction sources | config key `instructions: [...]` (absolute paths, `~/...`, globs, relative globs, http(s) URLs); local files are watched | |
| Memory | none native; replica store `<opencode home>/nativemem/memory/<project key>/` | |
| Sessions | SQLite `~/.local/share/opencode/opencode.db` | |
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
- **Session store** (facts for a future adapter, OBSERVED from the live database and binary strings):
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
  - `opencode session export <id>` and `opencode session import <file|url>` exist and are likely the
    supported writer path (schema not inspected, `[UNVERIFIED]`).

## What nativemem reads and writes today

- Reads `AGENTS.md` sections and the replica store.
- Writes instruction replicas in the region at the end of `AGENTS.md`, the replica memory store under
  `nativemem/memory/`, and the routing block in `AGENTS.md`.
- Never touches `opencode.json`, `opencode.jsonc`, the database or the data directory.

## Sessions: listing and resume

- `opencode session list` lists top-level sessions of the current project, newest first.
- `opencode -c` takes the newest top-level, non-archived session whose `directory` equals the cwd
  exactly.
- The TUI picker filters by project and by `path` equal to the cwd's subpath, ordered by
  `time_updated` then `id`, hiding sessions with a `parent_id` or `time_archived`.
- None of this has been confirmed by running OpenCode; it comes from binary strings (INFERRED).

## Quirks

- **OpenCode stops reading `~/.claude/CLAUDE.md` once its own global `AGENTS.md` exists.** Creating
  that file replaces the fallback, so the synced region must carry the Claude Code sections over.
  This is why the instruction replicas matter for OpenCode.
- **Skills are read from several roots**, including `~/.claude/skills` and `~/.agents/skills`, so
  skills copied into several directories appear more than once (duplicate-name policy
  `[UNVERIFIED]`).
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

The probes (`wiki/architecture/compatibility.md`) cover the global config directory and `AGENTS.md`
only; there is no database access. They are created by `AgentsFileAdapter` for the agent id
`opencode`; Pi's instance of the same adapter has none.

| Check id | Protects | Expected constant | Fails when |
|---|---|---|---|
| `opencode.config_dir` | instruction, memory | the OpenCode home is a directory | it is not |
| `opencode.agents_md` | instruction, memory | `AGENTS.md` in the home is absent or a UTF-8 text file | it is a directory, another non-file, or not UTF-8 |

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
| `session export` / `import` as the safest writer path | `[UNVERIFIED]` | help text only |
| The two probes pass on the fixtures and fail on an `AGENTS.md` directory | VERIFIED | `crates/core/tests/probes.rs` |
| Excess keys in `mcp` entries are ignored | `[UNVERIFIED]` | not testable read-only |

## Source anchors

- `crates/core/src/adapters/agents_file.rs`
- `crates/core/src/adapters/opencode_probes.rs`
- `crates/core/src/adapters/memory_store.rs`
- `crates/core/tests/probes.rs`
- `crates/cli/src/main.rs` (`opencode` root and adapter)
- `README.md` (Notes)

## Rules

- MUST: Create or edit only `AGENTS.md` and `nativemem/memory/` inside the OpenCode home.
- MUST: Use absolute paths in routing lines.
- MUST: Treat a session adapter as a new agent capability: research read-only first, then the staged
  rollout (`wiki/guides/add-an-agent.md`).
- MUST: Add probes for the session store before a session adapter is written.
- SHOULD: Prefer OpenCode's own `session import` over direct database writes when a session adapter
  is built.

## Verification checklist

- [ ] After a first sync the user confirms OpenCode loads the synced sections from its own `AGENTS.md`
- [ ] The README support table row for OpenCode matches this page
- [ ] Any new session fact is added to the table with its evidence level

## Related docs

- `wiki/architecture/compatibility.md`
- `wiki/runbooks/agent-update-action-required.md`
- `wiki/kinds/instruction-sections.md`
- `wiki/kinds/memory.md`
- `wiki/guides/add-an-agent.md`
- `wiki/agents/pi.md`

## Last updated

2026-11-11
