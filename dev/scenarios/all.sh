#!/usr/bin/env bash
# Every cross-component scenario in turn: a (dark mode), b (new mail, banner and badge), c (click
# opens the message), d (the daemon survives the window), e (the intent router starts mailo).
# Exit 0 only when all are green.
#
# Usage: dev/scenarios/all.sh
set -uo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
status=0
for scenario in a-dark-mode b-new-mail c-click-opens d-daemon-survives e-intents; do
  echo "######## $scenario"
  "$here/$scenario.sh" || status=1
done
[ "$status" = 0 ] && echo "ALL SCENARIOS GREEN" || echo "A SCENARIO FAILED"
exit "$status"
