# CI

## Purpose

What GitHub Actions runs, on which runners, and what stays outside CI.

## Scope

The three workflows under `.github/workflows/`. Release steps are in `wiki/guides/release.md`.

## Workflows

| Workflow | Trigger | Runner | What it does |
|---|---|---|---|
| `ci.yml` | push to `main`, pull requests (skipped when only `wiki/**`, `docs/**` or `*.md` changed) | `ubuntu-latest` | job `rust`: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`; job `ci-ok` fails unless `rust` succeeded |
| `portability.yml` | manual (`workflow_dispatch`) | `windows-latest`, `continue-on-error` | `cargo build --workspace`, `cargo test --workspace`; best-effort |
| `release.yml` | push of a tag `v*` | `ubuntu-latest`, `ubuntu-24.04-arm` | verifies the tag against the workspace version, builds the Linux CLI per architecture, writes `SHA256SUMS-linux.txt`, creates the release from the tag message |

Common settings: `permissions: contents: read` (`contents: write` only in `release.yml`),
`concurrency` cancels superseded `ci.yml` runs, `actions/cache` keys on `Cargo.lock`. Only
first-party `actions/*` actions (`checkout`, `cache`, `upload-artifact`, `download-artifact`, all v4)
and plain shell are used; the only credential is the built-in `github.token`. Dependabot proposes
weekly grouped updates for cargo and for the actions (`.github/dependabot.yml`).

## What runs outside CI

- macOS: the CLI build, the Swift app, `swift test` and the app bundle are built and checked only
  by `scripts/release.sh` on the maintainer's machine (reasons in `wiki/guides/release.md`). CI
  never exercises the macOS-only code (`schedule`, notifications) or the Swift app; the Linux job
  compiles the CLI without them.
- Windows: best-effort, run by hand from the Actions tab. Linux is best-effort too, but its checks
  run on every change.
- Workflow execution itself is `[UNVERIFIED]` until the first run on GitHub; the files were
  validated as YAML only.

## Source anchors

- `.github/workflows/ci.yml`
- `.github/workflows/portability.yml`
- `.github/workflows/release.yml`
- `.github/dependabot.yml`

## Rules

- MUST: Use only `actions/*` actions and shell; no third-party actions and no secrets besides `github.token`.
- MUST: Keep macOS and Swift out of workflows; extend `scripts/release.sh` instead.
- SHOULD: Keep the local check commands in `CLAUDE.md` identical to the `ci.yml` steps.

## Verification checklist

- [ ] Each workflow parses as YAML
- [ ] The `ci.yml` commands equal the commands in `CLAUDE.md`
- [ ] Runner labels in this page match the workflow files

## Related docs

- `wiki/guides/release.md`
- `wiki/architecture/file-structure.md`

## Last updated

2026-11-11
