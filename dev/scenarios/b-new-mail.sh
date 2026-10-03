#!/usr/bin/env bash
# Scenario (b): a new message produces a sill banner and a dock badge.
#
# `mailo watch`, the session's sync daemon, runs with no window. A fake IMAP server (the Twisted
# one in scripts/live-imapd.py) gains a message while the watch idles on it; the real sill, on a
# nested compositor, is the notification server and the dock. Checked:
#
#   1. dist/install.sh puts the desktop entry and the icon where sill reads them, and a second
#      run changes nothing.
#   2. The watch's first pass puts `count 0, count-visible false` on the launcher entry for
#      `application://mailo.desktop`, from one connection (the watch).
#   3. A message arrives: sill's banner column maps and draws, sill's `notifications status`
#      counts it, and the Notify call carried the app name `mailo`, the icon `mailo`, the
#      sender as the summary, the subject as the body, and the `desktop-entry` hint.
#   4. The launcher entry becomes `count 1, count-visible true`.
#
# Usage: dev/scenarios/b-new-mail.sh
set -uo pipefail
# shellcheck source=dev/scenarios/lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

notify_is() { [[ "$(last_notify)" == "$1"* ]]; }
kept_is() { [ "$(status_field kept)" = "$1" ]; }
banner_count_is() { [ "$(status_field banners)" = "$1" ]; }

build_all
ensure_compositor
new_session
check "1. dist/install.sh installs, and is idempotent" install_mailo
check "1. the desktop entry is where sill looks" test -f "$PREFIX_DIR/share/applications/mailo.desktop"
check "1. the icon is where sill looks" test -f "$PREFIX_DIR/share/icons/hicolor/scalable/apps/mailo.svg"
check "1. the unit is in place" test -f "$UNIT_DIR/mailo-watch.service"
start_imap
seed_account
start_sill
start_bus_monitor
start_watch

check "2. the first pass says the count: 0, hidden" wait_for 60 badge_is "0 false"
check "2. from one connection" test "$(badge_senders)" = 1

drop_mail "Lunch on Thursday"
check "3. the banner column draws" wait_for 60 banner_up
check "3. sill counts it (kept 1)" wait_for 20 kept_is 1
check "3. app name, icon, sender, subject, desktop-entry hint" \
  wait_for 20 notify_is "mailo|mailo|Grace Hopper|Lunch on Thursday|"
check "3. the desktop-entry hint rides along" bash -c "[[ \"$(last_notify)\" == *desktop-entry* ]]"
check "4. the launcher entry says 1, visible" wait_for 20 badge_is "1 true"
check "the watch is still running" kill -0 "$WATCH_PID"
finish "a new message gives a banner and a badge"
