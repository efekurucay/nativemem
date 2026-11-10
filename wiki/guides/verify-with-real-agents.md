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
  compare after each stage.

## Stages

1. **Preview.** `nativemem status`, then `nativemem sync` (dry run) on the real homes. Read the plan:
   counts of adds, updates, removes; every conflict and skipped note; the `mass_delete` refusal if the
   plan removes many entries. Nothing is written.
2. **Last day.** Apply a narrow slice: `nativemem --session-days 1 sync --apply` (sessions limited to
   files modified in the last day; memory and instruction items always take part). A narrower slice
   for a new writer is `--no-sessions`, which isolates memory and instruction changes. Codex session
   writes (and the `threads` rows that go with them) stay off unless the stage exports
   `NATIVEMEM_CODEX_SESSION_WRITES=on` on purpose.
3. **The user checks the agent's own listing and opens an item.** The checks per agent are below.
   Do not continue until each passes.
4. **Everything.** `nativemem sync --apply` without `--session-days`. Run `nativemem sync` again: the
   plan must be empty (second runs are idempotent). Re-install the timer only after this.

## Checks per agent

### Claude Code

- [ ] `claude --resume` in a project directory lists the copied sessions with the original's title
  and in the original's time order.
- [ ] Open a copy: the transcript renders, tool activity shows as `Tool: <name>` text blocks, and the
  conversation continues. After continuing, `nativemem sync` (dry run) plans no update for that copy.
- [ ] The synced region at the end of `CLAUDE.md` is between `<!-- nativemem:instructions:begin -->`
  and `<!-- nativemem:instructions:end -->`; text above it is unchanged. A project's `MEMORY.md` has
  the new index line and the new memory file exists.

### Codex

- [ ] `AGENTS.md` carries the instruction region and the routing block; text outside is unchanged.
- [ ] `shasum` of `config.toml` and the hooks files is unchanged.
- [ ] `nativemem doctor --agent codex` passes, then apply a narrow slice (`--session-days 1`) with
  `NATIVEMEM_CODEX_SESSION_WRITES=on`. Run
  `codex resume` in the project folder and confirm the session is listed and opens, and that Codex
  does not offer to import the same Claude session again (the `[UNVERIFIED]` facts on
  `wiki/agents/codex.md`; record the result there).
- [ ] `<state dir>/backups/<stamp>/codex/state_<n>.sqlite` exists after the first write; rollback is
  closing Codex and copying that file back over `state_<n>.sqlite` (remove its `-wal` and `-shm`).
- [ ] Without `NATIVEMEM_CODEX_SESSION_WRITES=on` no Codex rollout, row or ledger record is written.
- [ ] Ask Codex in a synced project about a synced memory item and confirm it reads the project index
  named in the routing block.

### Pi

- [ ] `pi --resume` (and its "all" scope) lists the copies by name or first message.
- [ ] Open a copy: no model-restore warning, the transcript renders, and Pi continues it.
- [ ] `AGENTS.md` carries the instruction region and routing block. If an `AGENTS.override.md` exists
  in the Pi home, sections reach Pi only through that file; check which file Pi loads.

### OpenCode

- [ ] After the first sync OpenCode has its own global `AGENTS.md`; confirm the Claude Code sections
  are visible to it (ask for the rules), since it reads `~/.claude/CLAUDE.md` only while its own
  `AGENTS.md` is absent.
- [ ] Ask in a synced project about a synced memory item and confirm it reads the project index.
- [ ] Sessions flow from OpenCode to the other agents without any switch. Writes into OpenCode need
  `NATIVEMEM_OPENCODE_SESSION_WRITES=on`; `nativemem doctor --agent opencode` must pass first. Quit
  OpenCode and its background service, apply a narrow slice (`--session-days 1`), start OpenCode again,
  run `opencode -c` and the session picker in a synced project folder and confirm the copies are
  listed newest first, open without a model warning, and continue (the `[UNVERIFIED]` facts on
  `wiki/agents/opencode.md`; record the result there).
- [ ] `<state dir>/backups/<stamp>/opencode/opencode.db` exists after the first write; rollback is
  quitting OpenCode and its service and copying that file back over `opencode.db` (remove its `-wal`
  and `-shm`).
- [ ] Without the switch, `opencode.db` is byte for byte unchanged after `sync --apply`.

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

2026-11-11
