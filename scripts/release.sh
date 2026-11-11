#!/bin/bash
# Cuts a release locally: checks, version bump, macOS artifacts, release notes, commit and tag.
# Usage: scripts/release.sh <X.Y.Z> [--dry-run] [--with-doctor] [--yes] [--publish]
set -euo pipefail

REPO="https://github.com/efekurucay/nativemem"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

usage() {
    echo "usage: scripts/release.sh <X.Y.Z> [--dry-run] [--with-doctor] [--yes] [--publish]" >&2
    exit 2
}

VERSION=""
DRY=0
WITH_DOCTOR=0
ASSUME_YES=0
PUBLISH=0
for arg in "$@"; do
    case "$arg" in
        --dry-run) DRY=1 ;;
        --with-doctor) WITH_DOCTOR=1 ;;
        --yes) ASSUME_YES=1 ;;
        --publish) PUBLISH=1 ;;
        -*) usage ;;
        *) [ -z "$VERSION" ] || usage; VERSION="$arg" ;;
    esac
done
[ -n "$VERSION" ] || usage
TAG="v$VERSION"

die() { echo "error: $*" >&2; exit 1; }
step() { echo; echo "==> $*"; }

# Runs a command, or only prints it under --dry-run.
run() {
    if [ "$DRY" = 1 ]; then
        echo "[dry-run] would run: $*"
    else
        "$@" < /dev/null
    fi
}

current_version() {
    sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1
}

# Succeeds when $1 is a strictly greater semver than $2.
semver_greater() {
    local a="$1" b="$2" acore bcore apre bpre
    acore="${a%%-*}"; bcore="${b%%-*}"
    apre=""; bpre=""
    [ "$a" = "$acore" ] || apre="${a#*-}"
    [ "$b" = "$bcore" ] || bpre="${b#*-}"
    if [ "$acore" != "$bcore" ]; then
        [ "$(printf '%s\n%s\n' "$acore" "$bcore" | sort -t. -k1,1n -k2,2n -k3,3n | tail -n 1)" = "$acore" ]
        return
    fi
    [ "$apre" != "$bpre" ] || return 1
    [ -n "$bpre" ] || return 1
    [ -n "$apre" ] || return 0
    [ "$(printf '%s\n%s\n' "$apre" "$bpre" | sort -V | tail -n 1)" = "$apre" ]
}

CONVENTIONAL='^([a-z]+)(\(([^)]*)\))?(!)?: (.*)$'

# Release notes from the commits since the previous v* tag, grouped by conventional-commit type.
generate_notes() {
    local out="$1" prev range work hash subject body type scope summary section bullet breaking
    work="$(mktemp -d)"
    prev="$(git describe --tags --abbrev=0 --match 'v*' 2>/dev/null || true)"
    range="HEAD"
    [ -z "$prev" ] || range="$prev..HEAD"
    for hash in $(git log --no-merges --format=%h "$range"); do
        subject="$(git log -1 --format=%s "$hash")"
        body="$(git log -1 --format=%b "$hash")"
        type="other"; scope=""; summary="$subject"; breaking=0
        if [[ "$subject" =~ $CONVENTIONAL ]]; then
            type="${BASH_REMATCH[1]}"
            scope="${BASH_REMATCH[3]}"
            [ -z "${BASH_REMATCH[4]}" ] || breaking=1
            summary="${BASH_REMATCH[5]}"
        fi
        if [ "$type" = "chore" ] && [ "$scope" = "release" ]; then
            continue
        fi
        if printf '%s\n' "$body" | grep -qE '^BREAKING[ -]CHANGE'; then
            breaking=1
        fi
        case "$type" in
            feat) section="features" ;;
            fix) section="fixes" ;;
            docs) section="docs" ;;
            chore | ci | test | refactor | build) section="maintenance" ;;
            *) section="other" ;;
        esac
        bullet="- "
        [ -z "$scope" ] || bullet="- **$scope:** "
        bullet="$bullet$summary (\`$hash\`)"
        printf '%s\n' "$bullet" >> "$work/$section"
        if [ "$breaking" = 1 ]; then
            printf '%s\n' "$bullet" >> "$work/breaking"
        fi
    done

    {
        echo "## nativemem $TAG"
        echo
        local pair file title
        for pair in "breaking:Breaking changes" "features:Features" "fixes:Fixes" \
            "docs:Documentation" "maintenance:Maintenance" "other:Other"; do
            file="${pair%%:*}"; title="${pair#*:}"
            if [ -s "$work/$file" ]; then
                echo "### $title"
                echo
                cat "$work/$file"
                echo
            fi
        done
        if [ ! -s "$work/features" ] && [ ! -s "$work/fixes" ] && [ ! -s "$work/docs" ] \
            && [ ! -s "$work/maintenance" ] && [ ! -s "$work/other" ]; then
            echo "No changes since ${prev:-the start of the history}."
            echo
        fi
        if [ -n "${DOCTOR_LINES:-}" ]; then
            echo "### Verified against agent versions"
            echo
            printf '%s\n\n' "$DOCTOR_LINES"
        fi
        echo "### Install"
        echo
        echo "Download \`nativemem-macos-universal.dmg\`, open it and drag the app to Applications."
        echo "The ZIP is an alternative. Checksums are in the \`SHA256SUMS-*\` assets. Install instructions: $REPO#get-started"
        echo
        echo "The macOS app is ad-hoc signed and not notarized. If macOS blocks the first launch,"
        echo "use System Settings > Privacy & Security > Open Anyway for nativemem, only if you trust the download."
    } > "$out"
    rm -rf "$work"
}

