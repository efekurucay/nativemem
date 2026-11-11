# Claude Code

## Purpose

What `nativemem` needs to know about Claude Code to read and write its memory, instruction sections
and sessions safely.

## Scope

Facts gathered read-only on a development machine running Claude Code 2.1.322 (transcripts written
by versions 2.1.247 to 2.1.322; 376 top-level session files across 63 project directories). The
`claude` binary is never driven by this repo.

## Status in nativemem

| Kind | Status | Adapter |
|---|---|---|
| Project memory | Read (originals) and write (new originals through `add`, replicas, edits, deletes) | `crates/core/src/adapters/claude.rs` |
| Instruction sections | Read and write in `CLAUDE.md`, including a new section through `add` | `claude.rs` through `sections.rs` |
| Sessions | Read (originals) and write (replicas) | `crates/core/src/adapters/claude_sessions.rs` |
| Global-scope memory | Not supported (`supports` is false) | |
| Skills (global) | Read (originals) and write (replicas, edits, deletes) | `crates/core/src/adapters/skills.rs`, `claude.rs` |
| MCP servers | Read by `mcp import` (global `mcpServers` only); `mcp install` and `uninstall` write the one entry `mcpServers.nativemem` | `crates/hub/src/import.rs`, `crates/hub/src/register.rs` |

## Native locations

| What | Global | Project |
|---|---|---|
| Instructions | `<claude home>/CLAUDE.md` | `<repo>/CLAUDE.md` (never touched) |
| Memory | none | `<claude home>/projects/<dir>/memory/*.md` with a `MEMORY.md` index |
| Sessions | | `<claude home>/projects/<dir>/<session id>.jsonl` |
| MCP servers | `~/.claude.json` key `mcpServers` | `~/.claude.json` `projects.<path>.mcpServers`, `<repo>/.mcp.json`, plugin `.mcp.json` |
| Skills | `<claude home>/skills/<name>/SKILL.md` (global), `<repo>/.claude/skills/<name>/` (project) (`synced/` is managed by Claude itself and left alone) | `<repo>/.claude/skills/`, plugin caches |

The Claude home is `--claude-home`, else `NATIVEMEM_CLAUDE_HOME`, else `CLAUDE_CONFIG_DIR`, else
`~/.claude`. The agent takes part in a run only when that directory exists.

## Formats

- **Memory file**: markdown with YAML frontmatter, in two variants that both parse (nested
  `metadata: { type, ... }` and flat `type`). `MEMORY.md` holds one line per entry:
  `- [<name>](<file>) — <description>`.
- **Instruction file**: markdown; `## ` sections become items (`wiki/kinds/instruction-sections.md`).
- **Session transcript**: UTF-8 JSONL, one object per line, append-only, file stem equals the
  `sessionId`. Chain lines (`user`, `assistant`, `attachment`, `system`) carry `uuid`, `parentUuid`,
  `isSidechain`, `sessionId`, `timestamp`, `userType`, `entrypoint`, `cwd`, `version`, `gitBranch`.
  One API message is split into one line per content block. Metadata lines (`last-prompt`,
  `ai-title`, `custom-title`, `mode`, `permission-mode`, and others) have no `uuid`. Titles live in
  `custom-title` (user rename), `ai-title` (generated) and `last-prompt`.
- **MCP entry** (`mcp import` reads it): `type` `stdio`, `http` or `sse`; `command`, `args`, `env`, `url`,
  `headers`, `oauth`. Inline secrets can sit in `headers` and `env`.

## What nativemem reads and writes today

- Reads `projects/<dir>/memory/*.md` (not `MEMORY.md`), `CLAUDE.md` sections and
  `projects/<dir>/*.jsonl` sessions.
- Writes replicas of memory (marker `metadata.nativemem_origin`), an instruction region at the end of
  `CLAUDE.md`, and session replicas (marker key `nativemem_origin` on every line).
- `mcp import` reads the global `mcpServers` of `.claude.json`: `<claude home>/../.claude.json` when the home
  is named `.claude`, else `<claude home>/.claude.json` (`claude_config_path` in
  `crates/hub/src/import.rs`). The import only reads the file. `mcp install` adds `mcpServers.nativemem` =
  `{type: "stdio", command, args, env: {}}` to the same file and `mcp uninstall` removes it
  (`crates/hub/src/register.rs`; `wiki/architecture/hub.md`, "Registration"). The rest of the file is kept
  byte for byte, with its indentation and trailing newline.
- Creates a project directory with the encoded name plus a helper file `nativemem.jsonl` holding the
  `cwd` when no directory exists for a project (`wiki/kinds/memory.md`).
- Reads and writes whole skill directories below `skills/` (`wiki/kinds/skills.md`).
- Never touches credentials or plugin directories, and writes `~/.claude.json` only for the `nativemem` MCP entry.
- Subagents run Claude headless: `claude -p --output-format json --model <model> [args...]` with the prompt
  on stdin; the answer is the `result` field of the JSON document, and `is_error: true` makes the run a
  failure (`build` and `parse_answer` in `crates/hub/src/harness.rs`; `wiki/architecture/hub.md`,
  "Subagents"). The binary is looked up in `PATH`, `~/.local/bin`, `/opt/homebrew/bin` and `/usr/local/bin`.

