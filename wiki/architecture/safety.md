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
| Atomic writes | Write a temp file named `.<file>.nativemem-tmp-<pid>` in the same directory, `fsync` it, rename it over the target. A failed write removes the temp file. A skill directory is copied into `.<name>.nativemem-tmp-<pid>` next to the target, verified against its manifest and swapped in by rename (`SkillStore::write` in `crates/core/src/adapters/skills.rs`); a symlinked skill is replaced, never written through | `atomic_write` in `crates/core/src/fsx.rs`; test `atomic_write_creates_parents_and_replaces` |
| Skill backups | Before a skill directory is overwritten or removed the whole tree is copied to the same backup layout; a symlink is recorded as a text file holding its target | `backup_dir` in `crates/core/src/fsx.rs`; tests in `crates/core/tests/skills.rs` |
| Mass-delete guard | `sync --apply` refuses with error code `mass_delete` when the plan removes more than 10 entries, or more than 3 entries and more than 20% of the ledger's items. Implemented as `removes > 10 \|\| (removes > 3 && removes * 5 > total)`. `total` counts ledger items that are not sessions. `--allow-mass-delete` lifts it. The check runs before anything is written | `exceeds_mass_delete_limit` and `apply` in `crates/core/src/engine.rs`; tests `mass_delete_limit_rule`, `mass_delete_guard_triggers_and_can_be_lifted`, `mass_delete_reports_error_code` |
| Failed scan aborts | If any adapter's `scan` fails, the run stops before planning, so a failed read never looks like a mass deletion | `plan` in `crates/core/src/engine.rs`; test `failed_scan_plans_nothing` |
| Single-writer lock | `sync --apply`, `delete`, `add`, `edit`, `doctor --accept` and the `mcp` commands that write (`add`, `edit`, `remove`, `enable`, `disable`, `set-secret`, `delete-secret`, `import --apply`, `install`, `uninstall`) take an exclusive lock on `<state dir>/sync.lock` for the whole run. A second writer fails with code `busy` and writes nothing. `status`, `items` and dry-run `sync` do not lock; a plain `doctor` takes the lock only to write `compat.json` and skips that write when it is busy | `lock_state` in `crates/cli/src/main.rs`; test `concurrent_run_is_refused_while_lock_is_held` |
| Nothing inside git repositories | Writes go only to agent home directories and the state directory. Project-level instruction files live in repositories and are never read or written; replicas for agents without native memory go to the agent home plus a marked block in the agent's global instruction file | `crates/core/src/adapters/memory_store.rs`; test `memory_writes_stay_inside_the_declared_paths` |
| Codex state database | The only write is `codex_threads.rs`: one `threads` row per replica rollout this tool wrote, only while `codex.state_db` and `codex.threads_columns` pass. It opens read-write only when a row is missing or a rollout was rewritten, sets `busy_timeout` to 5 s, takes a consistent copy with SQLite's online backup to `<state dir>/backups/<stamp>/codex/state_<n>.sqlite` (the 5 newest kept, nothing else pruned), writes in one `BEGIN IMMEDIATE` transaction, reads the rows back, and skips a locked database until the next run. It never deletes or changes a row of a thread Codex made, never changes the journal mode and runs no vacuum or checkpoint pragma. The probes read the file with `immutable=1` |
| OpenCode session database | The only write is `opencode_sessions.rs`, and only while `opencode.session_schema` and `opencode.version` pass and session sync is on. Per replica it opens `opencode.db` read-write (never created), sets `busy_timeout` to 5 s and `foreign_keys` on, takes the online backup of the run to `<state dir>/backups/<stamp>/opencode/opencode.db` (the 5 newest kept, nothing else pruned), then writes `project` (insert only when missing), `session_v2`, `session_message` and `event_sequence` in one `BEGIN IMMEDIATE` transaction and reads everything back. A busy database is skipped for the rest of the run with a `skipped` note and written on the next run. It never changes a row OpenCode created, never deletes a row (a refreshed replica rewrites its own rows; rows it no longer needs become `idle` rows), never changes the journal mode and runs no vacuum or checkpoint. A replica that has rows without the marker is left alone. The probes and the reader open the file read-only, not `immutable`, because a migration may still sit in the write-ahead log | `crates/core/tests/opencode_sessions.rs` |
| Other agent-owned state is untouched | No code writes `hooks.json`, credential or auth files, or generated `memories/MEMORY.md` content, and no code writes `config.toml`, Claude's `~/.claude.json`, `opencode.json(c)` or `mcp.json` except `crates/hub/src/register.rs`, which adds, updates or removes the one `nativemem` MCP entry (next row). Codex generated memory is read-only. `crates/hub/src/import.rs` only reads those files | tests `originals_are_read_only`, `instruction_originals_are_editable_but_memory_originals_are_not`, `import_previews_then_applies_from_the_four_agents` |
| The `nativemem` MCP entry in agent configs | `mcp install` and `uninstall` write the entry named `nativemem` and nothing else, under the state lock. The file is read, transformed and written only when it is unchanged since the read (re-read and retried up to 3 times, then code `busy` and nothing written); the previous file is backed up to `<state dir>/backups/<stamp>/<agent>/<file name>` first and the write is atomic. Every other byte stays. A file that fails to parse is never written (code `parse`); a second install changes nothing and backs nothing up | `update` in `crates/hub/src/register.rs`; tests in `crates/hub/tests/register.rs` (`a_file_that_does_not_parse_is_never_written`, `claude_gets_the_entry_and_everything_else_stays_byte_for_byte`, `codex_gets_a_table_and_every_other_byte_stays`), `claude_update_starts_over_when_the_file_changes_under_it` |
| Hub configuration and secrets | `<state dir>/hub.toml` is written by `HubConfig::save`: previous file backed up to `<state dir>/backups/<stamp>/hub/hub.toml`, atomic write, state lock held. Secret values live in the OS keychain (`FileStore` in a temp directory for tests, never backed up) and never reach `hub.toml`, output, errors, logs or the ledger; errors are scrubbed of resolved values | `crates/hub/src/config.rs`, `crates/hub/src/secrets.rs`; tests `config_round_trips_and_backs_up_the_previous_file`, `secret_references_reach_the_child_and_never_the_output`, `scrub_hides_secret_values` |
| OAuth tokens | `mcp login` and `mcp logout` take the file lock `<state dir>/hub-oauth.lock`, not the state lock, because login waits for the browser. Every token refresh in a hub process takes it too and reads the secret store again after taking it. Access and refresh tokens and the registered client live in the secret store under `oauth-<name>-tokens` and `oauth-<name>-client`; they are never printed, logged, put in `hub.toml` or the ledger, and are scrubbed from connection errors | `crates/hub/src/oauth.rs`; tests `a_refresh_waits_for_the_lock_and_reads_the_store_again`, `login_logout_list_and_test_work_through_the_cli` |
| Secrets are never printed | The ledger stores ids, hashes and paths, not text. `items` prints no session bodies. `status` prints labels and reasons only. Session tool output is cut at 1500 characters and tool input at 600, but a session can still contain secrets the user typed: sessions are copied between the user's own agent homes and never printed or sent anywhere | `crates/core/src/ledger.rs`, `crates/cli/src/main.rs`, `crates/core/src/session.rs` (`TOOL_RESULT_LIMIT`, `TOOL_INPUT_LIMIT`); test `items_leaves_sessions_out_unless_asked_and_never_prints_bodies` |
| Creating adds files only | `add` writes a new file, or a new section into an instruction file, and refuses a taken id with code `exists` before writing. The routing block, an index or an instruction file it rewrites are backed up first | `create_original` in `crates/core/src/adapters/`; tests `add_refuses_a_taken_id_and_leaves_the_file_alone`, `created_instruction_goes_before_the_region_and_a_taken_id_is_refused` |
| Sessions are not deletable here | `delete` and `edit` refuse session ids with code `unsupported` | `refuse_session` in `crates/cli/src/main.rs`; test `delete_and_edit_refuse_session_ids` |
| Oversized sources | Session sources larger than 200 MB are skipped and listed by `status` | `MAX_SOURCE_BYTES` in `crates/core/src/session.rs`; test `oversized_sources_are_skipped_and_reported` |

