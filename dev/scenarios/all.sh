#!/usr/bin/env bash
# Every cross-component scenario in turn: a (dark mode), b (new mail, banner and badge), c (click
# opens the message), d (the daemon survives the window), e (the intent router starts mailo).
# Exit 0 only when all are green.
#
# Usage: dev/scenarios/all.sh [--build] [--compositor casement|kwin]
#   --build                 build everything first, outside the jail (lib.sh's prebuild, and
#                           casement's tools through its dev/bootstrap.sh)
#   --compositor casement   (the default) each scenario in its own desktop jail
#                           (../desktop/harness/jail.sh) on a headless casement it starts itself;
#                           nothing in the jail builds, so run --build first
#   --compositor kwin       each scenario directly, on shell-host's headless KWin, building first
set -uo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
build=no compositor=casement
while [ $# -gt 0 ]; do
  case "$1" in
    --build) build=yes; shift ;;
    --compositor) compositor=${2:?--compositor needs a value}; shift 2 ;;
    *) echo "all.sh: unknown argument $1" >&2; exit 64 ;;
  esac
done
case "$compositor" in casement|kwin) ;; *) echo "all.sh: --compositor is casement or kwin" >&2; exit 64 ;; esac

# lib.sh's prebuild and paths, in a subshell so its EXIT trap stays out of this shell; prints the
# paths the jail needs, one per line.
paths() {
  (
    # shellcheck source=dev/scenarios/lib.sh
    source "$here/lib.sh"
    trap - EXIT
    if [ "$build" = yes ]; then
      prebuild || { echo "all.sh: the build failed (see $OUT_DIR/build.log)" >&2; exit 3; }
      if [ "$compositor" = casement ]; then
        (cd "$CASEMENT_REPO" && dev/bootstrap.sh --build-only) >>"$OUT_DIR/build.log" 2>&1 \
          || { echo "all.sh: casement's tools did not build (see $OUT_DIR/build.log)" >&2; exit 3; }
      fi
    fi
    printf '%s\n' "$MAILO_TARGET" "$SILL_TARGET" "$SILL_REPO" "$CASEMENT_REPO" "$CASEMENT_TOOLS" \
      "$SHELL_HOST_DEV"
  )
}
listed=$(paths) || exit 3
{ read -r mailo_target; read -r sill_target; read -r sill_repo; read -r casement_repo
  read -r casement_tools; read -r shell_host_dev; } <<<"$listed"

if [ "$compositor" = casement ]; then
  desktop="${MAILO_DESKTOP:-$(cd "$sill_repo/../desktop" 2>/dev/null && pwd || echo "$HOME/desktop")}"
  jail="${MAILO_JAIL:-$desktop/harness/jail.sh}"
  [ -x "$jail" ] || { echo "all.sh: the jail script $jail is missing" >&2; exit 3; }
  out="$mailo_target/scenarios/jailed"
  mkdir -p "$out"
  jail_env=(
    --env SILL_COMPOSITOR=casement --env SILL_PREBUILT=1 --env SCN_PREBUILT=1
    --env "CARGO_TARGET_DIR=$sill_target" --env "MAILO_TARGET=$mailo_target"
    --env "MAILO_SILL=$sill_repo" --env "SHELL_HOST_DEV=$shell_host_dev"
    --env "CASEMENT_REPO=$casement_repo" --env "CASEMENT_TOOLS=$casement_tools"
  )
fi

status=0
for scenario in a-dark-mode b-new-mail c-click-opens d-daemon-survives e-intents; do
  echo "######## $scenario"
  if [ "$compositor" = casement ]; then
    "$jail" --scratch "$out/$scenario" "${jail_env[@]}" -- "$here/$scenario.sh" </dev/null || status=1
  else
    SILL_COMPOSITOR=kwin "$here/$scenario.sh" || status=1
  fi
done
[ "$status" = 0 ] && echo "ALL SCENARIOS GREEN" || echo "A SCENARIO FAILED"
exit "$status"
