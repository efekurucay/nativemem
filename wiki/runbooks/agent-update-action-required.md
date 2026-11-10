# Runbook: an agent shows ActionRequired

## Purpose

What to do, step by step, when `nativemem doctor`, a notification or the desktop app reports that an
agent needs action.

## Scope

A failing compatibility check for Claude Code, Codex, OpenCode or Pi. The mechanism is in
`wiki/architecture/compatibility.md`; the expected constants are on `wiki/agents/<agent>.md`.

## Steps

1. **Read the report.** Run `nativemem doctor --agent <id>` (or open the Overview in the app). It lists
   the failing checks with expected and observed descriptors and the paused kinds. Those kinds are
   already paused for that agent: it is neither read nor written for them and its existing copies, originals
   and ledger rows are left alone; everything else keeps syncing.
2. **Decide what the difference is.** Open the agent's page in `wiki/agents/` and read the row of the
   failing check. Inspect the agent's files read-only, never with `nativemem` against a real home beyond
   `doctor`, and never paste their content anywhere.
3. **Harmless difference** (the adapter still reads and writes the format correctly, for example a key
   that nothing reads): run `nativemem doctor --accept <agent>`, or "Mark as reviewed" in the app. The
   command prints each accepted check and observed value; the kind resumes at the next sync from the
   ledger, without duplicates. A later, different
   observation fails again.
4. **Real format change:** run `nativemem doctor --prompt <agent>`, or "Copy prompt" in the app, and give
   the prompt to a coding agent working in this repo. It updates the expected constants in the
   `*_probes.rs` file, the adapter code, the tests and the wiki together, runs `cargo test --workspace`,
   `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all -- --check` and `swift test` in
   `apps/macos`, and works on temp directories and explicit roots only.
5. **Ship the fix.** Build (`cargo build --release`). A scheduled job runs its own copy of the binary at
   `<state dir>/bin/nativemem`, so run `nativemem schedule install` from the new build.
6. **Confirm.** Run `nativemem doctor --agent <id>`: it exits 0 and the agent is `ok` or `updated`. Run
   `nativemem sync` (dry run) and read the plan before the next `sync --apply`.

## Rollback

- A fix that proves wrong: revert the commit in git, rebuild, run `nativemem schedule install` and run
  `nativemem doctor --agent <id>` again.
- A review that proves wrong: restore `<state dir>/compat.json` from
  `<state dir>/backups/<stamp>/nativemem/compat.json`, or delete the check id from the agent's `accepted`
  object. The check fails again and the kinds are paused.
- Files written while a mistaken review was active: restore them from `<state dir>/backups/<stamp>/<agent>/`
  (`wiki/guides/verify-with-real-agents.md`).

## Source anchors

- `crates/cli/src/doctor.rs`
- `crates/core/src/compat.rs`
- `crates/cli/tests/doctor.rs` (`accept_records_the_observed_value_and_clears_the_pause`, `accepting_resumes_sync_without_duplicates`)

## Rules

- MUST: Never run the binary, the tests or the app against a real home except `nativemem doctor`, which
  reads agent files and writes only `<state dir>/compat.json`.
- MUST: Review a difference only after checking that the adapter still handles it.
- MUST: Update the wiki page of the agent in the same task as the adapter fix.
- SHOULD: Prefer the prompt flow over editing probe constants by hand.

## Verification checklist

- [ ] `nativemem doctor --agent <id>` exits 0
- [ ] The probe constants, adapter, tests and `wiki/agents/<agent>.md` changed together
- [ ] The scheduled job was reinstalled from the new build

## Related docs

- `wiki/architecture/compatibility.md`
- `wiki/guides/add-an-agent.md`
- `wiki/guides/verify-with-real-agents.md`

## Last updated

2026-11-11
