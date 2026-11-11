# Contributing

Thanks for helping. The rules of this repo are short and strict because the tool writes into other
programs' files.

## Before you start

- Read `CLAUDE.md` and `wiki/README.md`. The wiki is the canonical description of the behaviour.
- Hard rules: never run the binary, the tests or the app against real agent homes (`~/.claude`,
  `~/.codex`, `~/.pi`, `~/.config/opencode`, `~/.hermes`, `~/.nativemem`); use temp directories and
  explicit roots. Never print secrets or session bodies. Repository writes are limited to project
  skill directories; `.gitignore` and git tracking remain the user's policy. Every delete or
  overwrite backs up first.
- `scripts/demo-home.sh <dir> [path-to-nativemem]` builds a synthetic environment to try changes on.
- No personal data in code, tests, fixtures or docs: use `/Users/x/...` and `acme`.
- Code and docs describe the current state only; history lives in git.

## Checks

```
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cd apps/macos && swift test      # when the app changed
```

CI runs the Rust checks on Linux. The macOS app and its tests run only on the maintainer's
machine (`scripts/release.sh`), so run `swift test` yourself when you change the app.

## Startup performance

```sh
cargo build --release
python3 scripts/benchmark-startup.py target/release/nativemem --size small
python3 scripts/benchmark-startup.py target/release/nativemem --size large
# Compare the commands profile of another nativemem build with the snapshot profile:
python3 scripts/benchmark-startup.py target/release/nativemem --compare /tmp/nativemem-baseline
```

The script owns a temporary fixture tree, redirects `HOME`, passes every root explicitly and gives
probes an empty executable search path. It never runs an agent. Paired comparisons alternate order on
the exact same files. Output contains fixture counts and millisecond median/min/max only. The large
fixture has 80 nine-file skills per agent, 200 memories and 160 sessions of 100 turns. Five samples
are the default; filesystem caches are not flushed. `first_content` covers the initial core reads;
`content_with_sessions` includes an immediate session listing. Neither measures rendered app launch,
hub loading or upstream network latency. Use Instruments for main-thread work and cold app launches.

Measure before changing an implementation and remeasure the same workload. Keep disk observation
fresh before sync rather than substituting a persistent metadata/hash cache. Relevant references:
[Apple's performance feedback loop](https://developer.apple.com/videos/play/wwdc2023/10160/),
[launch-time measurement and deferred work](https://developer.apple.com/documentation/xcode/reducing-your-app-s-launch-time),
[main-thread responsiveness](https://developer.apple.com/documentation/xcode/improving-app-responsiveness),
[the USE method](https://www.brendangregg.com/usemethod.html) and
[SQLite's in-process query costs](https://sqlite.org/np1queryprob.html).

## Landing page and screenshots

The static website lives in `site/`. Run `npm ci`, `npx playwright install chromium` and `npm test`
there for the browser and accessibility checks; `site/README.md` covers preview and Pages deployment.
No Node runtime is needed to serve the page.

README and website screenshots must show the current app using synthetic data only. Generate a demo
with `scripts/demo-home.sh`, redirect `HOME`, and set **every** printed agent root plus the state root
before launching the app. Capture its window, not the whole desktop. Never publish personal agent
paths, private conversations, tokens or another app's contents. The assets in `docs/images/` are the
PNG originals; `site/assets/` contains WebP copies for the website.

## Commits and pull requests

Commit subjects follow `type(scope): summary`, for example `fix(codex): skip locked databases`.
Types: `feat`, `fix`, `docs`, `chore`, `ci`, `test`, `refactor`, `build`; a `!` after the type or a
`BREAKING CHANGE:` line in the body marks a breaking change. The convention matters because
`scripts/release.sh` builds the release notes from the commit subjects. Do not add AI attribution
trailers (`Co-Authored-By`, "Generated with") to commits or pull request text.

Update the wiki in the same pull request; `wiki/WIKI-CREATOR.md` routes each kind of change to
its page.

## Adding an agent

Follow `wiki/guides/add-an-agent.md`: adapter, probes, test fixtures, wiki page and support table.

## Security

Report vulnerabilities privately, see `SECURITY.md`.
