#!/usr/bin/env bash
# The sans-I/O boundary, mechanically enforced.
#
# `cargo tree -i <dep>` exits 101 when the dependency is absent, which is precisely the state
# we want. Checking the exit status would therefore fail whenever the boundary holds, so we
# check for OUTPUT instead: any line naming the dependency is a leak.
set -uo pipefail

PURE=(mail-domain mail-mime mail-proto)
FORBIDDEN=(tokio rusqlite dioxus reqwest keyring)
fail=0

for crate in "${PURE[@]}"; do
  for dep in "${FORBIDDEN[@]}"; do
    if cargo tree -p "$crate" -i "$dep" 2>/dev/null | grep -q .; then
      echo "LEAK: $crate depends on $dep"
      cargo tree -p "$crate" -i "$dep" 2>/dev/null | head -20
      fail=1
    fi
  done
done

if [ "$fail" -eq 0 ]; then
  echo "sans-I/O boundary holds: ${PURE[*]} reach none of ${FORBIDDEN[*]}"
fi
exit "$fail"
