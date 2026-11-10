# Safety

## Purpose

Every rule that keeps `nativemem` from losing or leaking a user's agent data, exactly as
implemented, and the rules for working on this repo without touching real data.

## Scope

Write paths of the CLI and core, the state directory, and the test and rollout discipline. Agent
formats are in `wiki/agents/`.

## Runtime rules

| Rule | Exactly as implemented | Anchor |
|---|---|---|
| Preview before apply | `sync` without `--apply` prints the plan and changes nothing. A dry run with no ledger file uses an in-memory ledger | `sync` in `crates/cli/src/main.rs`; test `dry_run_plan_changes_nothing` |
| Backups | Before a delete or an overwrite the existing file is copied to `<state dir>/backups/<UTC stamp>/<agent>/<path relative to the agent root>`. The stamp is `YYYYMMDDTHHMMSSZ`, taken once per process run. The first copy of a path in a run is kept, so it holds the state before any write of that run | `backup`, `BackupSession` in `crates/core/src/fsx.rs`; tests `backup_layout`, `remove_with_backup_deletes_original` |
| Atomic writes | Write a temp file named `.<file>.nativemem-tmp-<pid>` in the same directory, `fsync` it, rename it over the target. A failed write removes the temp file | `atomic_write` in `crates/core/src/fsx.rs`; test `atomic_write_creates_parents_and_replaces` |
| Mass-delete guard | `sync --apply` refuses with error code `mass_delete` when the plan removes more than 10 entries, or more than 3 entries and more than 20% of the ledger's items. Implemented as `removes > 10 \|\| (removes > 3 && removes * 5 > total)`. `total` counts ledger items that are not sessions. `--allow-mass-delete` lifts it. The check runs before anything is written | `exceeds_mass_delete_limit` and `apply` in `crates/core/src/engine.rs`; tests `mass_delete_limit_rule`, `mass_delete_guard_triggers_and_can_be_lifted`, `mass_delete_reports_error_code` |
| Failed scan aborts | If any adapter's `scan` fails, the run stops before planning, so a failed read never looks like a mass deletion | `plan` in `crates/core/src/engine.rs`; test `failed_scan_plans_nothing` |
| Single-writer lock | `sync --apply`, `delete` and `edit` take an exclusive lock on `<state dir>/sync.lock` for the whole run. A second writer fails with code `busy` and writes nothing. `status`, `items` and dry-run `sync` do not lock | `lock_state` in `crates/cli/src/main.rs`; test `concurrent_run_is_refused_while_lock_is_held` |
| Nothing inside git repositories | Writes go only to agent home directories and the state directory. Project-level instruction files live in repositories and are never read or written; replicas for agents without native memory go to the agent home plus a marked block in the agent's global instruction file | `crates/core/src/adapters/memory_store.rs`; test `memory_writes_stay_inside_the_declared_paths` |
| Agent-owned state is untouched | No code reads or writes Codex SQLite files, `hooks.json`, `config.toml`, Claude's `~/.claude.json`, credential or auth files, or generated `memories/MEMORY.md` content. Codex generated memory is read-only | grep of `crates/` finds none of these names outside tests; tests `originals_are_read_only`, `instruction_originals_are_editable_but_memory_originals_are_not` |
| Secrets are never printed | The ledger stores ids, hashes and paths, not text. `items` prints no session bodies. `status` prints labels and reasons only. Session tool output is cut at 1500 characters and tool input at 600, but a session can still contain secrets the user typed: sessions are copied between the user's own agent homes and never printed or sent anywhere | `crates/core/src/ledger.rs`, `crates/cli/src/main.rs`, `crates/core/src/session.rs` (`TOOL_RESULT_LIMIT`, `TOOL_INPUT_LIMIT`); test `items_leaves_sessions_out_unless_asked_and_never_prints_bodies` |
| Sessions are not deletable here | `delete` and `edit` refuse session ids with code `unsupported` | `refuse_session` in `crates/cli/src/main.rs`; test `delete_and_edit_refuse_session_ids` |
| Oversized sources | Session sources larger than 200 MB are skipped and listed by `status` | `MAX_SOURCE_BYTES` in `crates/core/src/session.rs`; test `oversized_sources_are_skipped_and_reported` |

Backups are copies of agent files and can hold what those files hold. Treat the state directory as
sensitive; never paste its contents into issues, logs or the wiki.

## Test and development rules

- Tests and manual runs use temp directories (`tempfile`) only. Nothing in the build reads or writes
  the real `~/.claude`, `~/.codex`, `~/.pi`, `~/.config/opencode`, `~/.hermes` or `~/.nativemem`.
- Adapters take their root as a constructor argument. Only `resolve` in `crates/cli/src/main.rs`
  turns defaults and environment variables into paths.
- CLI tests redirect `HOME` to a temp path, remove `CLAUDE_CONFIG_DIR`, `CODEX_HOME`,
  `OPENCODE_CONFIG_DIR`, `PI_CODING_AGENT_DIR` and the `NATIVEMEM_*` root variables, and pass every
  root explicitly (`run` in `crates/cli/tests/cli.rs`). A new CLI test copies that pattern, so a
  missing flag can never fall back to the real home.
- Fixture homes come from `crates/core/src/testkit.rs` (`ClaudeHome`, `CodexHome`, `PiHome`).
- An agent's real binary is never driven by tests. Facts about an agent that need the real binary
  stay `[UNVERIFIED]` until a staged check (below) confirms them.

## Staged rollout rule

A change that makes `nativemem` write a new kind of file into a real agent store (a new agent, a new
session writer, a changed format) is applied to a real home only in stages, with the user checking in
the real agent between stages:

1. Preview (`sync`, no `--apply`) and read the plan.
2. Apply a narrow slice (`--session-days 1`, or one kind) and have the user verify in the real agent.
3. Apply everything only after step 2 is confirmed.

The concrete recipe and the checks per agent are in `wiki/guides/verify-with-real-agents.md`.
Rollback from the backup directory is described there too.

## Source anchors

- `crates/core/src/fsx.rs`
- `crates/core/src/engine.rs`
- `crates/cli/src/main.rs`
- `crates/cli/tests/cli.rs`
- `crates/core/src/testkit.rs`
- `README.md` (Safety section)

## Rules

- MUST: Back up before every delete or overwrite, through `fsx::backup`.
- MUST: Write through `fsx::atomic_write`; never truncate in place.
- MUST: Keep the guard rule in this page equal to `exceeds_mass_delete_limit`.
- MUST: Never write inside a project working directory.
- MUST: Never print secrets or session bodies; never store entry text in the ledger.
- MUST: Take the lock before any write command.
- MUST: Run tests and manual experiments against temp roots only.
- SHOULD: Verify a new write path with `sync` (dry run) on temp roots before any real apply.

## Verification checklist

- [ ] The guard numbers (10, 3, 20%) match `exceeds_mass_delete_limit`
- [ ] A new write path calls `fsx::backup` and `fsx::atomic_write`
- [ ] A new CLI test passes every root explicitly and redirects `HOME`
- [ ] No new code touches an agent-owned file listed above

## Related docs

- `wiki/architecture/overview.md`
- `wiki/guides/verify-with-real-agents.md`
- `wiki/decisions/native-flow-only.md`

## Last updated

2026-11-10