Backups are copies of agent files and can hold what those files hold. Treat the state directory as
sensitive; never paste its contents into issues, logs or the wiki.

## Test and development rules

- Tests and manual runs use temp directories (`tempfile`) only. Nothing in the build reads or writes
  the real `~/.claude`, `~/.codex`, `~/.pi`, `~/.config/opencode`, `~/.local/share/opencode`, `~/.hermes` or `~/.nativemem`.
- Adapters take their root as a constructor argument. Only `resolve` in `crates/cli/src/main.rs`
  turns defaults and environment variables into paths.
- CLI tests redirect `HOME` to a temp path, remove `CLAUDE_CONFIG_DIR`, `CODEX_HOME`,
  `OPENCODE_CONFIG_DIR`, `PI_CODING_AGENT_DIR`, `XDG_DATA_HOME` and the `NATIVEMEM_*` root variables, and pass every
  root explicitly, `--opencode-data-home` included (`run` in `crates/cli/tests/cli.rs`). A new CLI test copies that pattern, so a
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
- `crates/hub/src/secrets.rs`
- `crates/hub/src/oauth.rs`
- `crates/hub/src/register.rs`
- `crates/core/src/engine.rs`
- `crates/cli/src/main.rs`
- `crates/cli/tests/cli.rs`
- `crates/core/src/testkit.rs`
- `crates/core/src/adapters/codex_probes.rs`
- `crates/core/src/adapters/codex_threads.rs`
- `crates/core/src/adapters/sqlite_guard.rs`
- `crates/core/src/adapters/opencode_sessions.rs`
- `README.md` (Safety section)

