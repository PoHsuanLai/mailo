# shellcheck shell=bash
# Shared by the cross-component scenarios (dev/scenarios/*.sh; sourced, never run): mailo, the real
# `mailo watch` and the real window, against the real sill, on a headless compositor (casement in
# the desktop jail, or shell-host's headless KWin with SILL_COMPOSITOR=kwin), with a private D-Bus,
# a scratch HOME and a fake IMAP server. Nothing here reaches the person's session, their
# ~/.config, their keyring or any mail.
#
# Nothing inside the jail calls cargo or the network, so everything is built beforehand, outside
# it (`prebuild`, which `dev/scenarios/all.sh --build` runs), and a scenario under the jail
# (SCN_PREBUILT=1) only checks that the artefacts are there.
#
# SAFETY, the rules of sill's dev/lib/bus.sh, which this file builds on:
# - ONE private dbus-daemon is both the session and the system bus of everything started, so
#   NetworkManager, BlueZ, UPower, logind and the Secret Service are simply absent, never the
#   person's own (`start_private_bus`, `require_private_system_bus`).
# - HOME, XDG_RUNTIME_DIR, XDG_CONFIG_HOME, XDG_DATA_HOME, XDG_STATE_HOME and XDG_CACHE_HOME are
#   scratch (`scratch_new`), and `in_env` hands them to every process it runs.
# - Credentials are plain files in the scratch tree (`MAILO_TEST_SECRETS_DIR`, a debug-build seam
#   of mail-runtime), so no keyring is asked, and the IMAP account is the fake server's.
# - sill runs with its fake DDC (`--fake-ddc`): no /dev/i2c-*.
# - Only the processes a script started are killed (`track`, `cleanup`), and the compositor is the
#   one this script started (sill's dev/lib/compositor.sh: start_compositor, stop_compositor).
#
# Needs: dbus-daemon, dbus-monitor, python3 with PIL, uv (for the fake server's Twisted, installed
# by prebuild), the sill checkout beside this one (../sill, or MAILO_SILL) with casement,
# shell-host, quire, blitz-kit, keycap and palmrest beside it, as sill's own scripts need, and for
# casement the desktop jail (../desktop/harness/jail.sh). MAILO_BIN and SILL_BIN name already
# built debug binaries.

SCN_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
MAILO_REPO="$(cd "$SCN_DIR/../.." && pwd)"
SILL_CHECKOUT="${MAILO_SILL:-$MAILO_REPO/../sill}"
[ -f "$SILL_CHECKOUT/dev/lib/all.sh" ] || {
  echo "no sill checkout at $SILL_CHECKOUT (set MAILO_SILL to one)" >&2
  exit 1
}
# shellcheck source=/dev/null
source "$SILL_CHECKOUT/dev/lib/all.sh"
# mailo's own target dir: CARGO_TARGET_DIR is sill's in sill's scripts, so it is never read here.
MAILO_TARGET="${MAILO_TARGET:-$MAILO_REPO/target}"
MAILO_BIN="${MAILO_BIN:-$MAILO_TARGET/debug/mailo}"
# What prebuild leaves for the jailed run: the fake server's venv and the path of the seeding
# fixture's test binary. Both live in the target dir, which the jail reads; its /tmp is its own.
PREBUILT="$MAILO_TARGET/scenarios-prebuilt"
VENV="${MAILO_VENV:-$PREBUILT/venv}"
SEED_BIN_FILE="$PREBUILT/seed-bin"

FAILED=0
TRACKED=()
# Logs and captures: the target dir in a checkout, the jail's own scratch inside one (the
# checkout is read-only there).
if [ -n "${JAIL_SCRATCH:-}" ]; then
  OUT_DIR="${OUT_DIR:-$JAIL_SCRATCH/mailo/$(basename "${0%.sh}")}"
else
  OUT_DIR="${OUT_DIR:-$MAILO_TARGET/scenarios/$(basename "${0%.sh}")}"
fi
mkdir -p "$OUT_DIR"

die() { echo "scenario: $*" >&2; exit 1; }
say() { echo "== $*" >&2; }
check() { # check NAME COMMAND...
  local name="$1"; shift
  if "$@"; then echo "PASS $name"; else echo "FAIL $name"; FAILED=1; fi
}

# track PID: a process this script started, to kill at the end.
track() { TRACKED+=("$1"); }

# ---- builds ----

