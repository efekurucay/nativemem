# Compatibility watch

## Purpose

How `nativemem` notices that an agent updated itself, checks that the on-disk formats the adapters
depend on still match, stops using the affected feature instead of writing wrong data, and tells the
user.

## Scope

The probes, the state file, status logic, feature gating, `nativemem doctor`, the prompt, notifications
and the desktop notices. The checks of each agent are listed with their expected constants on
`wiki/agents/<agent>.md`; the steps to take when one fails are in
`wiki/runbooks/agent-update-action-required.md`.

## How it works

1. An adapter answers `Adapter::version()` and `Adapter::probes()` (`crates/core/src/model.rs`). Both
   are read-only and fast: they sample the newest files, never scan a whole store, and never run an
   agent.
2. A probe compares a structural descriptor of what is on disk with a constant that lives next to
   the adapter code (the format the adapter was written against). The result is a `Check { id,
   feature_kinds, summary, expected, observed, ok, accepted, sources }` (`crates/core/src/compat.rs`).
   `sources` are the repo paths of the adapter files the check protects.
3. `compat::assess` combines the checks with `<state dir>/compat.json` into an `AgentReport` with a
   `Status`.
4. Before every `sync` (preview and apply), `status` and `doctor`, the CLI assesses every agent
   (`assess_all` in `crates/cli/src/doctor.rs`). The kinds protected by failing checks are passed to the
   adapters through `with_blocked_kinds(...)`, so `supports(kind, scope)` returns false for them.
5. An agent whose home directory does not exist is `NotInstalled` and its probes do not run.

## Observed values are descriptors only

An `expected` or `observed` value is a key set, a column signature, a set of types, a count or a short
tag such as `no rollouts`. It never contains message text, titles, paths of the user's projects or
secrets. A new probe that needs content to decide has to reduce it to such a descriptor first.

## Status

| Status | When |
|---|---|
| `ActionRequired` (`action_required`) | any check is not ok after reviewed overrides are applied |
| `Updated` (`updated`) | every check is ok and the version changed within the last 3 days (`UPDATED_WINDOW_MS`) |
| `Ok` (`ok`) | every check is ok and no recent version change |
| `NotInstalled` (`not_installed`) | the agent's home directory does not exist |

The first version ever seen is not a change. A run that cannot find the version (a scheduled job with a
minimal `PATH`) keeps the remembered one.

## State file

`<state dir>/compat.json`, per agent: `version`, `previous_version`, `changed_ms`, `accepted` (check id
to the observed value the user reviewed) and `notified` (the last status the user was told about).
Reading a missing or unparsable file gives an empty state. Only `doctor` and `sync --apply` write it,
atomically and with a backup under `<state dir>/backups/<stamp>/nativemem/compat.json`, and only when its
content changed. `status` and dry-run `sync` never write it. `doctor --accept` and `sync --apply` hold
the lock of `wiki/architecture/safety.md`; a plain `doctor` skips its write while another run holds the
lock.

## Reviewed overrides

`nativemem doctor --accept <agent>` records the current observed value of each failing check of that
agent. A failing check whose observed value equals its recorded value counts as ok and is shown as
`reviewed`. A different observation fails again and needs a new review, so a second change cannot slip
through behind the first. Probe observed values for failures are therefore built from stable facts
(unknown names, missing keys, counts of deviations), not from the size of the sample.

## Notices

`AgentReport.notices` carries informational lines that never change the status or the exit code. Four
notices: while OpenCode session writes are off (`NATIVEMEM_OPENCODE_SESSION_WRITES` is not `on` and `opencode.db` exists), `opencode: session writes are off; set NATIVEMEM_OPENCODE_SESSION_WRITES=on to let nativemem write sessions into OpenCode`; while Codex session writes are off (`NATIVEMEM_CODEX_SESSION_WRITES` is not `on`), `codex: session writes are off; set NATIVEMEM_CODEX_SESSION_WRITES=on to let nativemem write sessions into Codex`; and while Claude Code's retention is the default 30 days (`cleanupPeriodDays` absent from its
settings), `doctor` and `status` say `transcripts older than 30 days are deleted by Claude itself; set
cleanupPeriodDays in Claude's settings to keep older sessions` when another agent holds a session
Claude would delete (`add_notices` in `crates/cli/src/doctor.rs`, which scans those agents). It is the
`notices` array of each agent in `doctor --json` and `status --json` and secondary text in the agent's
Overview box. The settings file is never edited.

