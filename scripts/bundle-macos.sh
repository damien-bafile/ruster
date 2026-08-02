#!/bin/bash
# Assemble ruster.app — the macOS bundle.
#
# A bare binary has no Dock icon, no app name in the menu bar, and no Finder
# identity, however the window is configured. That is what a bundle provides;
# `set_window_icon` alone cannot.
#
#   ./scripts/bundle-macos.sh [debug|release]     (default: release)
#
# Output: target/<profile>/bundle/ruster.app
set -euo pipefail

PROFILE="${1:-release}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="$ROOT/target/$PROFILE/ruster"
OUT="$ROOT/target/$PROFILE/bundle/ruster.app"
ICON="$ROOT/assets/ruster.icns"

if [ ! -x "$BIN" ]; then
    echo "no binary at $BIN — run: cargo build $([ "$PROFILE" = release ] && echo --release)" >&2
    exit 1
fi

# Version comes from the crate rather than being repeated here, or the two
# drift and the About box lies.
VERSION="$(awk -F'"' '/^version/ {print $2; exit}' "$ROOT/crates/ruster-bin/Cargo.toml")"

rm -rf "$OUT"
mkdir -p "$OUT/Contents/MacOS" "$OUT/Contents/Resources"
cp "$BIN" "$OUT/Contents/MacOS/ruster"

if [ -f "$ICON" ]; then
    cp "$ICON" "$OUT/Contents/Resources/ruster.icns"
else
    echo "warning: $ICON missing — the bundle will show the generic app icon" >&2
fi

cat > "$OUT/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>              <string>ruster</string>
    <key>CFBundleDisplayName</key>       <string>ruster</string>
    <key>CFBundleIdentifier</key>        <string>dev.ruster.editor</string>
    <key>CFBundleExecutable</key>        <string>ruster</string>
    <key>CFBundleIconFile</key>          <string>ruster</string>
    <key>CFBundlePackageType</key>       <string>APPL</string>
    <key>CFBundleShortVersionString</key><string>$VERSION</string>
    <key>CFBundleVersion</key>           <string>$VERSION</string>
    <key>LSMinimumSystemVersion</key>    <string>11.0</string>

    <!-- Without this the window renders at 1x and looks soft on every Mac
         made in the last decade. -->
    <key>NSHighResolutionCapable</key>   <true/>

    <!-- ruster is a GUI app; the raylib backend opens a real window. -->
    <key>LSBackgroundOnly</key>          <false/>
</dict>
</plist>
PLIST

# Ad-hoc signature. Unsigned bundles are refused outright on Apple silicon;
# this is not distribution signing, it is the minimum to let it launch here.
codesign --force --sign - "$OUT" 2>/dev/null || \
    echo "warning: codesign failed — the bundle may not launch" >&2

echo "built $OUT ($VERSION)"