# Agent name and version only, from `doctor --json`; never paths or content.
doctor_section() {
    local bin="$1" state json
    state="$(mktemp -d)"
    json="$("$bin" --state-dir "$state" doctor --json 2>/dev/null || true)"
    rm -rf "$state"
    printf '%s' "$json" | grep -oE '"id":"[a-z]+"|"version":(null|"[^"]*")' | paste - - | while IFS=$'\t' read -r id ver; do
        id="${id#\"id\":\"}"; id="${id%\"}"
        case "$ver" in
            'null' | '"version":null') ;;
            *) ver="${ver#\"version\":\"}"; ver="${ver%\"}"; echo "- $id $ver" ;;
        esac
    done
}

# ---------------------------------------------------------------- preconditions
step "Checking preconditions"
[[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]] || die "'$VERSION' is not a valid semver version"
CURRENT="$(current_version)"
[ -n "$CURRENT" ] || die "cannot read the workspace version from Cargo.toml"
semver_greater "$VERSION" "$CURRENT" || die "$VERSION is not greater than the current version $CURRENT"
[ "$(git rev-parse --abbrev-ref HEAD)" = "main" ] || die "not on the main branch"
[ -z "$(git status --porcelain)" ] || die "the working tree is not clean"
git rev-parse -q --verify "refs/tags/$TAG" >/dev/null && die "tag $TAG already exists"
echo "version $CURRENT -> $VERSION, tag $TAG, branch main, clean tree"
[ "$DRY" = 0 ] || echo "(dry run: nothing below is executed or changed)"

BUMPED=0
COMMITTED=0
cleanup() {
    if [ "$BUMPED" = 1 ] && [ "$COMMITTED" = 0 ]; then
        git checkout -- Cargo.toml Cargo.lock
        echo "release aborted: the version bump was reverted" >&2
    fi
}
trap cleanup EXIT

# ---------------------------------------------------------------- checks
step "Running checks"
run cargo fmt --all -- --check
run cargo clippy --workspace --all-targets -- -D warnings
run cargo test --workspace
if [ "$DRY" = 1 ]; then
    echo "[dry-run] would run: (cd apps/macos && swift test)"
else
    (cd apps/macos && swift test < /dev/null)
fi

# ---------------------------------------------------------------- bump
step "Bumping the workspace version to $VERSION"
if [ "$DRY" = 1 ]; then
    echo "[dry-run] would set version = \"$VERSION\" in Cargo.toml and run: cargo update --workspace"
else
    awk -v v="$VERSION" '!done && /^version = "/ { print "version = \"" v "\""; done = 1; next } { print }' \
        Cargo.toml > Cargo.toml.tmp
    mv Cargo.toml.tmp Cargo.toml
    BUMPED=1
    [ "$(current_version)" = "$VERSION" ] || die "version bump did not apply"
    cargo update --workspace
fi

# ---------------------------------------------------------------- macOS artifacts
step "Building the macOS artifacts into dist/"
TARGETS="aarch64-apple-darwin x86_64-apple-darwin"
STAGE=""
if [ "$DRY" = 1 ]; then
    for t in $TARGETS; do
        echo "[dry-run] would run: rustup target add $t (when missing)"
        echo "[dry-run] would run: cargo build --release --locked -p nativemem-cli --target $t"
    done
    echo "[dry-run] would run: lipo -create the two binaries into one universal nativemem"
    echo "[dry-run] would run: NATIVEMEM_CLI=<universal> VERSION=$VERSION UNIVERSAL=1 apps/macos/scripts/bundle.sh"
    echo "[dry-run] would write: dist/nativemem-macos-universal.dmg (app and Applications shortcut)"
    echo "[dry-run] would write: dist/nativemem-macos-universal.zip (ditto -c -k --keepParent)"
    echo "[dry-run] would write: dist/nativemem-cli-$TAG-macos-universal.tar.gz"
    echo "[dry-run] would write: dist/SHA256SUMS-macos.txt"