Every agent also has one skills probe, `<agent>.skills_layout` (protects `skill`): the skills directory is
absent, empty, or most of its directories (dot-directories and `synced` left out) hold a top-level
`SKILL.md` (`LAYOUT_EXPECTED` in `crates/core/src/adapters/skills.rs`); the observed value is a bucket, not
a count. A third notice, `skill <name>: the copy in the shared skills directory has other content than
this agent's own copy`, comes from `Adapter::skill_report` for Codex and Pi. Tests:
`doctor_probes_the_skills_directory_and_names_shadowed_copies` and
`a_skills_directory_that_stops_looking_like_one_pauses_skills_for_that_agent` in `crates/cli/tests/cli.rs`.

## Gating

The kinds of failing checks are paused for that agent only:

- The agent's `scan` returns no item of a paused kind and does not read those files at all, so a
  format that no longer matches cannot produce wrong data that replicates to other agents. `supports`
  stays false for the kind, so nothing is written, written back or removed in the agent.
- Everything that already exists for a paused kind is left alone: the engine plans nothing for an item
  whose origin is paused or that has a replica in a paused agent (`plan_key` in
  `crates/core/src/engine.rs`), and a session of a paused agent is not an original. Ledger rows,
  replicas and originals stay as they are: no removal, update, tombstone or adoption. The second plan
  with a paused kind shows nothing for that kind.
- Other agents and other kinds keep syncing. `items` is a read command and lists what the files hold.
- `sync` and `status` print the notice lines, for example `codex: sessions paused, run nativemem doctor`.
- After `--accept` the kind resumes from the ledger without duplicates.

Tests in `crates/cli/tests/doctor.rs`: `sync_pauses_exactly_the_protected_kinds_and_keeps_everything_else_syncing`,
`a_paused_memory_kind_leaves_the_agent_alone_in_both_directions`,
`paused_claude_sessions_are_not_read_so_they_do_not_replicate_out`,
`a_paused_kind_leaves_existing_replicas_and_ledger_rows_alone`, `accepting_resumes_sync_without_duplicates`;
`paused_codex_memory_leaves_existing_replicas_untouched` in `crates/core/tests/paused.rs`.

Codex's checks gate a write path that is itself opt-in (`NATIVEMEM_CODEX_SESSION_WRITES=on`): with the switch on and while `codex.state_db`, `codex.threads_columns` and the other Codex
session checks pass, session replicas are written into Codex together with their `threads` row; a
failing one pauses Codex sessions, so no rollout and no row is written
(`schema_drift_or_a_switch_off_writes_no_row_and_takes_no_backup` in `crates/core/tests/codex_threads.rs`).
The row writer sets explicit values for every column it writes and relies on none of the table's
triggers; a Codex release that adds a column to `threads` therefore pauses it until the constants are
reviewed.

OpenCode's session checks protect `session` and exist only while `opencode.db` exists: `opencode.session_schema` (column signature of `session_v2`, `session_message`, `project` and `event_sequence`) and `opencode.version` (installed major version `2`, from the install path; unknown passes). A failing one pauses OpenCode sessions, so nothing is read or written; a busy database is not a failure. Writes into OpenCode are opt-in (`NATIVEMEM_OPENCODE_SESSION_WRITES=on`) and then follow these checks (`wiki/decisions/opencode-session-writes-gated.md`). Tests: `a_drifted_schema_pauses_opencode_sessions_in_both_directions` in `crates/cli/tests/opencode_sessions.rs`, `the_schema_probe_passes_on_the_real_structure_and_pauses_on_drift` in `crates/core/tests/opencode_sessions.rs`.

## `nativemem doctor`

`doctor [--json] [--agent <id>] [--accept <agent>] [--prompt <agent>]`. Human output lists, per agent,
version, status, reviewed checks, failing checks with expected and observed descriptors, and the paused
kinds. `--prompt <agent>` prints the prompt for an agent that needs action. `--accept <agent>` needs an
installed, known agent (`unknown_agent` and `not_installed` are the error codes). `--json` prints one
document with a `prompt` for every `ActionRequired` agent; the shape is in `docs/desktop.md`. Exit
codes: 0 when everything is ok or only updated, 3 when any shown agent needs action, 1 on error.
`status` prints one `warning:` line per agent that is updated or needs action, and `status --json` adds a
`compat` summary.