# Everything a scenario runs, built outside the jail: sill with its debug feature, the mailo
# binary, the seeding fixture's test binary (its path kept in $SEED_BIN_FILE) and the fake
# server's venv. Casement's own tools come from its dev/bootstrap.sh.
prebuild() {
  say "build"
  mkdir -p "$PREBUILT"
  {
    (cd "$SILL_REPO" && cargo build -p sill --features debug) || return 1
    (cd "$MAILO_REPO" && cargo build --target-dir "$MAILO_TARGET" -p mail-app --bin mailo) || return 1
    (cd "$MAILO_REPO" && cargo test --target-dir "$MAILO_TARGET" -p mail-core --test seed_scenario --no-run \
      --message-format=json) \
      | python3 -c '
import json, sys
for line in sys.stdin:
    m = json.loads(line)
    if m.get("reason") == "compiler-artifact" and m.get("executable") and m["target"]["name"] == "seed_scenario":
        print(m["executable"])' >"$SEED_BIN_FILE" || return 1
    [ -s "$SEED_BIN_FILE" ] || return 1
    if [ ! -x "$VENV/bin/python" ]; then
      uv venv -q "$VENV" && uv pip install -q --python "$VENV/bin/python" twisted || return 1
    fi
  } >"$OUT_DIR/build.log" 2>&1
}

# Builds outside the jail; under it (SCN_PREBUILT=1) only checks that prebuild has run.
build_all() {
  if [ -z "${SCN_PREBUILT:-}" ]; then
    prebuild || die "the build failed (see $OUT_DIR/build.log)"
  fi
  local artefact
  for artefact in "$SILL_BIN" "$MAILO_BIN" "$VENV/bin/python" "$(cat "$SEED_BIN_FILE" 2>/dev/null)"; do
    [ -x "$artefact" ] || die "$artefact is missing: run dev/scenarios/all.sh --build (outside the jail)"
  done
}

# ---- the compositor ----

# Brings up the compositor (casement by default, SILL_COMPOSITOR=kwin for shell-host's headless
# KWin) and sets WL. The session's private bus comes after it, from new_session.
ensure_compositor() {
  say "compositor"
  start_compositor || die "no compositor (see $OUT_DIR)"
}

# ---- the session: bus, scratch tree, environment ----

# Sets BUS, the scratch tree, FAKE_DDC and IN_ENV_EXTRA (what every in_env gets beyond the scratch
# tree: the data dirs that make the scratch prefix visible, the credentials directory).
new_session() {
  say "session: private bus and scratch tree"
  scratch_new
  start_private_bus || die "no private bus"
  PREFIX_DIR="$SCRATCH/prefix"
  UNIT_DIR="$SCRATCH/units"
  SPOOL="$SCRATCH/spool"
  SECRETS="$SCRATCH/secrets"
  mkdir -p "$SPOOL" "$SECRETS" "$DATA/applications"
  use_fake_ddc "$CONFIG" >/dev/null
  # The host's X display is not ours: nothing here may reach it.
  IN_ENV_UNSET=(DISPLAY XAUTHORITY)
  IN_ENV_EXTRA=(
    "XDG_DATA_DIRS=$PREFIX_DIR/share:/usr/share"
    "MAILO_TEST_SECRETS_DIR=$SECRETS"
    "PATH=$(dirname "$MAILO_BIN"):$PATH"
  )
  # A wallpaper-less output is fine; sill only needs the compositor.
}

# The installer, run for real into the scratch prefix with a stub binary (the debug build is
# ~900 MB and the scenarios run it from target/debug): every file in place, and a second run
# changes nothing.
install_mailo() {
  say "dist/install.sh into $PREFIX_DIR"
  printf '#!/bin/sh\nexit 0\n' >"$SCRATCH/mailo-stub"
  chmod +x "$SCRATCH/mailo-stub"
  local run=(env MAILO_PREFIX="$PREFIX_DIR" MAILO_UNIT_DIR="$UNIT_DIR" MAILO_BIN="$SCRATCH/mailo-stub"
    "$MAILO_REPO/dist/install.sh" --no-build)
  "${run[@]}" >"$OUT_DIR/install.log" 2>&1 || { cat "$OUT_DIR/install.log" >&2; return 1; }
  "${run[@]}" >"$OUT_DIR/install-again.log" 2>&1 || return 1
  grep -q "unchanged: $PREFIX_DIR/share/applications/org.quire.Mail.desktop" "$OUT_DIR/install-again.log"
}

# ---- the fake mail server ----

