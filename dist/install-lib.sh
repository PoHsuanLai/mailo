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
# The intents manifest the desktop's router reads, and the D-Bus service that starts the provider
# of it on demand: both under $PREFIX/share, which is on XDG_DATA_DIRS for /usr and /usr/local.
INTENTS_DEST="$PREFIX/share/quire/intents/org.quire.Mail.toml"
SERVICE_DEST="$PREFIX/share/dbus-1/services/org.quire.Mail.service"
# The skill that teaches the desktop's companion to use those actions: SKILL.md and skill.toml.
SKILL_DIR="$PREFIX/share/quire/skills/mail"
# The binary to install: a release build unless MAILO_BIN says otherwise (the scenarios install a debug
# build, which is what carries their test seams).
BUILT="${MAILO_BIN:-${CARGO_TARGET_DIR:-$ROOT/target}/release/mailo}"
ICON_SRC="$ROOT/packaging/icons/hicolor/scalable/apps/mailo.svg"
# What the person asked to escalate with, if anything (SUDO=doas ./install.sh), read before SUDO is
# reset below.
ASKED_SUDO="${SUDO:-}"
# Set by need_sudo to the program that gains root when the destination is not ours to write;
# empty for a scratch prefix.
SUDO=

DRY=0

# service_text: the D-Bus service file, naming the binary where it was installed.
service_text() { sed "s|@BIN@|$BIN_DEST|" "$DIST/org.quire.Mail.service.in"; }

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

# asroot CMD...: run CMD as root through the program need_sudo chose, or as ourselves when
# MAILO_PREFIX points somewhere we own (the scenario scripts install into a scratch prefix and never
# need root).
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
    echo "run this as yourself, not root: it builds as you and asks for root only to install" >&2
    exit 1
  }
}

# escalator: print the program that gains root here: the one asked for in $SUDO, else sudo, doas
# or pkexec, whichever is installed first. Prints nothing when there is none.
escalator() {
  local want
  for want in "$ASKED_SUDO" sudo doas pkexec; do
    [ -n "$want" ] && command -v "$want" >/dev/null && { echo "$want"; return 0; }
  done
  return 1
}

# need_sudo: pick the program that gains root, and ask for the password once, unless both
# destinations were redirected (MAILO_PREFIX and MAILO_UNIT_DIR, a scratch directory the caller
# owns) or this is a dry run. With none to be had, say which commands to run as root instead.
need_sudo() {
  if [ -n "${MAILO_PREFIX:-}" ] && [ -n "${MAILO_UNIT_DIR:-}" ]; then SUDO=; return 0; fi
  if ! SUDO="$(escalator)"; then
    SUDO=sudo
    [ "$DRY" = 1 ] && return 0
    {
      echo "this needs root to install, and this computer has no sudo, doas or pkexec."
      echo "Either install one of them and run this again, or list the steps and run each"
      echo "line that starts with \$ yourself, as root:"
      echo "    $0 --dry-run"
    } >&2
    exit 1
  fi
  [ "$DRY" = 1 ] && return 0
  case "$SUDO" in
    sudo) note "sudo will ask for your password once"; sudo -v ;;
    *) note "$SUDO will ask for your password" ;;
  esac
}

# manager_present: whether there is a systemd manager to tell (not for a scratch unit directory).
manager_present() { [ "$UNIT_DIR" = /etc/systemd/user ] && command -v systemctl >/dev/null; }
