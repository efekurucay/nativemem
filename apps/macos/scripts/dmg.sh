#!/bin/bash
# Package a built app with an Applications shortcut for drag-and-drop installation.
set -euo pipefail

if [ "$#" -ne 2 ]; then
    echo "usage: apps/macos/scripts/dmg.sh <nativemem.app> <output.dmg>" >&2
    exit 2
fi
APP="$1"
OUTPUT="$2"
[ -d "$APP/Contents" ] || { echo "error: $APP is not an app bundle" >&2; exit 1; }
[ ! -e "$OUTPUT" ] || { echo "error: $OUTPUT already exists" >&2; exit 1; }

STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
ditto "$APP" "$STAGE/nativemem.app"
ln -s /Applications "$STAGE/Applications"
cat > "$STAGE/Install.txt" <<'TEXT'
Drag nativemem.app to Applications, then open it from there.

Requires macOS 14 or later. Supports Apple silicon and Intel Macs.

This app is ad-hoc signed, not notarized. If macOS blocks the first launch,
open System Settings > Privacy & Security and choose Open Anyway for nativemem.
Only approve the app if you trust the download. SHA-256 checksums are included
in the GitHub release.

https://efekurucay.com/nativemem/
https://github.com/efekurucay/nativemem/releases
TEXT
hdiutil create -volname "nativemem" -fs HFS+ -srcfolder "$STAGE" -format UDZO "$OUTPUT"
hdiutil verify "$OUTPUT"
echo "Built $OUTPUT"
