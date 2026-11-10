# Agent rules (wiki is canonical)

## Purpose

Rules for any coding agent or engineer changing `nativemem`. Read at the start of every session.

## Scope

All work in this repo: Rust core and CLI, the macOS app, tests, docs and this wiki.

## Reading order

1. `wiki/README.md`
2. `wiki/architecture/overview.md`
3. `wiki/architecture/safety.md`
4. The page of the kind (`wiki/kinds/`) or agent (`wiki/agents/`) the task touches
5. `wiki/guides/` when adding an agent or a kind, or when verifying against real agents
6. `wiki/runbooks/agent-update-action-required.md` when `nativemem doctor` reports ActionRequired

## Invariants

- **Never touch real agent state.** Do not run the `nativemem` binary, the tests or the desktop app
  against the real `~/.claude`, `~/.codex`, `~/.pi`, `~/.config/opencode`, `~/.local/share/opencode`, `~/.hermes` or
  `~/.nativemem`. Use temp directories and explicit roots (`--claude-home`, `--codex-home`,
  `--opencode-home`, `--opencode-data-home`, `--pi-home`, `--state-dir`). Anchor: `crates/cli/tests/cli.rs` (the `run`
  helper redirects `HOME`, removes every root variable and passes temp roots).
- **Adapters take their root as a constructor argument.** Only the CLI resolves defaults
  (`resolve` in `crates/cli/src/main.rs`).
- **The engine knows nothing about agent formats.** Format knowledge lives in
  `crates/core/src/adapters/`. Anchor: outside its tests, `crates/core/src/engine.rs` imports nothing from `adapters`.
- **Capabilities are declared, not guessed.** `Adapter::supports(kind, scope)` decides what an agent
  stores; the engine never plans an item for an agent that cannot hold it.
- **Every adapter ships read-only probes** (`Adapter::probes`) for the formats it depends on. A failing
  probe pauses the kinds it protects for that agent. Anchor: `wiki/architecture/compatibility.md`.
- **Memory and instruction items replicate faithfully in both directions, deletes included, with
  backups.** Sessions are snapshots: deletes and edits never propagate.
  See `wiki/decisions/session-snapshot-policy.md`.
- **Nothing is written inside a git repository.** Only agent home directories and the state
  directory are written.
- **Agent-owned state is never edited.** Codex SQLite files (except the `threads` rows of `codex_threads.rs`), OpenCode's `opencode.db` (except the rows of `opencode_sessions.rs`), `hooks.json`, `config.toml`, generated
  `memories/MEMORY.md` and `memory_summary.md`, and any file that holds secrets. The one config exception is
  the `nativemem` MCP entry that `nativemem mcp install` adds to each agent's MCP config and
  `mcp uninstall` removes, written only through `crates/hub/src/register.rs`
  (`wiki/decisions/mcp-hub.md`).
- **Secrets are never printed or stored.** The ledger holds ids, hashes and paths, not entry text.
  Do not paste file contents of agent homes into output, logs, tests or wiki pages.
- **Every delete or overwrite backs up first** to `<state dir>/backups/<UTC stamp>/<agent>/<relative path>`.
- **Code and docs describe the current state only.** One canonical name per symbol. No history words
  in code, comments, docs or memory; history lives in git.
- **No placeholder wiki pages.** A page exists only with real content and every mandatory section.
- **Unverified agent behaviour is tagged `[UNVERIFIED]`** and stays tagged until a real run or a
  test confirms it. Writing into a real agent's store is rolled out in stages with the user
  verifying in the real agent (`wiki/guides/verify-with-real-agents.md`).

## Commands

```
cargo build --release
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
cd apps/macos && swift test
apps/macos/scripts/bundle.sh
scripts/release.sh X.Y.Z [--dry-run]
scripts/demo-home.sh <tmp dir> [path-to-nativemem]
```

Build and test commands are safe: tests use temp directories. `scripts/release.sh` is run by the maintainer only; `--with-doctor` reads the real agent homes. Running `target/release/nativemem`
without explicit temp roots is not.

## Wiki discipline

When behaviour, a format, a safety rule or a decision changes, update the wiki in the same task.
Routing: `wiki/WIKI-CREATOR.md`. Quality bar: `wiki/architecture/context-quality.md`.
Commits are `type(scope): summary`, which feeds the release notes (`wiki/guides/release.md`), and carry no AI attribution trailers.

## Source anchors

- `CLAUDE.md`
- `crates/cli/tests/cli.rs`
- `crates/cli/src/main.rs`
- `crates/core/src/engine.rs`
- `docs/design.md`

## Rules

- MUST: Follow the invariants above in every task.
- MUST: Name the files you plan to touch before a non-trivial edit.
- SHOULD: Report pre-existing drift you notice between code and docs instead of fixing it unasked.

## Verification checklist

- [ ] No test or manual run reads or writes a real agent home
- [ ] `cargo test --workspace`, clippy and fmt pass; `swift test` passes when the app changed
- [ ] The wiki page that owns the changed behaviour is updated, with its `Last updated`

## Related docs

- `wiki/README.md`
- `wiki/WIKI-CREATOR.md`
- `wiki/architecture/safety.md`

## Last updated

2026-11-11
