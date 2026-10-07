#!/usr/bin/env bash
# Build mailo.app, and a .dmg holding it beside a link to /Applications. macOS only. Unsigned.
#
#   ./packaging/macos/bundle.sh
#
# Needs Xcode's command line tools (iconutil, hdiutil, plutil, all part of macOS) and `resvg`
# (`cargo install resvg`), which draws the icon from the one SVG every package uses. Lands in
# target/macos/: mailo.app and mailo-<version>.dmg.
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$root"
target="${CARGO_TARGET_DIR:-$root/target}"
out="$target/macos"
svg=packaging/icons/hicolor/scalable/apps/mailo.svg

for tool in resvg iconutil hdiutil plutil; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "bundle.sh: $tool is missing" >&2
        exit 1
    fi
done

# `path+file:///…/crates/mail-app#0.1.0`, or `…#mail-app@0.1.0`: the version is after the last
# `#` or `@`.
version="$(cargo pkgid -p mail-app | sed 's/.*[#@]//')"

# `quire-desktop` is our desktop's extras (the link to accountd over D-Bus): not in a macOS build.
cargo build --release --locked --no-default-features -p mail-app --bin mailo

rm -rf "$out"
app="$out/mailo.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$target/release/mailo" "$app/Contents/MacOS/mailo"
sed "s/@VERSION@/$version/g" packaging/macos/Info.plist >"$app/Contents/Info.plist"
plutil -lint "$app/Contents/Info.plist"

# The icon, drawn at every size an .icns holds.
iconset="$out/mailo.iconset"
mkdir -p "$iconset"
for size in 16 32 128 256 512; do
    resvg --width "$size" --height "$size" "$svg" "$iconset/icon_${size}x${size}.png"
    resvg --width "$((size * 2))" --height "$((size * 2))" "$svg" "$iconset/icon_${size}x${size}@2x.png"
done
iconutil -c icns "$iconset" -o "$app/Contents/Resources/mailo.icns"
rm -rf "$iconset"

# Signing and notarisation would go here, and are left out: they need an Apple Developer ID
# certificate and an App Store Connect key, which would come in as CI secrets (for example
# MACOS_CERTIFICATE_P12, MACOS_CERTIFICATE_PASSWORD, APPLE_ID, APPLE_TEAM_ID, APPLE_APP_PASSWORD),
# then `codesign --deep --options runtime --sign "Developer ID Application: …" "$app"`,
# `xcrun notarytool submit … --wait` on the .dmg, and `xcrun stapler staple`.

stage="$out/dmg"
mkdir -p "$stage"
cp -R "$app" "$stage/"
ln -s /Applications "$stage/Applications"
hdiutil create -volname mailo -srcfolder "$stage" -ov -format UDZO "$out/mailo-$version.dmg"
rm -rf "$stage"

echo "bundle.sh: $app and $out/mailo-$version.dmg"