else
    rm -rf dist
    mkdir -p dist
    STAGE="$(mktemp -d)"
    installed="$(rustup target list --installed 2>/dev/null || true)"
    for t in $TARGETS; do
        if ! printf '%s\n' "$installed" | grep -qx "$t"; then
            rustup target add "$t" || die "cannot add the Rust target $t; run: rustup target add $t"
        fi
        cargo build --release --locked -p nativemem-cli --target "$t"
    done
    lipo -create -output "$STAGE/nativemem" \
        target/aarch64-apple-darwin/release/nativemem target/x86_64-apple-darwin/release/nativemem
    [ "$("$STAGE/nativemem" --version)" = "nativemem $VERSION" ] || die "the built binary does not report version $VERSION"

    NATIVEMEM_CLI="$STAGE/nativemem" VERSION="$VERSION" UNIVERSAL=1 apps/macos/scripts/bundle.sh
    apps/macos/scripts/dmg.sh apps/macos/build/nativemem.app dist/nativemem-macos-universal.dmg
    ditto -c -k --keepParent apps/macos/build/nativemem.app dist/nativemem-macos-universal.zip

    mkdir "$STAGE/cli"
    cp "$STAGE/nativemem" LICENSE "$STAGE/cli/"
    tar -C "$STAGE/cli" -czf "dist/nativemem-cli-$TAG-macos-universal.tar.gz" nativemem LICENSE
    (cd dist && shasum -a 256 nativemem-macos-universal.dmg nativemem-macos-universal.zip "nativemem-cli-$TAG-macos-universal.tar.gz" > SHA256SUMS-macos.txt)
fi

# ---------------------------------------------------------------- notes
step "Writing the release notes"
NOTES="dist/RELEASE_NOTES.md"
DOCTOR_LINES=""
if [ "$WITH_DOCTOR" = 1 ]; then
    if [ "$DRY" = 1 ]; then
        echo "[dry-run] would run: nativemem doctor --json (read-only, real agent homes, temporary state dir)"
    else
        echo "Running nativemem doctor on the real agent homes (read-only; the state directory is a temporary one)."
        DOCTOR_LINES="$(doctor_section "$STAGE/nativemem")"
    fi
fi
if [ "$DRY" = 1 ]; then
    NOTES="$(mktemp)"
fi
generate_notes "$NOTES"

while true; do
    echo
    echo "---------------- release notes ($NOTES) ----------------"
    cat "$NOTES"
    echo "---------------------------------------------------------"
    if [ "$DRY" = 1 ] || [ "$ASSUME_YES" = 1 ]; then
        break
    fi
    printf 'Use these notes? [y]es / [e]dit / [a]bort: '
    read -r answer || answer="a"
    case "$answer" in
        y | Y | yes) break ;;
        e | E | edit) ${VISUAL:-${EDITOR:-vi}} "$NOTES" ;;
        *) die "aborted at the release notes" ;;
    esac
done
[ "$DRY" = 0 ] || rm -f "$NOTES"

# ---------------------------------------------------------------- commit and tag
step "Committing and tagging $TAG"
if [ "$DRY" = 1 ]; then
    echo "[dry-run] would run: git add Cargo.toml Cargo.lock"
    echo "[dry-run] would run: git commit -m \"chore(release): $TAG\""
    echo "[dry-run] would run: git tag -a $TAG --cleanup=verbatim -F dist/RELEASE_NOTES.md"
else
    git add Cargo.toml Cargo.lock
    git commit -q -m "chore(release): $TAG"
    COMMITTED=1
    git tag -a "$TAG" --cleanup=verbatim -F "$NOTES"
    echo "created commit and annotated tag $TAG"
fi

# ---------------------------------------------------------------- next steps
MACOS_ASSETS="dist/nativemem-macos-universal.dmg dist/nativemem-macos-universal.zip dist/nativemem-cli-$TAG-macos-universal.tar.gz dist/SHA256SUMS-macos.txt"
step "Next steps"
if [ "$PUBLISH" = 1 ] && [ "$DRY" = 0 ]; then
    git push origin main "$TAG"
    echo "Waiting for the release workflow to create the release $TAG..."
    waited=0
    until gh release view "$TAG" >/dev/null 2>&1; do
        [ "$waited" -lt 3600 ] || die "the release $TAG did not appear within an hour; upload later: gh release upload $TAG $MACOS_ASSETS --clobber"
        sleep 20
        waited=$((waited + 20))
    done
    # shellcheck disable=SC2086
    gh release upload "$TAG" $MACOS_ASSETS --clobber
    echo "Release $TAG is published with the macOS assets."
else
    echo "Nothing was pushed or uploaded. Run:"
    echo
    echo "  git push origin main $TAG"
    echo "  gh release upload $TAG $MACOS_ASSETS --clobber   # once the release workflow has created the release"
    echo
    echo "or rerun with --publish to do both."
fi