## Rules

- MUST: Back up before every delete or overwrite, through `fsx::backup`.
- MUST: Write through `fsx::atomic_write`; never truncate in place.
- MUST: Keep the guard rule in this page equal to `exceeds_mass_delete_limit`.
- MUST: Never write inside a project working directory.
- MUST: Write Codex's state database only through `codex_threads.rs`; every other Codex file stays read-only.
- MUST: Write OpenCode's `opencode.db` only through `opencode_sessions.rs`; every other file of its data home and config directory is untouched, and `opencode.json` and `service.json` are never opened.
- MUST: Never print secrets or session bodies; never store entry text in the ledger.
- MUST: Take the lock before any write command, `mcp` write commands included (`mcp login` and `logout` take `hub-oauth.lock` instead).
- MUST: Run hub tests with `nativemem mcp --secrets-dir <temp dir>`; the OS keychain is never used by tests.
- MUST: Edit an agent's MCP config only through `crates/hub/src/register.rs`, and only the `nativemem` entry.
- MUST: Run tests and manual experiments against temp roots only.
- SHOULD: Verify a new write path with `sync` (dry run) on temp roots before any real apply.

## Verification checklist

- [ ] The guard numbers (10, 3, 20%) match `exceeds_mass_delete_limit`
- [ ] A new write path calls `fsx::backup` and `fsx::atomic_write`
- [ ] A new CLI test passes every root explicitly and redirects `HOME`
- [ ] No new code touches an agent-owned file listed above, except the `nativemem` MCP entry through `register.rs`

## Related docs

- `wiki/architecture/overview.md`
- `wiki/guides/verify-with-real-agents.md`
- `wiki/decisions/native-flow-only.md`
- `wiki/architecture/compatibility.md`
- `wiki/architecture/hub.md`
- `wiki/decisions/mcp-hub.md`

## Last updated

2026-11-11
