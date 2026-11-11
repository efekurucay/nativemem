# CI

## Purpose

What GitHub Actions runs, on which runners, and what stays outside CI.

## Scope

The workflows under `.github/workflows/`. Release steps are in `wiki/guides/release.md`.

## Workflows

| Workflow | Trigger | Runner | What it does |
|---|---|---|---|
| `ci.yml` | push to `main`, pull requests (skipped when only `wiki/**`, `docs/**` or `*.md` changed) | `ubuntu-latest` | job `rust`: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`; job `ci-ok` fails unless `rust` succeeded |
| `site.yml` | push to `main` or PR touching `site/**` or the workflow; manual | `ubuntu-latest` | Node 22, `npm ci`, Chromium install, Playwright tests with axe; stage static files and deploy to GitHub Pages only from `main`, never from PRs |
| `portability.yml` | manual (`workflow_dispatch`) | `windows-latest`, `continue-on-error` | `cargo build --workspace`, `cargo test --workspace`; best-effort |
| `release.yml` | push of a tag `v*`, or manual with an existing `tag` input | `ubuntu-latest`, `ubuntu-24.04-arm` | verifies the tag against the workspace version, builds the Linux CLI per architecture, writes `SHA256SUMS-linux.txt`, creates the release from the tag message |

Common settings: `permissions: contents: read` (`contents: write` only in `release.yml`;
`pages: write` and `id-token: write` only in the site's deployment job),
`concurrency` cancels superseded `ci.yml` runs, `actions/cache` keys on `Cargo.lock`. Only
first-party `actions/*` actions (`checkout`, `cache`, `upload-artifact`, `download-artifact`, all v4)
and the site's `setup-node@v4`, `upload-pages-artifact@v3`, `deploy-pages@v4` plus plain shell are used;
the only credentials are the built-in `github.token` and the Pages deployment OIDC token. Dependabot proposes
weekly grouped updates for cargo and for the actions (`.github/dependabot.yml`).

## What runs outside CI

- macOS: the CLI build, the Swift app, `swift test` and the app bundle are built and checked only
  by `scripts/release.sh` on the maintainer's machine (reasons in `wiki/guides/release.md`). CI
  never exercises the macOS-only code (`schedule`, notifications) or the Swift app; the Linux job
  compiles the CLI without them.
- Windows: not supported yet. The manual `portability.yml` run compiles the workspace and passes the core tests, while 22 of the 27 CLI integration tests fail with `Access is denied` (file replacement or locking). Run it by hand from the Actions tab to re-check. Linux is best-effort too, but its checks
  run on every change.
- Workflow execution itself is `[UNVERIFIED]` until the first run on GitHub; the files were
  validated as YAML only.

## Source anchors

- `.github/workflows/ci.yml`
- `.github/workflows/portability.yml`
- `.github/workflows/release.yml` (`RELEASE_TAG` selects the version tag for both builds and publication; manual dispatch uses the workflow from `main` without moving the tag)
- `.github/workflows/site.yml`
- `site/playwright.config.js`
- `site/tests/site.spec.js`
- `site/tests/server.py` (local static fixture; OS-sized accept queue supports parallel browser connection pools)
- `.github/dependabot.yml`

## Rules

- MUST: Use only `actions/*` actions and shell; no third-party actions and no stored secrets besides the built-in `github.token`. Pages uses a short-lived OIDC token.
- MUST: Upload only the site's HTML, CSS, JavaScript and `assets/` for deployment. Agent data, tests and `node_modules` are not website artifacts.
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
