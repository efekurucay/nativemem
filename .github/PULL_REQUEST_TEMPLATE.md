## What and why

<!-- One or two sentences. -->

## Checklist

- [ ] `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` pass
- [ ] `swift test` in `apps/macos` passes when the app changed
- [ ] The wiki is updated (routing table in `wiki/WIKI-CREATOR.md`)
- [ ] No personal data in code, tests, fixtures or docs (use `/Users/x/...`, `acme`)
- [ ] No test or manual run touched a real agent home or `~/.nativemem`
- [ ] Commit messages follow `type(scope): summary`, with no AI attribution trailers
