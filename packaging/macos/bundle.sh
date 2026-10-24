#!/usr/bin/env bash
# Build mailo.app, and a .dmg holding it beside a link to /Applications. macOS only. Unsigned.
#
#   ./packaging/macos/bundle.sh
#
# Needs Xcode's command line tools (hdiutil, plutil, codesign, all part of macOS). Lands in
# target/macos/: mailo.app and mailo-<version>.dmg.
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$root"
target="${CARGO_TARGET_DIR:-$root/target}"
out="$target/macos"

for tool in hdiutil plutil codesign; do
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

# The icon, at every size an .icns holds, drawn on macOS's rounded square with its margin.
cp packaging/macos/mailo.icns "$app/Contents/Resources/mailo.icns"

# The launch agent Settings' "Check for mail when quit" registers with SMAppService, which finds it
# here by name (crates/mail-app/src/login_item.rs). Inside the seal, like Info.plist.
mkdir -p "$app/Contents/Library/LaunchAgents"
cp packaging/macos/watch.plist "$app/Contents/Library/LaunchAgents/io.github.PoHsuanLai.mailo.watch.plist"
plutil -lint "$app/Contents/Library/LaunchAgents/io.github.PoHsuanLai.mailo.watch.plist"

# An ad-hoc signature over the whole bundle, under the bundle identifier. The linker already signs
# the binary ad hoc, but under a name taken from its hash and with Info.plist and the icon outside
# the seal; sealing them under the identifier makes the bundle one thing macOS can recognise, which
# is what the keychain and the notification centre ask of an app. It is not a Developer ID: Gatekeeper
# still says the developer cannot be verified.
codesign --force --sign - --identifier io.github.PoHsuanLai.mailo "$app"
codesign --verify --strict "$app"

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
