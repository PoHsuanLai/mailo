# shellcheck shell=bash disable=SC2034
# Shared by dist/install.sh and dist/uninstall.sh (sourced, never run). Where each piece of mailo
# goes, and the helpers that print every step and honour --dry-run. The same shape as sill's
# dist/install-lib.sh, so one habit covers both.
#
# Nothing here touches the person's own files: no ~/.config, no mail, no keyring. The user's
# mail (~/.local/share/mailo) and settings stay where they are through an uninstall.

DIST="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$DIST/.." && pwd)"

PREFIX="${MAILO_PREFIX:-/usr/local}"
UNIT_DIR="${MAILO_UNIT_DIR:-/etc/systemd/user}"
BIN_DEST="$PREFIX/bin/mailo"
DESKTOP_DEST="$PREFIX/share/applications/mailo.desktop"
ICON_DEST="$PREFIX/share/icons/hicolor/scalable/apps/mailo.svg"
UNIT_NAME=mailo-watch.service
UNIT_DEST="$UNIT_DIR/$UNIT_NAME"
# The binary to install: a release build unless MAILO_BIN says otherwise (the scenarios install a debug
# build, which is what carries their test seams).
BUILT="${MAILO_BIN:-${CARGO_TARGET_DIR:-$ROOT/target}/release/mailo}"
ICON_SRC="$ROOT/packaging/icons/hicolor/scalable/apps/mailo.svg"
# Set to "sudo" by need_sudo when the destination is not ours to write; empty for a scratch prefix.
SUDO=

DRY=0

# parse_flags "$@": --dry-run, and the extra flags a script allows ($EXTRA_FLAGS, space-separated).
parse_flags() {
  local arg name
  for arg in "$@"; do
    case "$arg" in
      --dry-run) DRY=1 ;;
      -h|--help) usage; exit 0 ;;
      *) if [[ " ${EXTRA_FLAGS:-} " == *" $arg "* ]]; then name="${arg#--}"; declare -g "FLAG_${name//-/_}=1"; else
           echo "unknown argument: $arg" >&2; usage >&2; exit 2; fi ;;
    esac
  done
}

step() { printf '\n==> %s\n' "$*"; }
note() { printf '    %s\n' "$*"; }

# run CMD...: print the command; run it unless --dry-run.
run() {
  printf '    $ %s\n' "$*"
  [ "$DRY" = 1 ] || "$@"
}

# asroot CMD...: run CMD as root through sudo, or as ourselves when MAILO_PREFIX points somewhere we
# own (the scenario scripts install into a scratch prefix and never need sudo).
asroot() { run ${SUDO:+"$SUDO"} "$@"; }

# put MODE SRC DEST: install SRC at DEST, unless DEST already is a copy of it.
put() {
  local mode="$1" src="$2" dest="$3"
  if [ -f "$dest" ] && cmp -s "$src" "$dest"; then
    note "unchanged: $dest"
    return 0
  fi
  asroot install -D -m "$mode" "$src" "$dest"
}

# gone PATH: remove PATH when it exists.
gone() {
  if [ -e "$1" ]; then asroot rm -f "$1"; else note "already gone: $1"; fi
}

refuse_root() {
  [ "$(id -u)" -ne 0 ] || {
    echo "run this as yourself, not root: it builds as you and uses sudo only to install" >&2
    exit 1
  }
}

# need_sudo: ask for the password once, unless both destinations were redirected (MAILO_PREFIX and
# MAILO_UNIT_DIR, a scratch directory the caller owns) or this is a dry run.
need_sudo() {
  if [ -n "${MAILO_PREFIX:-}" ] && [ -n "${MAILO_UNIT_DIR:-}" ]; then SUDO=; return 0; fi
  SUDO=sudo
  [ "$DRY" = 1 ] && return 0
  command -v sudo >/dev/null || { echo "sudo is needed" >&2; exit 1; }
  note "sudo will ask for your password once"
  sudo -v
}

# manager_present: whether there is a systemd manager to tell (not for a scratch unit directory).
manager_present() { [ "$UNIT_DIR" = /etc/systemd/user ] && command -v systemctl >/dev/null; }