## Prompt

`compat::prompt(report, repo)` writes a prompt for a coding agent working in this repo: the agent and
version change, every failing check with expected and observed descriptors and the kinds it pauses, the
adapter files to read (the `sources` of the failing checks), the wiki pages
(`wiki/agents/<agent>.md`, `wiki/guides/add-an-agent.md`, `wiki/architecture/overview.md`), the hard
rules, the task (update constants, adapter code, tests and wiki together), the commands to run and a
closing request that the user run `nativemem doctor --agent <id>`. The repo location is `NATIVEMEM_REPO`,
else the directory the binary was built from while it still exists, else `<path to the nativemem repo>`
(`repo_location`).

## Notifications and schedule

`sync --apply --notify` shows a macOS notification (`osascript -e 'display notification …'`) once per
status change, compared with `notified` in `compat.json`. Only `ActionRequired` ("Codex needs attention,
run nativemem doctor") and `Updated` ("Codex updated to 0.170.0, all checks pass") notify; a return to `Ok`
is recorded silently. The text names the agent and the status only. `schedule install` adds `--notify` to
the job. `NATIVEMEM_NOTIFY_FILE` appends the text to that file instead of calling `osascript`; tests use
it (`deliver` in `crates/cli/src/doctor.rs`).

## Desktop app

The app runs `doctor --json` on load and on Reload (`DoctorDocument` in
`apps/macos/Sources/NativememKit/Contract.swift`). The Overview shows a native box per agent that needs
action (failing checks in a disclosure, "Copy prompt", "Mark as reviewed" with a confirmation, then
`doctor --accept <agent>` and a reload) and one line per updated agent. The Overview row of the sidebar
carries the number of agents that need action. Tests use fixtures (`doctor.json`, `doctor_ok.json`),
never the real binary.

## Source anchors

- `crates/core/src/compat.rs`
- `crates/core/src/probe.rs`
- `crates/core/src/adapters/claude_probes.rs`
- `crates/core/src/adapters/codex_probes.rs`
- `crates/core/src/adapters/codex_threads.rs`
- `crates/core/src/adapters/pi_probes.rs`
- `crates/core/src/adapters/opencode_probes.rs`
- `crates/core/src/adapters/opencode_sessions.rs`
- `crates/cli/src/doctor.rs`
- `crates/core/tests/probes.rs`
- `crates/core/tests/paused.rs`
- `crates/cli/tests/doctor.rs`
- `apps/macos/Tests/NativememKitTests/DoctorTests.swift`

## Rules

- MUST: Probes are read-only and fast; they open the Codex state database with `immutable=1` and never write it. The row writer runs only while its checks pass (`wiki/decisions/codex-session-writes-gated.md`).
- MUST: Observed values are structural descriptors, never content or secrets.
- MUST: A check names the kinds it protects and the adapter files behind it.
- MUST: A new agent ships probes (`wiki/guides/add-an-agent.md`).
- MUST: Change a probe constant, the adapter and the wiki table of the agent in one task.
- SHOULD: Build a failing observed value from stable facts so a reviewed override keeps matching.

## Verification checklist

- [ ] Every check id listed on an agent page exists in its `*_probes.rs`
- [ ] The probes pass on the testkit fixtures (`crates/core/tests/probes.rs`)
- [ ] Each drift test pauses exactly the protected kinds (`crates/cli/tests/doctor.rs`)
- [ ] The JSON shape in `docs/desktop.md` matches `report_json` in `crates/cli/src/doctor.rs`

## Related docs

- `wiki/runbooks/agent-update-action-required.md`
- `wiki/architecture/overview.md`
- `wiki/architecture/safety.md`
- `wiki/agents/claude-code.md`
- `wiki/agents/codex.md`
- `wiki/agents/pi.md`
- `wiki/agents/opencode.md`
- `wiki/decisions/codex-session-writes-gated.md`
- `wiki/decisions/opencode-session-writes-gated.md`

## Last updated

2026-11-11
