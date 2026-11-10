# Hermes

## Purpose

The verified facts about Hermes Agent and the recommended native placement for synced instructions,
memory and sessions, so an adapter can be built without repeating the research.

## Scope

**No Hermes adapter exists.** `nativemem` does not read or write anything of Hermes today: `hermes`
is not a known agent id (`delete` and `edit` answer `unknown_agent`), there is no `--hermes-home`
flag and no wiki or README claim of support. This page is research for the day an adapter is added
(`wiki/guides/add-an-agent.md`). Facts were gathered read-only from the installed sources and files of
Hermes Agent 0.15.1 (state schema version 14); the agent itself was run at most to print its version.

## Status in nativemem

| Kind | Status |
|---|---|
| Project memory | Not implemented |
| Instruction sections | Not implemented |
| Sessions | Not implemented (listed under "Not yet" in `README.md`) |
| MCP servers, skills | Not implemented |

## Native locations

| What | Global | Project |
|---|---|---|
| Instructions | `~/.hermes/SOUL.md` (identity slot, always loaded). No global `AGENTS.md` or `CLAUDE.md` equivalent | one project-context file from the session cwd: `.hermes.md` / `HERMES.md` (cwd up to the git root), else `AGENTS.md` (cwd only), else `CLAUDE.md` (cwd only), else `.cursorrules` and `.cursor/rules/*.mdc`; first match wins, one type only |
| Memory | `~/.hermes/memories/MEMORY.md` and `USER.md` | none (one global store) |
| Sessions | SQLite `~/.hermes/state.db` | |
| MCP servers | `~/.hermes/config.yaml` key `mcp_servers.<name>` | none (profile-wide) |
| Skills | `~/.hermes/skills/<category>/<name>/SKILL.md`, plus `skills.external_dirs` | none |

`HERMES_HOME` selects a profile; each profile has its own `SOUL.md`, memory and database.

## Formats

- **`SOUL.md`**: markdown, cap 20,000 characters (head 70% and tail 20% are kept with a marker when
  longer, so the middle is dropped silently). It is scanned for injection patterns in the context
  scope; any hit replaces the whole file with a `[BLOCKED: ...]` stub and Hermes falls back to its
  default identity.
- **`memories/MEMORY.md` and `USER.md`**: entries separated by the line `§` (`"\n§\n"`), capped at
  2200 and 1375 characters for the whole store (config keys `memory.memory_char_limit`,
  `user_char_limit`). The caps are enforced on the tool's `add` and `replace`, not on load. The store
  is a frozen snapshot at session start and each entry is scanned with the strict scanner at load
  (a hit becomes `[BLOCKED: ...]`).
- **Sessions** (`state.db`): table `sessions(id, source, model, model_config, system_prompt,
  parent_session_id, started_at REAL, ended_at, end_reason, message_count, tool_call_count, cwd, title,
  archived, ...)` and table `messages(id INTEGER AUTOINCREMENT, session_id, role, content, tool_call_id,
  tool_calls JSON, tool_name, timestamp REAL, finish_reason, reasoning, ..., active)`. Timestamps are
  REAL Unix seconds. `content` is a plain string, or `"\x00json:" + JSON` for multimodal lists. Tool
  calls use the OpenAI function-call shape. Roles stored: `user`, `assistant`, `tool`. Full-text
  search is maintained by SQL triggers, so a plain `INSERT` suffices. Session ids:
  `YYYYMMDD_HHMMSS_<6 hex>` for CLI and TUI, 36-character UUIDs for ACP.
  `~/.hermes/sessions/` holds only `request_dump_*.json` debug files that contain auth headers: they
  are never a session store and must never be read or copied.
- **Skill**: `<name>/SKILL.md` with YAML frontmatter (`name` up to 64 characters, `description` up to
  1024, optional `platforms`, `metadata.hermes.*`, ...). Optional sibling folders `scripts/`,
  `references/`, `templates/`. Curator state lives in `.usage.json`, `.curator_state`,
  `.bundled_manifest` and `.hub/lock.json` inside the skills directory.
