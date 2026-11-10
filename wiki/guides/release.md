# Release

## Purpose

How to cut a release of `nativemem`: the local script, what the release workflow adds, artifact
names, versioning, release notes, signing and rollback.

## Scope

Tagged releases on GitHub. The CI checks of pull requests are in `wiki/architecture/ci.md`.

## Why macOS is built locally

GitHub Actions builds only Linux binaries. The macOS CLI and the desktop app are built on the
maintainer's machine by `scripts/release.sh`. There is a single maintainer, macOS runners cost
more than Linux runners, and a local build uses the exact toolchain (Xcode, Swift 6) the app is
developed with. The local checks in `scripts/release.sh` (fmt, clippy, `cargo test --workspace`,
`swift test`) are therefore the gate for macOS and Swift code: CI never builds or tests the Swift
app.

## Cut a release

Run on `main` with a clean working tree, from the repo root:

```
scripts/release.sh 0.2.0 --dry-run     # prints every step, changes nothing
scripts/release.sh 0.2.0               # does the work, stops before pushing
git push origin main v0.2.0
gh release upload v0.2.0 dist/nativemem-macos-universal.zip dist/nativemem-cli-v0.2.0-macos-universal.tar.gz dist/SHA256SUMS-macos.txt --clobber
```

The script:

1. Refuses unless the version is valid semver and greater than the workspace version, the branch
   is `main`, the tree is clean and the tag does not exist.
2. Runs `cargo fmt --all -- --check`, clippy with `-D warnings`, `cargo test --workspace` and
   `swift test` in `apps/macos`.
3. Sets the workspace version in `Cargo.toml` and refreshes `Cargo.lock`.
4. Builds the CLI for `aarch64-apple-darwin` and `x86_64-apple-darwin` (it runs `rustup target add`
   for a missing target), joins them with `lipo`, builds the app with `UNIVERSAL=1` and that CLI,
   and writes the files below into `dist/` (ignored by git).
5. Writes the release notes, shows them and asks to confirm, edit (`$VISUAL` or `$EDITOR`) or abort.
6. Commits `chore(release): vX.Y.Z` and creates the annotated tag `vX.Y.Z` whose message is the
   reviewed notes. An abort before the commit restores `Cargo.toml` and `Cargo.lock`.
7. Prints the push and upload commands. It never pushes or uploads on its own.

Flags: `--dry-run`; `--yes` accepts the notes without asking; `--with-doctor` adds the agent
versions section (below); `--publish` runs the push, waits with `gh release view` until the
workflow has created the release, then uploads the macOS assets.

Pushing the tag starts `.github/workflows/release.yml`, which builds the Linux binaries and
creates the release. The macOS assets are uploaded afterwards, so a release is complete only
after the `gh release upload` step.

## Release notes

The notes are generated locally from the commit subjects between the previous `v*` tag and `HEAD`
(from the first commit when there is no tag). A subject `type(scope): summary` becomes one bullet
with the short hash and no author. Groups: Breaking changes (a `!` after the type or a
`BREAKING CHANGE` line in the body; such a commit also appears under its own type), Features
(`feat`), Fixes (`fix`), Documentation (`docs`), Maintenance (`chore`, `ci`, `test`, `refactor`,
`build`) and Other (everything else, including subjects without a type). `chore(release)` commits
are dropped. A fixed footer points to the install instructions, the `SHA256SUMS-*` assets and the
Gatekeeper command for the app.

With `--with-doctor` the script runs `nativemem doctor --json` of the fresh binary on the real
agent homes, read-only and with a temporary state directory, and lists agent id and version only,
under "Verified against agent versions". It prints nothing else from the output.

The reviewed text is written to `dist/RELEASE_NOTES.md` and becomes the body of the annotated tag
(`git tag -a vX.Y.Z --cleanup=verbatim -F dist/RELEASE_NOTES.md`), so the notes live in git; there
is no changelog file. The release workflow reads them back with
`git tag -l --format='%(contents)' vX.Y.Z` and passes them to `gh release create --notes-file`.
To change the notes of a published release, edit the release on GitHub.

## Artifacts

| File | Built by | Content |
|---|---|---|
| `nativemem-macos-universal.zip` | `scripts/release.sh` | `nativemem.app` (arm64 and x86_64), embeds the CLI |
| `nativemem-cli-<tag>-macos-universal.tar.gz` | `scripts/release.sh` | `nativemem` and `LICENSE` |
| `SHA256SUMS-macos.txt` | `scripts/release.sh` | checksums of the two macOS files |
| `nativemem-cli-<tag>-linux-x86_64.tar.gz` | release workflow | `nativemem` and `LICENSE` |
| `nativemem-cli-<tag>-linux-aarch64.tar.gz` | release workflow | `nativemem` and `LICENSE` |
| `SHA256SUMS-linux.txt` | release workflow | checksums of the Linux files |

Windows has no release artifact; it builds from source and its build is checked by the manual
`portability` workflow.

## Versioning

Semantic versioning. The version lives in `[workspace.package]` of `Cargo.toml` and is inherited by
both crates; `nativemem --version` prints it and `scripts/release.sh` writes it into
`CFBundleShortVersionString` and `CFBundleVersion` of the app. The tag is `v` plus that version.
A tag with a hyphen (`v0.3.0-rc.1`) creates a GitHub prerelease. The release workflow fails when
the tag differs from the workspace version of the tagged commit.

## Signing and Gatekeeper

The app is ad-hoc signed (`codesign -s -`, the default of `CODESIGN_IDENTITY` in
`apps/macos/scripts/bundle.sh`) and not notarized, because there is no Apple Developer account.
Gatekeeper blocks the first launch of a downloaded copy; users run
`xattr -dr com.apple.quarantine nativemem.app` or right-click the app and choose Open. A future
notarization needs: an Apple Developer Program membership, a Developer ID Application certificate
passed as `CODESIGN_IDENTITY`, the hardened runtime option on `codesign`, submission of the zip with
`xcrun notarytool submit --wait` and `xcrun stapler staple` on the app before zipping. None of this
is set up or exercised in the repo `[UNVERIFIED]`.

## Roll back a bad release

- Not pushed yet: `git tag -d vX.Y.Z` and `git reset --hard HEAD~1` (drops the release commit),
  then `rm -rf dist`.
- Pushed: `gh release delete vX.Y.Z --cleanup-tag --yes` removes the release and the remote tag, and
  `git tag -d vX.Y.Z` the local one. If the release commit was pushed, add a revert commit; never
  force-push `main`.
- Ship the fix as the next patch version; a published version number is not reused.

## Source anchors

- `scripts/release.sh`
- `apps/macos/scripts/bundle.sh`
- `.github/workflows/release.yml`
- `Cargo.toml`
- `.gitignore`

## Rules

- MUST: Cut releases only through `scripts/release.sh`, from a clean `main`.
- MUST: Commit subjects follow `type(scope): summary`; they become the release notes.
- MUST: Never push or upload before reading the notes and the artifact list.
- SHOULD: Run `--dry-run` first.
- SHOULD: Keep the Gatekeeper note in the footer and in `README.md` while the app is not notarized.

## Verification checklist

- [ ] `nativemem --version` of the built binary equals the tag without the `v`
- [ ] `dist/` holds the three macOS files and `RELEASE_NOTES.md`
- [ ] `git tag -n99 vX.Y.Z` shows the reviewed notes
- [ ] After the workflow, the release lists the Linux tarballs, both checksum files and the macOS assets

## Related docs

- `wiki/architecture/ci.md`
- `wiki/WIKI-CREATOR.md`
- `README.md`

## Last updated

2026-11-11
