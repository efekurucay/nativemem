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
| Project memory | Read (originals) and write (replicas, edits, deletes) | `crates/core/src/adapters/claude.rs` |
| Instruction sections | Read and write in `CLAUDE.md` | `claude.rs` through `sections.rs` |
| Sessions | Read (originals) and write (replicas) | `crates/core/src/adapters/claude_sessions.rs` |
| Global-scope memory | Not supported (`supports` is false) | |
| MCP servers, skills | Not implemented | |

## Native locations

| What | Global | Project |
|---|---|---|
| Instructions | `<claude home>/CLAUDE.md` | `<repo>/CLAUDE.md` (never touched) |
| Memory | none | `<claude home>/projects/<dir>/memory/*.md` with a `MEMORY.md` index |
| Sessions | | `<claude home>/projects/<dir>/<session id>.jsonl` |
| MCP servers | `~/.claude.json` key `mcpServers` | `~/.claude.json` `projects.<path>.mcpServers`, `<repo>/.mcp.json`, plugin `.mcp.json` |
| Skills | `<claude home>/skills/<name>/SKILL.md` | `<repo>/.claude/skills/`, plugin caches |

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
- **MCP entry** (not implemented): `type` `stdio`, `http` or `sse`; `command`, `args`, `env`, `url`,
  `headers`, `oauth`. Inline secrets can sit in `headers` and `env`.

## What nativemem reads and writes today

- Reads `projects/<dir>/memory/*.md` (not `MEMORY.md`), `CLAUDE.md` sections and
  `projects/<dir>/*.jsonl` sessions.
- Writes replicas of memory (marker `metadata.nativemem_origin`), an instruction region at the end of
  `CLAUDE.md`, and session replicas (marker key `nativemem_origin` on every line).
- Creates a project directory with the encoded name plus a helper file `nativemem.jsonl` holding the
  `cwd` when no directory exists for a project (`wiki/kinds/memory.md`).
- Never touches `~/.claude.json`, credentials or plugin directories.

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
  It is the one agent file with real concurrent-writer contention. MCP sync, if ever added, must be an
  optimistic read-modify-write with a short critical section.
- **`~/.claude/.mcp.json` is not Claude's global MCP file.** Claude reads `.mcp.json` only from the
  session's project directory.
- **Thinking blocks carry opaque signatures** and are dropped on conversion.

## Version detection

`nativemem` does not detect the Claude Code version. The session writer pins the transcript
`version` field to the constant `CLAUDE_VERSION` in `crates/core/src/adapters/claude_sessions.rs`
(`2.1.322`). Every transcript line carries the writing version in `version`. The installed version
comes from the agent's own `--version` output, run by the engineer, never by tests.

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
| MCP server schema, `~/.claude.json` layout | OBSERVED | read-only file and binary-string inspection |
| Claude Code hot-reloads edits to `mcpServers` | `[UNVERIFIED]` | assume a restart is needed |

## Source anchors

- `crates/core/src/adapters/claude.rs`
- `crates/core/src/adapters/claude_sessions.rs`
- `crates/core/src/testkit.rs` (`ClaudeHome`)
- `README.md` (support table)
- `docs/design.md` (Claude Code adapter)

## Rules

- MUST: Resolve a project's path from transcript lines, not from the directory name alone.
- MUST: Keep unknown frontmatter keys when editing a memory original.
- MUST: Never delete a session; never propagate a session delete.
- MUST: Mark every session line the tool writes with `nativemem_origin`.
- SHOULD: Pin any new Claude-facing version constant in one place and name it on this page.

## Verification checklist

- [ ] The staged real-agent check for Claude Code (resume list, opening a copy) is recorded here when run
- [ ] The facts table agrees with the tests it names
- [ ] The README support table row for Claude Code matches this page

## Related docs

- `wiki/kinds/memory.md`
- `wiki/kinds/sessions.md`
- `wiki/guides/verify-with-real-agents.md`
- `wiki/agents/codex.md`

## Last updated

2026-11-10
