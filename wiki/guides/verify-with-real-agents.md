# Verify with real agents

## Purpose

The staged rollout recipe for checking `nativemem` against the real agents, the concrete checks per
agent, and how to roll back from the backup directory.

## Scope

Real homes of Claude Code, Codex, Pi and OpenCode (Hermes has no adapter). The user runs the real
apply and judges what the agent shows; a coding agent prepares commands and rehearses on copies but
does not run `nativemem` against a real home (`wiki/CLAUDE.md`). Paths below use `/Users/x`.

## Before any stage

- [ ] `nativemem schedule status` shows whether the timer is installed. Run `nativemem schedule
  uninstall` first, so a timer cannot apply everything between stages.
- [ ] Note the agents' versions (each agent's own `--version`) in the stage notes.
- [ ] Rehearse on copies: copy the agent homes to a temp directory and run with explicit roots, for
  example `nativemem --claude-home /tmp/rehearsal/claude --codex-home /tmp/rehearsal/codex
  --opencode-home /tmp/rehearsal/opencode --opencode-data-home /tmp/rehearsal/opencode-data --pi-home /tmp/rehearsal/pi --state-dir /tmp/rehearsal/state
  sync`. Copies can hold secrets; delete them afterwards.
- [ ] Take a hash of files that must not change, for example `shasum` of `~/.codex/config.toml`, and
  compare after each stage. `sync` never writes that file; only `nativemem mcp install` and `mcp uninstall`
  do, and only the `nativemem` MCP entry (`wiki/decisions/mcp-hub.md`).

## Verification with synthetic conversations

Claude Code 2.1.330, Codex 0.171.0, OpenCode 2.3.2 and Pi 1.6.0 each created a session in
the same synthetic project using a loopback model. The first scoped apply created all twelve
directed copies. Each target resumed its copy, sent the prior user message to the model and
persisted a follow-up. The stored copies contained both original speakers' turns.

All 24 three-agent routes were tested independently from the same clean baseline. After the
middle agent persisted a follow-up, a scoped apply refreshed untouched copies. In all 24
routes the third agent resumed its copy and included both the initial history and the middle
agent's follow-up in its loopback model request.

All 24 four-agent orders passed as well. The fourth agent resumed through its native CLI,
and its loopback model request included the initial history and both intermediate follow-ups.

Codex and Pi also resumed separate copies of a Claude session and persisted divergent turns.
Scoped apply reported one conflict, wrote zero sessions, and left the Codex, Pi and OpenCode
copies intact.

An OpenCode original resumed through Claude, Codex and Pi in separate tests. In all three,
OpenCode resumed the same native session id after scoped sync and sent the original and the
other agent's follow-up to the local model. Claude → OpenCode → Pi → OpenCode also returned
to the same locally continued OpenCode copy with both follow-ups. The database fixture
checks that existing message rows stay byte-identical and that an active session causes
the append and the remaining writes for that session to be skipped.

A Codex original resumed through Claude, OpenCode and Pi in separate tests. All three
returned to the same paginated Codex thread id. The native history returned both the
appended user and assistant messages, and the next model request carried the conversation.
Interactive terminal resume rendered both roles. Claude → Codex → Pi → Codex also
returned to the same locally continued legacy copy with both roles in the interactive terminal.
A simulated interrupted projection repaired missing index rows on apply. A second apply
kept the rollout and the repaired row count stable. A rollout fixture with a started but
incomplete Codex turn skipped the append and subsequent writes for that session.

A Pi original resumed through Claude, Codex and OpenCode in separate tests. Each route
returned to the same Pi file and native id with the original bytes preserved. Claude →
Pi → Codex → Pi returned to the same locally continued copy. Pi 1.6.0 included the
prior turns in the local model request and rendered both follow-ups in the interactive
terminal history.

A Claude Code original resumed through Codex, OpenCode and Pi in separate tests. Each
route returned to the same Claude UUID. Its native resume sent the original and intervening
turns to the local model, and the full initial transcript stayed byte-identical. Pi →
Claude Code → Codex → Claude Code also returned to the same locally continued copy. These checks
used the native print-mode CLI. The isolated interactive profile did not reach the transcript.

Four distinct global skill fixtures synced byte for byte and were advertised by all four agents.
Two project skills originated under `.claude/skills` and `.agents/skills`. Both directories and
their auxiliary files synced in both directions. All four native CLIs advertised both skills in
model requests in a trusted test project. OpenCode and Pi loaded them without local duplicates.

Codex 0.171.0 imported a default-profile Claude project's note through native app-server on
`sync --apply --no-sessions`. The fixture verified byte-identical content, a backed-up update,
note deletion, cleanup of the removed project, and a stable rerun. A project with a replica marker
was skipped. The dry run and session-only apply did not import memory. Codex queued and started
its own consolidation job. The loopback probe did not produce a new consolidated summary.

The auxiliary memory store remains available to the manager. Full apply removes the generated
global memory-routing block in the fixture homes. Synced instruction section markers still
appeared in Codex, OpenCode and Pi model requests.

## Stages

1. **Preview.** `nativemem status`, then `nativemem sync` (dry run) on the real homes. Read the plan:
   counts of adds, updates, removes; every conflict and skipped note; the `mass_delete` refusal if the
   plan removes many entries. Nothing is written.
2. **One project.** For a session-only check, preview with `nativemem sync --session-project /absolute/project`
   and apply it with `--apply` after checking the plan. To test a time window across projects, use
   `nativemem --session-days 1 sync --apply` (sessions limited to
   files modified in the last day; memory and instruction items always take part). A narrower slice
   for a new writer is `--no-sessions`, which isolates memory and instruction changes and writes no
   Codex or OpenCode session row.
