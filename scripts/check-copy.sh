#!/usr/bin/env bash
# Every word the window shows lives in crates/mail-ui-model/src/copy/, one file per kind (buttons,
# menus, headers, titles, tooltips, accessible names, fields, toasts, status lines, empty states,
# confirmations, errors). This holds the literals still written elsewhere under
# crates/mail-ui/src/ui, crates/mail-ui-kit/src and in the rest of crates/mail-ui-model/src to a ratchet: scripts/copy-literals.txt holds "<file> <count>", and the
# check fails when a file's count rises (or a file not listed has one), and when a count falls
# until that file says so. A new word goes in copy/ and is named from there.
#
# What counts as a visible literal is scripts/copy-literals.py's heuristic (its docstring says
# what is skipped: comments, tests, classes, keys, ids, CSS, logs, panics, key names, fonts...).
#
#   ./scripts/check-copy.sh                 check against the list
#   ./scripts/check-copy.sh --print         the counts as they are now, in the list's format
#   ./scripts/check-copy.sh --show <file>   the literals one file still holds, with their lines
#
# Plain bash and python3: nothing here builds.
set -uo pipefail
cd "$(dirname "$0")/.." || exit 2

LIST=scripts/copy-literals.txt
COUNT=(python3 -I scripts/copy-literals.py)

case "${1:-}" in
  --print) exec "${COUNT[@]}" ;;
  --show) exec "${COUNT[@]}" --show "${2:?a file under crates/mail-ui/src/ui, crates/mail-ui-kit/src or crates/mail-ui-model/src}" ;;
  "") ;;
  *) echo "usage: $0 [--print | --show <file>]" >&2; exit 2 ;;
esac

now=$("${COUNT[@]}") || exit 2
listed=$(grep -vE '^[[:space:]]*(#|$)' "$LIST")
fail=0
while read -r file count; do
  [ -z "$file" ] && continue
  allowed=$(awk -v f="$file" '$1 == f { print $2 }' <<<"$listed")
  if [ -z "$allowed" ]; then
    echo "$file shows $count literal words and is not in $LIST: put them in crates/mail-ui-model/src/copy/ (see ./scripts/check-copy.sh --show $file)"
    fail=1
  elif [ "$count" -gt "$allowed" ]; then
    echo "$file shows $count literal words, over the $allowed in $LIST: put the new ones in crates/mail-ui-model/src/copy/ (see ./scripts/check-copy.sh --show $file)"
    fail=1
  elif [ "$count" -lt "$allowed" ]; then
    echo "$file shows only $count literal words: lower the $allowed in $LIST to $count"
    fail=1
  fi
done <<<"$now"
while read -r file allowed; do
  [ -z "$file" ] && continue
  if ! grep -q "^$file " <<<"$now"; then
    echo "$file shows no literal words now: remove its line from $LIST"
    fail=1
  fi
done <<<"$listed"
total=$(awk '{ s += $2 } END { print s + 0 }' <<<"$now")
if [ "$fail" -eq 0 ]; then
  echo "no new literal words outside crates/mail-ui-model/src/copy ($total left to move)"
fi
exit "$fail"