# A Twisted IMAP server (scripts/live-imapd.py) that pushes a message dropped into $SPOOL to
# whoever is idling. Sets IMAP_PORT and IMAP_PID.
start_imap() {
  say "fake IMAP server"
  IMAP_PORT=$(python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')
  MAILO_EXTRA_MAIL="$SPOOL" "$VENV/bin/python" "$MAILO_REPO/scripts/live-imapd.py" "$IMAP_PORT" \
    >"$OUT_DIR/imap.log" 2>&1 &
  IMAP_PID=$!
  track "$IMAP_PID"
  for _ in $(seq 50); do
    (exec 3<>"/dev/tcp/127.0.0.1/$IMAP_PORT") 2>/dev/null && return 0
    sleep 0.2
  done
  die "the fake IMAP server never came up (see $OUT_DIR/imap.log)"
}

# The account for ada@example.test on that server, in the scratch data home.
seed_account() {
  say "seed the account"
  # The fixture's test binary, built by prebuild, reads nothing from the environment but the
  # three variables it is given.
  env MAILO_SEED_DIR="$DATA" MAILO_SCENARIO_IMAP_PORT="$IMAP_PORT" MAILO_TEST_SECRETS_DIR="$SECRETS" \
    "$(cat "$SEED_BIN_FILE")" --ignored >"$OUT_DIR/seed.log" 2>&1 || die "seeding failed (see $OUT_DIR/seed.log)"
}

# drop_mail SUBJECT: a message from grace@example.test appears on the server.
drop_mail() {
  local n
  n=$(ls "$SPOOL" | wc -l)
  cat >"$SPOOL/new-$n.eml" <<EOF
From: Grace Hopper <grace@example.test>
To: ada@example.test
Subject: $1
Date: $(date -R)
Message-ID: <scenario-$n-$$@example.test>
MIME-Version: 1.0
Content-Type: text/plain; charset=us-ascii

The body of "$1".
EOF
}

# ---- sill ----

# The daemon on the compositor, and what it names its surfaces.
start_sill() {
  say "sill"
  OUT_W=
  # A debug build on a loaded machine can take longer than launch_daemon's ten seconds to answer.
  launch_daemon 2>/dev/null || wait_for 90 sill ping >/dev/null 2>&1 \
    || die "sill did not start (see $OUT_DIR/daemon.log)"
  track "$DAEMON"
  OUTPUT=$(output_name) || die "no output"
  BANNERS="sill-banners-$OUTPUT"
}

# ---- the mailo processes ----

# start_watch: `mailo watch` as a session unit would run it. Sets WATCH_PID.
start_watch() {
  say "mailo watch"
  in_env "$MAILO_BIN" watch >"$OUT_DIR/watch.log" 2>&1 &
  WATCH_PID=$!
  track "$WATCH_PID"
}

# start_window [ARGS...]: the mailo window. Sets WINDOW_PID.
start_window() {
  in_env "$MAILO_BIN" "$@" >"$OUT_DIR/window.log" 2>&1 &
  WINDOW_PID=$!
  track "$WINDOW_PID"
}

# The watch has finished its pass and parked in IDLE. The fake server announces mail only to a
# connection already idling (it has no UIDNEXT to tell a late one), so a message dropped before this
# would wait for the five-minute poll.
watch_idling() { sleep 6; }

# ---- the bus, watched ----

# Records the notification calls and launcher-entry updates on the private bus into $BUS_LOG.
start_bus_monitor() {
  BUS_LOG="$OUT_DIR/bus.log"
  : >"$BUS_LOG"
  dbus-monitor --address "$BUS" \
    "type='method_call',interface='org.freedesktop.Notifications',member='Notify'" \
    "type='signal',interface='org.freedesktop.Notifications'" \
    "type='signal',interface='com.canonical.Unity.LauncherEntry'" \
    "type='method_call',interface='org.freedesktop.Application'" \
    >"$BUS_LOG" 2>&1 &
  track $!
}

# wait_for SECONDS COMMAND...: poll until COMMAND succeeds.
wait_for() {
  local limit=$(( $1 * 10 )); shift
  for _ in $(seq "$limit"); do "$@" && return 0; sleep 0.1; done
  return 1
}

# Runs a python SCRIPT over $BUS_LOG, already split into `blocks`: one string per message the bus
# monitor printed, each starting at its `signal ` or `method call ` line.
bus_py() { # bus_py SCRIPT
  python3 -c '
import re, sys
text = open(sys.argv[1]).read()
blocks = re.split(r"\n(?=signal |method call |method return |error )", text)
'"$1" "$BUS_LOG"
}

# The last `count` the launcher entry for mailo carried on the bus (empty when none).
last_badge() {
  bus_py '
last = ""
for b in blocks:
    if "LauncherEntry" in b and "string \"application://org.quire.Mail.desktop\"" in b:
        m = re.search(r"string \"count\"\s+variant\s+int64 (\d+)", b)
        v = re.search(r"string \"count-visible\"\s+variant\s+boolean (\w+)", b)
        if m:
            last = m.group(1) + " " + (v.group(1) if v else "?")
print(last)'
}

badge_is() { [ "$(last_badge)" = "$1" ]; }

# How many different connections sent the mailo launcher entry's updates (the watch alone: 1).
badge_senders() {
  bus_py '
senders = set()
for b in blocks:
    if "LauncherEntry" in b and "application://org.quire.Mail.desktop" in b:
        m = re.search(r"sender=(\S+)", b)
        if m: senders.add(m.group(1))
print(len(senders))'
}

# The arguments of the last Notify call as `app|icon|summary|body|hints`, or empty.
last_notify() {
  bus_py '
last = ""
for b in blocks:
    if b.startswith("method call") and "member=Notify" in b:
        strs = re.findall(r"^\s+string \"(.*)\"$", b, re.M)
        if len(strs) >= 4:
            hints = " ".join(re.findall(r"string \"([\w-]+)\"\s+variant", b))
            last = "|".join([strs[0], strs[1], strs[2], strs[3], hints])
print(last)'
}

# The token of the last ActivationToken signal the server sent, or empty.
last_token() {
  bus_py '
last = ""
for b in blocks:
    if b.startswith("signal") and "member=ActivationToken" in b:
        m = re.findall(r"^\s+string \"(.*)\"$", b, re.M)
        if m: last = m[0]
print(last)'
}

# The `Open` calls a client made on the running window: the uris, one per line.
open_calls() {
  bus_py '
for b in blocks:
    if b.startswith("method call") and "interface=org.freedesktop.Application" in b and "member=Open" in b:
        print(" ".join(re.findall(r"string \"(mailo:[^\"]*)\"", b)) + " | " + " ".join(re.findall(r"string \"(activation-token)\"", b)))'
}

# A field of `sill notifications status`: banners or kept.
status_field() {
  sill notifications status 2>/dev/null | python3 -c '
import json, sys
def find(v):
    if isinstance(v, dict):
        if "server" in v and "kept" in v:
            return v
        for x in v.values():
            r = find(x)
            if r: return r
    return None
r = find(json.loads(sys.stdin.read())) or {}
print(r.get(sys.argv[1], ""))' "$1" 2>/dev/null
}

# Whether the banner column is mapped and has drawn something.
banner_up() {
  local png="$OUT_DIR/banners.png"
  rm -f "$png"
  sill debug capture "$BANNERS" "$png" >/dev/null 2>&1 && [ -s "$png" ] && [ "$(opaque_pixels "$png")" -gt 1000 ]
}

# The mean luminance (0..255) of the middle of the output, as the compositor's `shot` draws it.
screen_luminance() {
  local png="$OUT_DIR/screen-$1.png"
  shot "$png" 2>/dev/null || return 1
  python3 - "$png" <<'PY'
import sys
from PIL import Image
img = Image.open(sys.argv[1]).convert("L")
w, h = img.size
box = img.crop((w * 3 // 10, h * 3 // 10, w * 7 // 10, h * 7 // 10))
px = box.tobytes()
print(sum(px) // len(px))
PY
}

# How much is drawn in the middle of the last screenshot taken for $1: the standard deviation of its
# luminance, which a blank frame lacks and an interface has.
screen_detail() {
  python3 - "$OUT_DIR/screen-$1.png" <<'PY'
import sys
from PIL import Image, ImageStat
img = Image.open(sys.argv[1]).convert("L")
w, h = img.size
print(int(ImageStat.Stat(img.crop((w * 3 // 10, h * 3 // 10, w * 7 // 10, h * 7 // 10))).stddev[0]))
PY
}

# The thread id the store holds for the conversation with subject $1, read from the scratch
# database with sqlite3's own CLI (read-only).
thread_of() {
  python3 - "$DATA/mailo/mail.db" "$1" <<'PY'
import sqlite3, sys
db = sqlite3.connect(f"file:{sys.argv[1]}?mode=ro", uri=True)
rows = db.execute("select thread from messages where subject = ?", (sys.argv[2],)).fetchall()
print(rows[0][0] if rows else "")
PY
}

# ---- the end ----

cleanup() {
  local pid
  for pid in "${TRACKED[@]}"; do
    kill "$pid" 2>/dev/null
  done
  sleep 0.5
  for pid in "${TRACKED[@]}"; do
    kill -9 "$pid" 2>/dev/null
  done
  [ -z "${DAEMON:-}" ] || stop_daemon >/dev/null 2>&1
  stop_compositor >/dev/null 2>&1
  stop_private_bus
  scratch_drop
}
trap cleanup EXIT

finish() {
  if [ "$FAILED" = 0 ]; then echo "SCENARIO GREEN: $1"; else echo "SCENARIO FAILED: $1 (logs in $OUT_DIR)"; fi
  exit "$FAILED"
}