3. **The user checks the agent's own listing and opens an item.** The checks per agent are below.
   Do not continue until each passes.
4. **Everything.** `nativemem sync --apply` without `--session-days`. Run `nativemem sync` again: the
   plan must be empty (second runs are idempotent). Re-install the timer only after this.

## Checks per agent

### Claude Code

- [ ] `claude --resume` in a project directory lists the copied sessions with the original's title
  and in the original's time order.
- [ ] Open a copy: the transcript renders, tool activity shows as `Tool: <name>` text blocks, and the
  conversation continues. After continuing, `nativemem sync` (dry run) leaves that copy untouched and
  plans to relay its new turns to eligible copies.
- [ ] The synced region at the end of `CLAUDE.md` is between `<!-- nativemem:instructions:begin -->`
  and `<!-- nativemem:instructions:end -->`; text above it is unchanged. A project's `MEMORY.md` has
  the new index line and the new memory file exists.

### Codex

- [ ] `AGENTS.md` carries the instruction region. A full apply removes a generated memory-routing block and keeps a backup.
- [ ] `shasum` of `config.toml` and the hooks files is unchanged (after `sync`; `mcp install` changes `config.toml` by one table).
- [ ] `nativemem doctor --agent codex` passes, then apply a narrow slice (`--session-days 1`). Run
  `codex resume` in the project folder and confirm the session is listed and opens, and that Codex
  does not offer to import the same Claude session again (the `[UNVERIFIED]` facts on
  `wiki/agents/codex.md`; record the result there).
- [ ] `<state dir>/backups/<stamp>/codex/state_<n>.sqlite` exists after the first write; rollback is
  closing Codex and copying that file back over `state_<n>.sqlite` (remove its `-wal` and `-shm`).
- [ ] With `--no-sessions` or a failing Codex check no Codex rollout, row or ledger record is written.

### Pi

- [ ] `pi --resume` (and its "all" scope) lists the copies by name or first message.
- [ ] Open a copy: no model-restore warning, the transcript renders, and Pi continues it.
- [ ] `AGENTS.md` carries the instruction region. If an `AGENTS.override.md` exists
  in the Pi home, sections reach Pi only through that file; check which file Pi loads.

### OpenCode

- [ ] After the first sync OpenCode has its own global `AGENTS.md`; confirm the Claude Code sections
  are visible to it (ask for the rules), since it reads `~/.claude/CLAUDE.md` only while its own
  `AGENTS.md` is absent.
- [ ] Sessions flow between OpenCode and the other agents. `nativemem doctor --agent opencode` must
  pass before the slice is applied. Quit
  OpenCode and its background service, apply a narrow slice (`--session-days 1`), start OpenCode again,
  run `opencode -c` and the session picker in a synced project folder and confirm the copies are
  listed newest first, open without a model warning, and continue (the `[UNVERIFIED]` facts on
  `wiki/agents/opencode.md`; record the result there).
- [ ] `<state dir>/backups/<stamp>/opencode/opencode.db` exists after the first write; rollback is
  quitting OpenCode and its service and copying that file back over `opencode.db` (remove its `-wal`
  and `-shm`).
- [ ] With `--no-sessions` or a failing OpenCode check, `opencode.db` is byte for byte unchanged after `sync --apply`.

## What to record

On the agent page: the date, the agent version, which checks passed, and the evidence level of each
fact the stage confirmed (`wiki/architecture/context-quality.md`). A failed check is a finding for
the page and a test for the repo, not a reason to edit the agent's files by hand.

## Roll back from `~/.nativemem/backups`

Every run that overwrites or deletes a file first copies it to
`~/.nativemem/backups/<UTC stamp>/<agent>/<path relative to the agent root>`; the stamp is
`YYYYMMDDTHHMMSSZ` and one run shares one stamp, holding each file's state before that run.

1. List the runs: `ls ~/.nativemem/backups`. Pick the stamp of the run to undo.
2. Restore a file: copy `~/.nativemem/backups/<stamp>/<agent>/<relative path>` back to
   `<agent root>/<relative path>` (for Claude Code, root `~/.claude`; Codex `~/.codex`; OpenCode
   `~/.config/opencode`, its database `~/.local/share/opencode/opencode.db`; Pi `~/.pi/agent`).
3. Files that did not exist before the run (new replicas, a new region in a new file) have no backup:
   `nativemem sync --json` output of the run lists them as `add` actions. Delete them in the agent
   if they should go.
4. A restored original looks like an edit to the next sync and would propagate. Stop the timer and do
   not apply again until the plan is read.
5. A session copy the user deletes in its own agent is never recreated (tombstone), which is the
   quickest way to drop a copy for good.
6. Backups can hold what the agent files held; keep the directory private.

## Source anchors

- `crates/core/src/fsx.rs` (`backup`, `BackupSession`)
- `crates/cli/src/main.rs` (`--session-days`, `--no-sessions`, `schedule`)
- `crates/core/src/engine.rs` (`plan_session`, `Action::Tombstone`)
- `README.md` (Usage, Safety)

## Rules

- MUST: Preview before every real apply; never skip stages for a new writer.
- MUST: The user verifies in the real agent between stages.
- MUST: Uninstall the timer for the duration of a rollout.
- MUST: Record the outcome on the agent page.
- SHOULD: Rehearse on copies in a temp directory first.

## Verification checklist

- [ ] The final `sync` after stage 4 plans nothing
- [ ] Files that must not change have the same hash as before
- [ ] The agent pages record this rollout's results

## Related docs

- `wiki/architecture/safety.md`
- `wiki/guides/add-an-agent.md`
- `wiki/agents/codex.md`
- `wiki/agents/pi.md`
- `wiki/agents/claude-code.md`
- `wiki/agents/opencode.md`

## Last updated

2026-11-22
