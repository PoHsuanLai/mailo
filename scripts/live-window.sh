#!/usr/bin/env bash
# Drive the real window and report what it showed.
#
# The window is the product, and until this existed nothing here had ever been checked the way
# its user runs it — a component-tree test proves the components agree with each other, not that
# WebKit renders them or that a keystroke reaches them. Three rounds and a retracted finding
# (FINDINGS F106/F107) went into learning that the obvious route, `xdotool` under
# `GDK_BACKEND=x11`, silently breaks the WebView's edit delivery and makes a working window look
# broken.
#
# So the page drives itself: `scripts/live-window/probe.js` goes into the head through
# `$MAILO_PROBE` (debug builds only — see `ui::probe`), types into the real search box, and posts
# what it sees to a loopback listener. No input tooling, no compositor, and it works with the
# screen locked.
#
# Usage: scripts/live-window.sh [probe.js]
#
# The seeded store contains mail. To look at a different state — an account added but not signed
# in, say — build it by running `mailo account add` against the directory rather than writing the
# rows: a hand-made one of those omitted `account_caps`, which `account add` always writes, and
# produced a first-run screen no user could ever see.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
root="$(dirname "$here")"
probe="${1:-$here/live-window/probe.js}"
port=18099

work="$(mktemp -d)"
trap 'kill "${listener:-}" "${app:-}" 2>/dev/null || true; rm -rf "$work"' EXIT

cargo build -q -p mail-app
MAILO_SEED_DIR="$work" cargo test -q -p mail-app --test seed_live -- --ignored >/dev/null

python3 "$here/live-window/listen.py" > "$work/report.log" 2>&1 &
listener=$!
sleep 1

# `GDK_BACKEND` unset deliberately: forcing x11 is what broke this before.
# `XDG_CONFIG_HOME` too, and just as deliberately: the window remembers its appearance there, and
# a probe that clicks the picker must not rewrite the look of the person running this.
env -u GDK_BACKEND XDG_DATA_HOME="$work" XDG_CONFIG_HOME="$work/config" MAILO_PROBE="$(cat "$probe")" \
    "$root/target/debug/mailo" > "$work/app.log" 2>&1 &
app=$!
sleep 12

echo "--- what the page reported ---"
cat "$work/report.log"

# Printing what the page said is not the same as checking it. This script used to exit 0 as long
# as *something* was reported, and it did exactly that while every stage showed an identical list
# — which is the evidence of F140, printed and passed over. If the list never changes across a
# search and a clear, the window is not responding to what is typed into it.
if [ "$probe" = "$here/live-window/probe.js" ]; then
    distinct="$(grep -o '"subjects": \[[^]]*\]' "$work/report.log" | sort -u | wc -l)"
    if [ "$distinct" -le 1 ]; then
        echo >&2
        echo "the list was identical at every stage: typing into the search box changed" >&2
        echo "nothing, so the window is not re-rendering. See FINDINGS F140." >&2
        exit 1
    fi
fi
# The window's own stderr, always. It used to be printed only when the page reported nothing,
# which is the case where it is least useful: a page that reports the *wrong* thing is exactly
# when you want to see what the process was saying while it did so.
if [ -s "$work/app.log" ]; then
    echo "--- what the window said ---"
    cat "$work/app.log"
fi
if [ ! -s "$work/report.log" ]; then
    echo "nothing reported. The window's own output:" >&2
    cat "$work/app.log" >&2
    exit 1
fi
