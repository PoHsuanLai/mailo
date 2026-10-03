#!/usr/bin/env bash
# Scenario (e): the desktop's intent router can start mailo, and only the router can use it.
#
# No window, no compositor, no mail server: this is the D-Bus activation path of `mailo intents`
# (the Rust tests run the provider on a private bus with the router played by the test; this runs
# the binary as installed). Checked:
#
#   1. dist/install.sh puts the intents manifest and the D-Bus service file where the session
#      reads them, naming the binary it installed.
#   2. A call to `org.quire.Mail` on a bus with nothing running starts `mailo intents` through
#      that service file, and `Summon`, the one member anybody may call, answers "declined".
#   3. The provider is running, in the scratch home and never the person's.
#   4. A caller that does not own `org.quire.Intents1` is refused `Perform`, `Search` and the rest.
#   5. A second `mailo intents` finds the name taken and leaves quietly (exit 0).
#   6. The provider ends with the bus, which is the session.
#
# The bus is started in the scratch environment, because a service the bus starts inherits the
# bus's environment, and the installed `mailo` is a wrapper that refuses to run anywhere else.
#
# Usage: dev/scenarios/e-intents.sh
set -uo pipefail
# shellcheck source=dev/scenarios/lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

gone() { ! kill -0 "$1" 2>/dev/null; }
provider_pid() { pgrep -u "$(id -u)" -f "^$MAILO_BIN intents$" | head -1; }
busctl_bus() { busctl --address="$BUS" "$@"; }
summon() {
  busctl_bus call org.quire.Mail /org/quire/IntentProvider1 org.quire.IntentProvider1 \
    Summon ts 1 '{"kind":"double_tap"}'
}
refused() { # refused MEMBER SIGNATURE ARGS...
  local out
  out=$(busctl_bus call org.quire.Mail /org/quire/IntentProvider1 org.quire.IntentProvider1 "$@" 2>&1)
  [ $? -ne 0 ] && grep -qi "access denied" <<<"$out"
}

build_all
say "session: scratch tree and a private bus that can start services"
scratch_new
PREFIX_DIR="$SCRATCH/prefix"
UNIT_DIR="$SCRATCH/units"
SECRETS="$SCRATCH/secrets"
mkdir -p "$SECRETS" "$DATA/applications"
WL=
IN_ENV_UNSET=(DISPLAY XAUTHORITY WAYLAND_DISPLAY)
IN_ENV_EXTRA=(
  "XDG_DATA_DIRS=$PREFIX_DIR/share:/usr/share"
  "MAILO_TEST_SECRETS_DIR=$SECRETS"
)
# The real binary behind a wrapper that will not run outside the scratch home: the debug build is
# ~900 MB and is not copied, and a service started with the wrong environment would open the
# person's own store.
cat >"$SCRATCH/mailo-wrapper" <<WRAP
#!/bin/sh
[ "\$HOME" = "$HOME_DIR" ] || { echo "refusing to run outside the scratch home" >&2; exit 97; }
exec "$MAILO_BIN" "\$@"
WRAP
chmod +x "$SCRATCH/mailo-wrapper"
install_into_scratch() {
  env MAILO_PREFIX="$PREFIX_DIR" MAILO_UNIT_DIR="$UNIT_DIR" MAILO_BIN="$SCRATCH/mailo-wrapper" \
    "$MAILO_REPO/dist/install.sh" --no-build >"$OUT_DIR/install.log" 2>&1
}
check "1. dist/install.sh installs the manifest and the service file" install_into_scratch
check "1. the manifest is where the router reads it" \
  cmp -s "$MAILO_REPO/dist/intents/org.quire.Mail.toml" "$PREFIX_DIR/share/quire/intents/org.quire.Mail.toml"
check "1. the service file names the installed binary" \
  grep -qx "Exec=$PREFIX_DIR/bin/mailo intents" "$PREFIX_DIR/share/dbus-1/services/org.quire.Mail.service"

# The stock private bus config of sill's scripts, with the scratch prefix's service directory added.
sed "s|</busconfig>|  <servicedir>$PREFIX_DIR/share/dbus-1/services</servicedir>\n</busconfig>|" \
  "$SILL_CHECKOUT/dev/private-bus.conf" >"$SCRATCH/bus.conf"
out=$(in_env dbus-daemon --config-file="$SCRATCH/bus.conf" --fork --print-address=1 --print-pid=1) \
  || die "no private bus"
BUS=$(sed -n 1p <<<"$out")
BUS_PID=$(sed -n 2p <<<"$out")
case "$BUS" in unix:*) ;; *) die "no private bus address: $out" ;; esac
[ "$BUS" != "${DBUS_SESSION_BUS_ADDRESS:-}" ] || die "not a private bus"

check "2. nothing answers for org.quire.Mail yet" \
  bash -c '! busctl --address="$1" status org.quire.Mail >/dev/null 2>&1' _ "$BUS"
answered=$(summon 2>"$OUT_DIR/summon.err")
check "2. a call starts the provider, and Summon is declined" test "$answered" = 's "\"declined\""'

pid=$(provider_pid)
check "3. the provider is running" test -n "$pid"
check "3. in the scratch home" bash -c 'tr "\0" "\n" </proc/$1/environ | grep -qx "HOME=$2"' _ "${pid:-0}" "$HOME_DIR"

check "4. Perform is refused to a stranger" refused Perform 'sa{sv}' '{}' 0
check "4. Search is refused to a stranger" refused Search st lunch 0
check "4. Context is refused to a stranger" refused Context s '{"kind":"active_window"}'
check "4. Undo is refused to a stranger" refused Undo ss '"stack-1"' '{"kind":"cli"}'

in_env "$SCRATCH/mailo-wrapper" intents >"$OUT_DIR/second.log" 2>&1
check "5. a second provider leaves quietly (exit 0)" test $? = 0
check "5. the first is undisturbed" kill -0 "${pid:-0}"

stop_private_bus
check "6. the provider ends with the bus" wait_for 10 gone "${pid:-0}"
finish "the intent router can start mailo and only the router can use it"
