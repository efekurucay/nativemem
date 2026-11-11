#!/bin/bash
set -euo pipefail

START="$PWD"
cd "$(dirname "$0")/.."

VERSION="${VERSION:-$(sed -n 's/^version = "\(.*\)"$/\1/p' ../../Cargo.toml | head -n 1)}"
CLI="${NATIVEMEM_CLI:-../../target/release/nativemem}"
case "$CLI" in
    /*) ;;
    *) [ -z "${NATIVEMEM_CLI:-}" ] || CLI="$START/$CLI" ;;
esac
SIGN_IDENTITY="${CODESIGN_IDENTITY:--}"

ARCHS=()
if [ "${UNIVERSAL:-0}" = "1" ]; then
    ARCHS=(--arch arm64 --arch x86_64)
fi

swift build -c release ${ARCHS[@]+"${ARCHS[@]}"}

APP="build/nativemem.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleIdentifier</key><string>dev.nativemem.app</string>
    <key>CFBundleShortVersionString</key><string>$VERSION</string>
    <key>CFBundleVersion</key><string>$VERSION</string>
    <key>CFBundleName</key><string>nativemem</string>
    <key>CFBundleExecutable</key><string>nativemem</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleIconFile</key><string>AppIcon</string>
    <key>LSMinimumSystemVersion</key><string>14.0</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>NSPrincipalClass</key><string>NSApplication</string>
</dict>
</plist>
PLIST

cp "$(swift build -c release ${ARCHS[@]+"${ARCHS[@]}"} --show-bin-path)/Nativemem" "$APP/Contents/MacOS/nativemem"

if [ -f "$CLI" ]; then
    cp "$CLI" "$APP/Contents/MacOS/nativemem-cli"
fi

ICONSET="build/AppIcon.iconset"
rm -rf "$ICONSET"
mkdir -p "$ICONSET" "$APP/Contents/Resources"
cp ../../LICENSE "$APP/Contents/Resources/LICENSE"
for size in 16 32 128 256 512; do
    sips -z "$size" "$size" Resources/AppIcon.png --out "$ICONSET/icon_${size}x${size}.png" >/dev/null
    sips -z "$((size * 2))" "$((size * 2))" Resources/AppIcon.png --out "$ICONSET/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/AppIcon.icns"
rm -rf "$ICONSET"

codesign --force --deep -s "$SIGN_IDENTITY" "$APP"
echo "Built $APP ($VERSION)"