## Sessions: listing and resume

- There is no session index file. `~/.claude/history.jsonl` is the prompt history for the up-arrow,
  not the resume list. `~/.claude/sessions/<pid>.json` files are a registry of live processes.
- The resume list is built by scanning `projects/<encoded cwd>/*.jsonl` of the current project
  (INFERRED from the data; `claude --resume` was not run). Title priority: last `custom-title`, then
  last `ai-title`, then the last human prompt (`last-prompt`), then the first user message. The sort
  key is most likely the file mtime, which is independent of the last line's timestamp (the two are
  within 5 s on only 132 of 376 files), so the writer sets the mtime explicitly.
- A written replica needs: the encoded-cwd directory, at least one user or assistant chain with
  `cwd` on its lines, a file name equal to the session id, and optionally `custom-title` and
  `last-prompt` lines (INFERRED minimum; the writer produces exactly this).

## Quirks

- **Claude deletes old transcripts.** Transcripts whose file mtime is older than `cleanupPeriodDays`
  (`<claude home>/settings.json`, default 30) are deleted by Claude. A session replica carries the
  original's updated time as its mtime, so one older than the retention would be deleted soon after it
  is written. `ClaudeAdapter::accepts` (`retention` in `crates/core/src/adapters/claude.rs`) refuses a
  session whose updated time is older than the retention minus one day; the engine then plans nothing
  for it: no write, no note, no tombstone, and a missing copy of such a session is not read as a user
  deletion. The value is read at startup; absent or unparsable gives 30; a huge value is respected as
  given; `0` is read as retention disabled `[UNVERIFIED]`. `doctor` and `status` print an informational
  notice (never a failing check) while the retention is the default and another agent holds a session
  Claude would delete. The settings file is never edited.

- **Project directory names are lossy.** Every character outside `[A-Za-z0-9]` becomes `-`
  (VERIFIED on 369 of 376 files). The directory comes from the launch cwd, while lines can later carry
  a different `cwd` (149 of 376 files have more than one value), so `nativemem` reads the real path
  from the transcript's lines, not from the name, and decodes a name against real directories only
  when exactly one path matches.
- **Metadata lines come first and `cwd` appears on the first chain line**: 57 of 376 files have no
  `cwd` in their first five lines, and one title-only stub has none at all. A reader scans for the
  first line that has a `cwd`.
- **Claude prunes transcripts.** An original can vanish; session deletes therefore never propagate
  (`wiki/decisions/session-snapshot-policy.md`).
- **`~/.claude.json` is rewritten constantly** by Claude (write-temp-then-rename, rolling backups).
  It is the one agent file with real concurrent-writer contention. `mcp install` edits it as an optimistic
  read-modify-write: the file is read again just before the atomic rename and the edit starts over, up to 3
  times, when it changed; after that the agent fails with code `busy` and nothing is written (test
  `claude_update_starts_over_when_the_file_changes_under_it`).
- **`~/.claude/.mcp.json` is not Claude's global MCP file.** Claude reads `.mcp.json` only from the
  session's project directory.
- **Thinking blocks carry opaque signatures** and are dropped on conversion.

## Version detection and compatibility probes

The version is the newest `version` value on the most recent lines of the newest transcripts
(`version` in `crates/core/src/adapters/claude_probes.rs`); lines this tool wrote are skipped. The
session writer pins the transcript `version` field to the constant `CLAUDE_VERSION` in
`crates/core/src/adapters/claude_sessions.rs` (`2.1.322`).

The probes (`wiki/architecture/compatibility.md`) read the newest 3 transcripts (last 1 MiB each),
the newest 30 memory files and the newest 8 project directories. Observed values are key sets and
counts only.

| Check id | Protects | Expected constant | Fails when |
|---|---|---|---|
| `claude.line_types` | session | `KNOWN_LINE_TYPES`: 22 top-level `type`s (`user`, `assistant`, `attachment`, `system`, `custom-title`, `ai-title`, `last-prompt`, `mode`, `permission-mode`, and 13 metadata types) | a line has another `type` |
| `claude.message_keys` | session | `REQUIRED_MESSAGE_KEYS`: `cwd`, `message`, `message.content`, `sessionId`, `timestamp`, `type`, `uuid` on every `user` and `assistant` line | a key is missing on one of them |
| `claude.memory_frontmatter` | memory | `KNOWN_TOP_KEYS`: `name`, `description`, `metadata`, `type`, `originSessionId` at the top level; `name` and `description` on every file; `type` under `metadata` or at the top level; other `metadata` keys are free-form | an unknown top-level key, a file without `name` or `description`, or no file with a `type` |
| `claude.memory_index` | memory | link lines of `MEMORY.md` are `- [name](file.md) — description` (`INDEX_LINE_SHAPE`) | a link line has another shape |
| `claude.project_dir_encoding` | memory, session | directory name equals `encode_name(cwd)`, or starts with it plus `-` when the session moved into a subdirectory | a sampled directory (name under 200 characters) matches none of its `cwd` values |
| `claude.skills_layout` | skill | `LAYOUT_EXPECTED`: the skills directory is absent, empty, or most of its directories hold a top-level `SKILL.md` | it is not a directory, or most directories lack `SKILL.md` |

