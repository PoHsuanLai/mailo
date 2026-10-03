# shellcheck shell=bash
# Shared by the cross-component scenarios (dev/scenarios/*.sh; sourced, never run): mailo, the real
# `mailo watch` and the real window, against the real sill, on a nested compositor, with a private
# D-Bus, a scratch HOME and a fake IMAP server. Nothing here reaches the person's session, their
# ~/.config, their keyring or any mail.
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
# - Only the processes a script started are killed (`track`, `cleanup`); the nested compositor is
#   reused when one is up and stopped only when this script started it.
#
# Needs: a Wayland session to nest in, cosmic-comp, grim, dbus-daemon, dbus-monitor, python3 with
# PIL, uv (for the fake server's Twisted), and the sill checkout beside this one (../sill, or
# MAILO_SILL) with quire, shell-host, blitz-kit, keycap and palmrest beside it, as sill's own
# scripts need. MAILO_BIN and SILL_BIN name already built debug binaries.

SCN_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
MAILO_REPO="$(cd "$SCN_DIR/../.." && pwd)"
SILL_CHECKOUT="${MAILO_SILL:-$MAILO_REPO/../sill}"
[ -f "$SILL_CHECKOUT/dev/lib/all.sh" ] || {
  echo "no sill checkout at $SILL_CHECKOUT (set MAILO_SILL to one)" >&2
  exit 1
}
# shellcheck source=/dev/null
source "$SILL_CHECKOUT/dev/lib/all.sh"
MAILO_BIN="${MAILO_BIN:-$MAILO_REPO/target/debug/mailo}"
VENV="${MAILO_VENV:-${TMPDIR:-/tmp}/mailo-scenarios-venv}"
# What the nested compositor needs to render on Mesa (nested.sh sets the same itself). Not given to sill
# or mailo: forcing Mesa's EGL on sill's own GPU path segfaults it, and the accept scripts run both on
# the host's drivers.
MESA_ENV=(
  __EGL_VENDOR_LIBRARY_FILENAMES=/usr/share/glvnd/egl_vendor.d/50_mesa.json DRI_PRIME=1
  __GL_SYNC_TO_VBLANK=0 vblank_mode=0
)

FAILED=0
TRACKED=()
STARTED_COMPOSITOR=0
OUT_DIR="${OUT_DIR:-$MAILO_REPO/target/scenarios/$(basename "${0%.sh}")}"
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

build_all() {
  say "build"
  [ -x "$SILL_BIN" ] || (cd "$SILL_REPO" && cargo build -p sill --features debug) || die "sill did not build"
  (cd "$MAILO_REPO" && cargo build -p mail-app --bin mailo \
    && cargo test -p mail-core --test seed_scenario --no-run) >"$OUT_DIR/build.log" 2>&1 \
    || die "mailo did not build (see $OUT_DIR/build.log)"
}

# ---- the nested compositor ----

# Reuses the nested cosmic-comp when one is up; otherwise starts one (shell-host's nested.sh on
# Mesa) and adopts it, because nested.sh can exit before it has written its env file.
ensure_compositor() {
  say "nested compositor"
  if use_nested_socket 2>/dev/null; then return 0; fi
  local env_file="$SHELL_HOST_DEV/.nested-env" before after pid sock
  before=$(ls "${XDG_RUNTIME_DIR:?}" | grep -E '^wayland-[0-9]+$' | sort)
  env "${MESA_ENV[@]}" "$SILL_REPO/dev/nested.sh" start >"$OUT_DIR/nested.log" 2>&1 || true
  STARTED_COMPOSITOR=1
  for _ in $(seq 60); do
    if [ ! -f "$env_file" ]; then
      after=$(ls "$XDG_RUNTIME_DIR" | grep -E '^wayland-[0-9]+$' | sort)
      sock=$(comm -13 <(echo "$before") <(echo "$after") | head -1)
      if [ -n "$sock" ]; then
        pid=$(ss -xlp 2>/dev/null | grep "/$sock " | grep -o 'pid=[0-9]*' | head -1 | cut -d= -f2)
        [ -n "$pid" ] && printf 'WAYLAND_DISPLAY=%s\nNESTED_COMPOSITOR_PID=%s\nNESTED_XDG_CONFIG_HOME=%s/xdg-config\n' \
          "$sock" "$pid" "$SHELL_HOST_DEV" >"$env_file"
      fi
    fi
    use_nested_socket 2>/dev/null && return 0
    sleep 0.5
  done
  die "no nested compositor (see $OUT_DIR/nested.log)"
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
  # A wallpaper-less nested output is fine; sill only needs the compositor.
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
  grep -q "unchanged: $PREFIX_DIR/share/applications/mailo.desktop" "$OUT_DIR/install-again.log"
}

# ---- the fake mail server ----

# A Twisted IMAP server (scripts/live-imapd.py) that pushes a message dropped into $SPOOL to
# whoever is idling. Sets IMAP_PORT and IMAP_PID.
start_imap() {
  say "fake IMAP server"
  if [ ! -x "$VENV/bin/python" ]; then
    uv venv -q "$VENV" && uv pip install -q --python "$VENV/bin/python" twisted || die "no Twisted for the fake server"
  fi
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
  # Not through `in_env`: cargo needs the real HOME for its toolchain, and the fixture reads
  # nothing from the environment but the three variables it is given.
  env MAILO_SEED_DIR="$DATA" MAILO_SCENARIO_IMAP_PORT="$IMAP_PORT" MAILO_TEST_SECRETS_DIR="$SECRETS" \
    cargo test --manifest-path "$MAILO_REPO/Cargo.toml" -p mail-core --test seed_scenario \
    -- --ignored >"$OUT_DIR/seed.log" 2>&1 || die "seeding failed (see $OUT_DIR/seed.log)"
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

# The daemon on the nested compositor, and what it names its surfaces.
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
    if "LauncherEntry" in b and "string \"application://mailo.desktop\"" in b:
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
    if "LauncherEntry" in b and "application://mailo.desktop" in b:
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

# The mean luminance (0..255) of the middle of the nested output, as grim draws it.
screen_luminance() {
  local png="$OUT_DIR/screen-$1.png"
  WAYLAND_DISPLAY="$WL" grim "$png" 2>/dev/null || return 1
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
  if [ "$STARTED_COMPOSITOR" = 1 ]; then "$SHELL_HOST_DEV/nested.sh" stop >/dev/null 2>&1; fi
  stop_private_bus
  scratch_drop
}
trap cleanup EXIT

finish() {
  if [ "$FAILED" = 0 ]; then echo "SCENARIO GREEN: $1"; else echo "SCENARIO FAILED: $1 (logs in $OUT_DIR)"; fi
  exit "$FAILED"
}
