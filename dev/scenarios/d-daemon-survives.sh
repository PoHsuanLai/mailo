#!/usr/bin/env bash
# Scenario (d): the sync daemon survives the window closing.
#
# `mailo watch` and the window run together; the window is closed (killed, the way a closed
# window ends its process); the watch keeps running, and mail that arrives afterwards still
# produces a banner and moves the badge.
#
#   1. With the watch running, the window does not show the launcher count itself: every update
#      on the launcher entry comes from one connection, before and after the window opens.
#   2. The window goes away: the watch is alive and its count is still on the bus.
#   3. A message arrives with no window: the banner, and the badge says 1.
#   4. A second watch (a person typing `mailo watch` while the unit runs) leaves quietly.
#
# Usage: dev/scenarios/d-daemon-survives.sh
set -uo pipefail
# shellcheck source=dev/scenarios/lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

kept_is() { [ "$(status_field kept)" = "$1" ]; }
gone() { ! kill -0 "$1" 2>/dev/null; }

build_all
ensure_compositor
new_session
start_imap
seed_account
start_sill
start_bus_monitor
start_watch
check "the first pass says the count" wait_for 60 badge_is "0 false"

start_window
sleep 6
check "1. one connection speaks for the launcher entry, window open" test "$(badge_senders)" = 1

kill "$WINDOW_PID"
check "2. the window is gone" wait_for 10 gone "$WINDOW_PID"
sleep 1
check "2. the watch is still running" kill -0 "$WATCH_PID"
check "2. the badge is still the watch's" test "$(badge_senders)" = 1

drop_mail "Sent while nobody was looking"
check "3. a banner, with no window" wait_for 60 banner_up
check "3. kept 1" wait_for 20 kept_is 1
check "3. the badge says 1" wait_for 20 badge_is "1 true"

in_env "$MAILO_BIN" watch >"$OUT_DIR/second-watch.log" 2>&1
check "4. a second watch leaves quietly (exit 0)" test $? = 0
check "4. and says why" grep -q "already running" "$OUT_DIR/second-watch.log"
check "4. the first is undisturbed" kill -0 "$WATCH_PID"
finish "the sync daemon survives the window closing"
