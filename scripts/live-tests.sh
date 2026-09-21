#!/usr/bin/env bash
# Run the tests that talk to servers this repository did not write.
#
# They are `#[ignore]`d, so `cargo test` skips them; this is how you run them deliberately.
#
# It exists because of FINDINGS F88. Those tests need daemons, the daemons get started by hand
# while their fixtures are being edited, and a result read off a server whose state you have not
# pinned down is not a result — I reported a pass that way once and it did not reproduce. So the
# setup is a script rather than a habit: it stops whatever is on the ports, starts both servers
# from the committed scripts, waits until they answer, and only then runs anything.
#
#   ./scripts/live-tests.sh            # local servers only
#   ./scripts/live-tests.sh --network  # also the probe against NTU's public server
set -uo pipefail

IMAP_PORT=11143
SMTP_PORT=12525
VENV="${MAILO_VENV:-$(dirname "$0")/../.live-venv}"
RECEIVED="${TMPDIR:-/tmp}/mailo-live-received.eml"
WITH_NETWORK=0
[ "${1:-}" = "--network" ] && WITH_NETWORK=1

here() { cd "$(dirname "$0")/.." && pwd; }
ROOT="$(here)"

if [ ! -x "$VENV/bin/python" ]; then
  echo "creating $VENV"
  python3 -m venv "$VENV" || exit 1
  "$VENV/bin/pip" install -q aiosmtpd twisted || exit 1
fi

free_port() {
  # Anything still holding the port would make the new server fail to bind and leave the old
  # one answering — which is exactly how a stale fixture gets reported as a pass.
  for pid in $(pgrep -f "live-....d.py $1" 2>/dev/null); do kill -9 "$pid" 2>/dev/null; done
}

wait_for() {
  for _ in $(seq 1 50); do
    (exec 3<>"/dev/tcp/127.0.0.1/$1") 2>/dev/null && return 0
    sleep 0.2
  done
  echo "nothing came up on 127.0.0.1:$1" >&2
  return 1
}

free_port "$IMAP_PORT"; free_port "$SMTP_PORT"; sleep 0.5
"$VENV/bin/python" "$ROOT/scripts/live-imapd.py" "$IMAP_PORT" >/dev/null 2>&1 &
IMAP_PID=$!
"$VENV/bin/python" "$ROOT/scripts/live-smtpd.py" "$SMTP_PORT" "$RECEIVED" >/dev/null 2>&1 &
SMTP_PID=$!
trap 'kill -9 "$IMAP_PID" "$SMTP_PID" 2>/dev/null' EXIT

wait_for "$IMAP_PORT" || exit 1
wait_for "$SMTP_PORT" || exit 1

fail=0
run() {  # run <crate> <test-target>
  printf '%-14s ' "$2"
  if MAILO_RECEIVED="$RECEIVED" cargo test -q -p "$1" --test "$2" -- --ignored 2>&1 \
      | grep -qE "^test result: ok"; then
    echo "ok"
  else
    echo "FAILED"
    MAILO_RECEIVED="$RECEIVED" cargo test -p "$1" --test "$2" -- --ignored 2>&1 | tail -20
    fail=1
  fi
}

run mail-runtime live_smtp
run mail-runtime live_imap
run mail-app     sync_path
if [ "$WITH_NETWORK" -eq 1 ]; then
  run mail-runtime live_probe
else
  echo "live_probe      skipped (pass --network to include it; it reaches NTU)"
fi

exit "$fail"
