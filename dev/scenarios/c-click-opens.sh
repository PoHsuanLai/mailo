#!/usr/bin/env bash
# Scenario (c): clicking the banner opens the message.
#
# Two cases, one per way a click can find mailo.
#
#   Without a window. Click the banner of a new message: sill emits `ActivationToken`, then
#   `ActionInvoked default`; `mailo watch` starts `mailo open <thread>` with that token in
#   XDG_ACTIVATION_TOKEN and DESKTOP_STARTUP_ID, and the thread is the message's.
#
#   With a window. The window owns `io.github.PoHsuanLai.mailo` and answers
#   `org.freedesktop.Application`. Click a second banner: `mailo open <thread>` finds the window,
#   calls `Open(["mailo:thread/<id>"], {activation-token})` on it and exits; no second mailo
#   keeps running; the window says it opened that conversation.
#
# Usage: dev/scenarios/c-click-opens.sh
set -uo pipefail
# shellcheck source=dev/scenarios/lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

kept_is() { [ "$(status_field kept)" = "$1" ]; }
# The pid of a `mailo open <thread>` the watch started, if one is running.
child_open() { pgrep -P "$WATCH_PID" -f "open $1" | head -1; }
has_child_open() { [ -n "$(child_open "$1")" ]; }
env_of() { tr '\0' '\n' <"/proc/$1/environ" | sed -n "s/^$2=//p"; }
window_said() { grep -q "$1" "$OUT_DIR/window.log" 2>/dev/null; }
click_banner() { sill debug click "$BANNERS" 220 76 >>"$OUT_DIR/click.log" 2>&1; }

build_all
ensure_compositor
new_session
start_imap
seed_account
start_sill
start_bus_monitor
start_watch
wait_for 60 badge_is "0 false" || echo "(the first pass was slow)" >&2

# ---- no window ----
drop_mail "Plans for the weekend"
check "banner drawn" wait_for 60 banner_up
THREAD=$(wait_for 20 test -n "$(thread_of 'Plans for the weekend')" && thread_of 'Plans for the weekend')
check "the message is stored, with a thread" test -n "$THREAD"
click_banner
check "the click starts mailo open <thread>" wait_for 20 has_child_open "$THREAD"
OPEN_PID=$(child_open "$THREAD")
TOKEN=$(last_token)
check "sill sent an activation token" test -n "$TOKEN"
check "the started window has that token" test "$(env_of "${OPEN_PID:-0}" XDG_ACTIVATION_TOKEN)" = "$TOKEN"
check "and the startup id" test "$(env_of "${OPEN_PID:-0}" DESKTOP_STARTUP_ID)" = "$TOKEN"
[ -z "$OPEN_PID" ] || track "$OPEN_PID"
check "that window is the running one (it owns the name)" \
  wait_for 40 bash -c "gdbus call --address '$BUS' --dest org.freedesktop.DBus --object-path /org/freedesktop/DBus --method org.freedesktop.DBus.NameHasOwner io.github.PoHsuanLai.mailo | grep -q true"
sill notifications dismiss-all >/dev/null 2>&1

# ---- with a window ----
drop_mail "Dinner on Friday"
check "banner drawn again" wait_for 60 banner_up
THREAD2=$(wait_for 20 test -n "$(thread_of 'Dinner on Friday')" && thread_of 'Dinner on Friday')
check "second message stored" test -n "$THREAD2"
click_banner
check "the click reached the running window over D-Bus" \
  wait_for 30 bash -c "$(declare -f open_calls bus_py); BUS_LOG='$BUS_LOG'; open_calls | grep -q 'mailo:thread/$THREAD2 | activation-token'"
check "the window opened that conversation" wait_for 30 window_said "opened conversation $THREAD2"
check "no second mailo is left running" test -z "$(child_open "$THREAD2")"
finish "a banner click opens the message"