- **MCP entry**: `command`, `args`, `env` (values accept `${VAR}` from `~/.hermes/.env`), `url`,
  `headers`, `auth`, `enabled`, `tools.include/exclude`, timeouts.

## What nativemem reads and writes today

Nothing. No code, test or fixture touches `~/.hermes`.

## Recommended native placement (for the future adapter)

Principles: write only under `~/.hermes/skills/nativemem/` and between marker comments in
`SOUL.md`; never write `.usage.json`, `.bundled_manifest`, `.hub/`, or `memories/*.md` outside the `§`
protocol; make no config edits (Hermes re-dumps `config.yaml` on its own saves, dropping comments).

Instruction sections, ranked:

1. A managed block in `~/.hermes/SOUL.md` between `<!-- nativemem:begin -->` and
   `<!-- nativemem:end -->`: always loaded, no config edit. Keep persona plus block under 20,000
   characters and warn at about 80%. Pre-scan the block with Hermes' own patterns and fail closed:
   one hit blocks the whole file. The words `ignore`, `override`, `system`, `secret` and `hidden` must
   not appear inside the marker comments (HTML-comment injection pattern). SOUL.md has no file lock,
   so write atomically and re-read before replacing.
2. A rules skill `~/.hermes/skills/nativemem/instructions/SKILL.md`: no 20,000 cap, rides the
   "Skills (mandatory)" index, but loading is model-judged, so it is weaker for must-follow rules.
3. A few short entries in `USER.md` or `MEMORY.md`: only for one to three very short, high-priority
   rules; strict scan applies and the store is tiny.

Project and global memory items, ranked:

1. One skill per project: `~/.hermes/skills/nativemem/memory-<project-slug>/SKILL.md` with an index
   (one line per item) and `references/<item>.md` per item; `memory-global` for non-project items.
   The description carries the absolute project path (up to 1024 characters; about 120 when there are
   many projects). It is auto-indexed, curator-invisible (no `.usage.json` record) and needs no config
   edit. Loading is model-judged.
2. `skills.external_dirs` pointing at a nativemem-owned directory: survives Hermes reinstalls and is
   never touched by the curator, but needs a config edit.
3. Small global facts in `memories/MEMORY.md` and `USER.md` only: honour the `§` delimiter, the caps,
   the `fcntl` lock on `MEMORY.md.lock`, atomic temp-and-rename, and merge rather than replace.
4. `.hermes.md` in the repository: rejected, because nothing is written inside repositories.

Sessions: insert one `sessions` row (`source` `cli` so `hermes -c` and `/resume` see it, `started_at`
in REAL seconds, `archived` 0, `parent_session_id` NULL, a unique `title`) and the `messages` rows in
transcript order, then set `message_count` and `tool_call_count` to match. No other table and no JSON
file is needed. Keep `schema_version` unchanged.

## Sessions: listing and resume

- `hermes sessions list`, the browse picker and in-chat `/resume` call the same listing function: top
  sessions (no parent, or a parent that ended with `end_reason = 'branched'`), `archived = 0`, ordered
  by `started_at` descending; the preview is the first 60 characters of the earliest user message.
- `hermes -c` without an argument takes the newest session whose `source` is `cli` (or `tui` in TUI
  mode), with no archived or empty filter. `hermes -c "<name>"` and `-r <id-or-title>` resolve an exact
  id or title and follow a compression chain to the live tip.
- Resume loads the active messages ordered by `id`, so insert order is transcript order.
- A session with zero messages lists but resolves to nothing; untitled, ended TUI sessions without
  messages are pruned once 24 hours have passed.

## Quirks

