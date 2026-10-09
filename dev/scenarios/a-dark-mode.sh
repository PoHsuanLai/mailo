#!/usr/bin/env bash
# Scenario (a): a dark-mode switch restyles a running mailo.
#
# The real window, on a nested compositor, reads the desktop's one appearance file
# (`$XDG_CONFIG_HOME/quire/appearance.toml`, the store the shell and detent write). The file is
# changed from outside while the window runs, as detent's Appearance page or the control centre
# change it, and the window's own pixels are read back through the compositor (grim):
#
#   1. mailo's old `mailo/appearance.toml` (dark) and no desktop file: the window starts dark
#      (the one-time migration), `quire/appearance.toml` now exists, and mailo's own copy is
#      byte for byte what it was.
#   2. The desktop file says light: the window turns light, with no restart.
#   3. It says dark again: the window turns dark.
#   4. mailo wrote nothing of its own appearance in all that (its file is unchanged, and no
#      `appearance.toml` appeared beside the desktop's).
#
# The harness test (`tests/app/native_desktop_appearance.rs`) proves the restyle in the document; this
# proves the whole path: the file watch in a real process, the real renderer, the real pixels.
#
# Usage: dev/scenarios/a-dark-mode.sh
set -uo pipefail
# shellcheck source=dev/scenarios/lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

DARK_FILE=$'version = 1\n\n[appearance]\ntheme = "dark"\n'
LIGHT_FILE=$'version = 1\n\n[appearance]\ntheme = "light"\n'

# Bright with the interface drawn on it (a blank white frame is a window that has not painted yet).
bright() { [ "$(screen_luminance "$1")" -gt 150 ] && [ "$(screen_detail "$1")" -gt 4 ]; }
dim() { [ "$(screen_luminance "$1")" -lt 100 ] && [ "$(screen_detail "$1")" -gt 4 ]; }

build_all
ensure_compositor
new_session

mkdir -p "$CONFIG/mailo"
printf '%s' "$DARK_FILE" >"$CONFIG/mailo/appearance.toml"
OLD_SUM=$(sha256sum "$CONFIG/mailo/appearance.toml" | cut -d' ' -f1)

start_window
check "1. starts dark from mailo's old file" wait_for 60 dim start
check "1. the desktop's file now exists" test -s "$CONFIG/quire/appearance.toml"
check "1. mailo's own copy is untouched" \
  test "$(sha256sum "$CONFIG/mailo/appearance.toml" | cut -d' ' -f1)" = "$OLD_SUM"

printf '%s' "$LIGHT_FILE" >"$CONFIG/quire/appearance.toml"
check "2. the desktop says light: the running window turns light" wait_for 30 bright light
printf '%s' "$DARK_FILE" >"$CONFIG/quire/appearance.toml"
check "3. the desktop says dark: the running window turns dark" wait_for 30 dim dark

check "4. mailo wrote no appearance file of its own" \
  test "$(sha256sum "$CONFIG/mailo/appearance.toml" | cut -d' ' -f1)" = "$OLD_SUM"
check "4. the window is still the one it started" kill -0 "$WINDOW_PID"
finish "dark mode restyles a running mailo"