Evidence (read-only statistics on the development machine, Claude Code 2.1.322, 809 transcripts in
154 project directories): the newest 60 transcripts held 22 line types and every one of 5058 `user`
and 11118 `assistant` lines carried the required keys; 420 memory files all had frontmatter whose
top-level keys lie within the five above, and 418 of 418 link lines had the index shape; 133 of 136 directories equal
`encode_name(cwd)` and 3 are prefixes (the session moved into a subdirectory). One memory file
carries an extra key under `metadata`, which is why those keys are not checked.

## Facts and evidence

| Fact | Evidence level | Source |
|---|---|---|
| Directory name = launch cwd with non-alphanumerics replaced by `-` | VERIFIED (369 of 376 files) | read-only corpus check; `encode_name` in `crates/core/src/adapters/claude.rs` |
| One API message is split into one line per content block | OBSERVED | transcript corpus |
| Physical order is parents first in 371 of 376 files | OBSERVED | transcript corpus |
| Replica key `nativemem_origin` is ignored by Claude and kept when the session is continued | INFERRED (Claude reads known keys and appends) | `wiki/kinds/sessions.md` |
| The resume list scans the project directory and sorts by mtime | INFERRED | no index file exists; `claude --resume` not run |
| Minimum fields for a resumable file | INFERRED | `claude_sessions.rs` writer; tests `rendered_replica_parses_back_with_marker` |
| Claude lists a replica in the real resume list | `[UNVERIFIED]` until the staged check in `wiki/guides/verify-with-real-agents.md` is recorded | |
| The five probes pass on the fixtures and fail on an unknown line type, a missing key, an unknown frontmatter key, a changed index line and a renamed project directory | VERIFIED | `crates/core/tests/probes.rs` (`claude_*`) |
| Claude deletes transcripts by mtime after `cleanupPeriodDays` (default 30) | OBSERVED (the tombstones the engine recorded for replicas of old sessions); the meaning of `0` is `[UNVERIFIED]` | real-data report; tests `a_configured_retention_is_respected_and_needs_no_notice` |
| MCP server schema, `~/.claude.json` layout | OBSERVED | read-only file and binary-string inspection |
| Claude Code hot-reloads edits to `mcpServers` | `[UNVERIFIED]` | assume a restart is needed |
| The `nativemem` entry is written next to the other servers with their bytes kept | VERIFIED | `crates/hub/tests/register.rs` (`claude_gets_the_entry_and_everything_else_stays_byte_for_byte`) |
| Claude starts the registered hub from that entry | `[UNVERIFIED]` | needs a staged check with `claude mcp list` |
| `claude --help` lists `-p/--print`, `--output-format` (`text`, `json` as "single result", `stream-json`) and `--model` | OBSERVED | `claude --help` only; no run |
| The `json` output is one object with the answer in `result` and `is_error` | `[UNVERIFIED]` | the parser is tested on fixtures in `crates/hub/src/harness.rs`; no real run |

## Source anchors

- `crates/core/src/adapters/claude.rs`
- `crates/core/src/adapters/claude_sessions.rs`
- `crates/core/src/adapters/claude_probes.rs`
- `crates/core/src/testkit.rs` (`ClaudeHome`)
- `crates/core/tests/probes.rs`
- `README.md` (support table)
- `docs/design.md` (Claude Code adapter)

## Rules

- MUST: Resolve a project's path from transcript lines, not from the directory name alone.
- MUST: Keep unknown frontmatter keys when editing a memory original.
- MUST: Never write a session Claude would delete (`accepts`); never edit `settings.json`.
- MUST: Never delete a session; never propagate a session delete.
- MUST: Mark every session line the tool writes with `nativemem_origin`.
- MUST: Update the probe constants in `claude_probes.rs` and the table above together with the adapter.
- SHOULD: Pin any new Claude-facing version constant in one place and name it on this page.

## Verification checklist

- [ ] The staged real-agent check for Claude Code (resume list, opening a copy) is recorded here when run
- [ ] The facts table agrees with the tests it names
- [ ] The README support table row for Claude Code matches this page

## Related docs

- `wiki/architecture/compatibility.md`
- `wiki/runbooks/agent-update-action-required.md`
- `wiki/kinds/memory.md`
- `wiki/kinds/sessions.md`
- `wiki/guides/verify-with-real-agents.md`
- `wiki/agents/codex.md`

## Last updated

2026-11-11