- **Drift guard on the memory files.** Before any `add`, `replace` or `remove` the tool re-parses the
  file; drift is a round-trip mismatch or any entry longer than the whole-file limit. On drift it
  writes `MEMORY.md.bak.<ts>`, refuses the mutation and tells the model to clean the file. A sync
  that writes these files must produce a file that round-trips exactly.
- **Strict scanning blocks content that talks about editing agent files** (`AGENTS.md`, `CLAUDE.md`,
  `~/.ssh`, secrets-like strings). Dry-running the existing instruction files of the other agents
  through the scanner produced no hits, but a scan before writing is still required.
- **Titles must be unique** (partial unique index); a duplicate title makes the `INSERT` fail.
- **`config.yaml` is rewritten whole by Hermes** (comments lost, key order kept, `${VAR}` templates
  kept); a version key triggers rewrites on upgrade.
- **Skills placed by hand bypass the hub scanner** and are never curated, archived or patched by the
  curator unless a `.usage.json` record names them as agent-created. Reusing a bundled skill name
  collides with the re-seeding done on `hermes update`.
- **Every skill adds one line to every session's prompt** under a "you MUST load a matching skill"
  instruction.

## Version detection

No adapter, so nothing is pinned. The version comes from the agent's own `--version` output; the
database records its layout in the `schema_version` table (14 on the researched version).

## Facts and evidence

| Fact | Evidence level | Source |
|---|---|---|
| `SOUL.md` cap 20,000 characters, head 70% and tail 20% truncation | OBSERVED | Hermes source (`agent/prompt_builder.py`) |
| Project context: first match wins among `.hermes.md`, `AGENTS.md`, `CLAUDE.md`, `.cursorrules` | OBSERVED | Hermes source |
| Memory caps 2200 / 1375, delimiter `§`, drift guard, `fcntl` lock | OBSERVED | Hermes source (`tools/memory_tool.py`) and config values |
| Existing instruction files pass the strict and context scanners | VERIFIED | read-only dry run of the scanner |
| A plain `INSERT` into `messages` keeps the full-text index current | OBSERVED | trigger definitions in the schema |
| Minimum rows for a session to list and resume | INFERRED | listing and resume code; no round trip was run |
| Behaviour with an empty `system_prompt` | `[UNVERIFIED]` | not tested |
| `end_reason` vocabulary beyond `new_session`, `resumed_other`, `compression`, `branched` | `[UNVERIFIED]` | partially seen |
| Skills written under `~/.hermes/skills/nativemem/` stay curator-invisible | OBSERVED | curator source (`agent/curator.py`, `tools/skill_usage.py`) |
| Hermes edits a nativemem-written `SOUL.md` block or skill | `[UNVERIFIED]` | no write guard for `SOUL.md` found; treat Hermes edits as conflicts |

## Source anchors

- `README.md` ("Not yet")
- `docs/design.md` (long-term goal)
- `wiki/guides/add-an-agent.md`
- `wiki/decisions/native-flow-only.md`

## Rules

- MUST: Say plainly that no Hermes adapter exists wherever Hermes support is mentioned.
- MUST: Run Hermes' scanner patterns on any text before it is written into `SOUL.md`, memory files or
  a skill, and skip on a hit.
- MUST: Never read or copy `request_dump_*.json`.
- MUST: Never write `.usage.json`, `.bundled_manifest`, `.hub/` or `config.yaml`.
- SHOULD: Start the adapter with instruction sections in `SOUL.md`, then memory skills, then sessions.

## Verification checklist

- [ ] When an adapter lands, this page's status table, `README.md` and `wiki/README.md` change in the same task
- [ ] Every VERIFIED row names the staged check or test that proves it
- [ ] The placement above is re-checked against the then-current Hermes source

## Related docs

- `wiki/guides/add-an-agent.md`
- `wiki/kinds/instruction-sections.md`
- `wiki/kinds/memory.md`
- `wiki/decisions/native-flow-only.md`

## Last updated

2026-11-10
