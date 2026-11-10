# nativemem

`nativemem` is a deterministic Rust CLI (plus a small macOS app) that keeps each coding agent's own
memory, global instruction sections and chat sessions in sync with the other agents. There is no
shared store: every agent keeps reading its own files in its own format. Supported agents: Claude
Code, Codex, OpenCode, Pi. Hermes has research notes only and no adapter.

## Reading order

1. `wiki/README.md`
2. `wiki/CLAUDE.md`
3. `wiki/architecture/overview.md`
4. The page of the agent the task touches (`wiki/agents/`) and, if relevant, the kind (`wiki/kinds/`)

## Hard rules

- Never run the `nativemem` binary, the tests or the desktop app against the real `~/.claude`,
  `~/.codex`, `~/.pi`, `~/.config/opencode`, `~/.hermes` or `~/.nativemem`. Use temp directories and
  explicit roots (`--claude-home`, `--codex-home`, `--opencode-home`, `--pi-home`, `--state-dir`).
- CLI tests redirect `HOME` and pass every root explicitly (see `run` in `crates/cli/tests/cli.rs`).
- Never print secrets or session bodies, and never write inside a git repository.
- Every delete or overwrite backs up first; writes are atomic (`crates/core/src/fsx.rs`).
- Code, comments and docs describe the current state only; history lives in git. One canonical name
  per symbol, and a replaced implementation is removed, not kept beside its replacement.
- Update the wiki in the same task as the change (`wiki/WIKI-CREATOR.md` has the routing table).
- Commits are `type(scope): summary` (the subjects feed the release notes) and carry no AI attribution
  trailers (no `Co-Authored-By`, no "Generated with" footer).
- Keep changes surgical: every changed line traces to the request.

## Commands

```
cargo build --release                                   # binary: target/release/nativemem
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
cd apps/macos && swift test                             # app tests
apps/macos/scripts/bundle.sh                            # builds apps/macos/build/nativemem.app
scripts/release.sh X.Y.Z [--dry-run]                    # local release: checks, bump, macOS artifacts, notes, tag
scripts/demo-home.sh <tmp dir> [path-to-nativemem]      # synthetic agent homes for trying the CLI and app
```

Binary usage (on temp roots only): `nativemem status`, `items [--kind ...]`, `sync [--apply]`,
`doctor`, `delete`, `edit`, `schedule install|status|uninstall`. See `README.md`. When `doctor` reports
an agent that needs action, follow `wiki/runbooks/agent-update-action-required.md`.

## Related

- `README.md`, `docs/design.md`, `docs/desktop.md`, `CONTRIBUTING.md`
- Release and CI: `wiki/guides/release.md`, `wiki/architecture/ci.md`
- `AGENTS.md` is a symlink to this file.
