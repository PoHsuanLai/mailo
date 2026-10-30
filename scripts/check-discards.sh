#!/usr/bin/env bash
# A fallible result in a library crate is never discarded in silence.
#
# In mail-net, mail-runtime, mail-core, mail-proto, mail-store and mail-client each failure is propagated with `?`,
# recorded where the person sees it (a pass's trouble, a counted field), or logged with
# `Logged::or_log` / `or_log_default` (mail-runtime/src/error.rs). The few that are rightly
# ignored (a channel send at shutdown, best-effort cleanup, a reply to a client that has gone,
# reading an error's body) say why in a comment on the line or the line above.
#
# So this counts, per crate, the lines with `let _ = `, `.ok()` or `unwrap_or_default()` that
# carry no `// ` comment on the same line or the line above. Not every such line discards an
# error (an `Option` has `.ok_or`, not `.ok()`; a missing map entry defaults), so the counts are
# a ratchet rather than a ban: scripts/discards.txt holds "<crate> <pattern> <lines>", and the
# check fails when a count rises, and when one falls until that file says so.
#
# Tests are not counted: files named tests.rs or *_tests.rs, the test-support testing.rs and testing/,
# anything under a tests/ directory, and everything after a `#[cfg(test)]` that opens a `mod`. Plain bash, grep and awk:
# nothing here builds.
set -uo pipefail
cd "$(dirname "$0")/.." || exit 2

CRATES=(mail-net mail-runtime mail-core mail-proto mail-store mail-client)
LIST=scripts/discards.txt

# The extended regex each pattern name stands for.
regex_of() {
  case "$1" in
    let_underscore) printf '%s' 'let _ = ' ;;
    ok) printf '%s' '\.ok\(\)' ;;
    unwrap_or_default) printf '%s' 'unwrap_or_default\(\)' ;;
  esac
}
PATTERNS=(let_underscore ok unwrap_or_default)

# Lines in crate $1 matching pattern $2 with no `// ` on them or on the line above.
count() {
  local crate="$1" pattern
  pattern=$(regex_of "$2")
  find "crates/$crate/src" -name '*.rs' ! -name 'tests.rs' ! -name '*_tests.rs' \
    ! -name 'testing.rs' ! -path '*/testing/*' ! -path '*/tests/*' -print0 \
    | xargs -0 -r env PAT="$pattern" awk '
        FNR == 1 { prev = ""; intest = 0 }
        prev ~ /^[[:space:]]*#\[cfg\(test\)\]/ && $0 ~ /^[[:space:]]*(pub(\([a-z]+\))? )?mod / { intest = 1 }
        !intest && $0 ~ ENVIRON["PAT"] && index($0, "// ") == 0 && index(prev, "// ") == 0 { n++ }
        { prev = $0 }
        END { print n + 0 }' \
    | awk '{ s += $1 } END { print s + 0 }'
}

# `--print` writes the counts as they are now, in the list's format, for a new ratchet.
if [ "${1:-}" = "--print" ]; then
  for crate in "${CRATES[@]}"; do
    for pattern in "${PATTERNS[@]}"; do
      echo "$crate $pattern $(count "$crate" "$pattern")"
    done
  done
  exit 0
fi

fail=0
for crate in "${CRATES[@]}"; do
  for pattern in "${PATTERNS[@]}"; do
    allowed=$(awk -v c="$crate" -v p="$pattern" '$1 == c && $2 == p { print $3 }' "$LIST")
    now=$(count "$crate" "$pattern")
    if [ -z "$allowed" ]; then
      echo "$LIST has no line for \"$crate $pattern\": add \"$crate $pattern $now\""
      fail=1
    elif [ "$now" -gt "$allowed" ]; then
      echo "$crate has $now uncommented $pattern lines, over the $allowed in $LIST: propagate the error with ?, record it where the person sees it, log it with Logged::or_log, or say in a comment why ignoring it is right"
      fail=1
    elif [ "$now" -lt "$allowed" ]; then
      echo "$crate has only $now uncommented $pattern lines: lower the $allowed in $LIST to $now"
      fail=1
    fi
  done
done
if [ "$fail" -eq 0 ]; then
  echo "no new silent discards in ${CRATES[*]}"
fi
exit "$fail"
