#!/usr/bin/env bash
# Install mailo into the desktop from this checkout: build it in release, then put the binary, the
# desktop entry, the icon, the intents manifest with the D-Bus service that starts its provider,
# the companion's mail skill, and the user unit that starts the sync daemon at login where the
# session reads them. Run as yourself; sudo asks once. Safe to run again.
#
#   dist/install.sh --dry-run    print every step, change nothing (not even a build)
#   dist/install.sh              do it
#
# Installs to /usr/local (binary, desktop entry, icon, intents manifest and its D-Bus service) and
# /etc/systemd/user (the sync daemon's unit, enabled for graphical-session.target, so it starts in
# any session; the intents provider's, which only the bus starts). Not touched: your mail,
# ~/.config, the keyring, any other session's files. dist/uninstall.sh undoes it. Next: log in (or
# `systemctl --user start mailo-watch`), then add an account with `mailo account add you@example.org`.
#
# MAILO_BIN names the binary to install instead of the release build. MAILO_PREFIX and
# MAILO_UNIT_DIR redirect the destinations (the scenario scripts install into a scratch directory
# this way, and never need sudo or a systemd manager).
set -euo pipefail
# shellcheck source=install-lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/install-lib.sh"

usage() { sed -n '2,18p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; }
EXTRA_FLAGS="--no-build"
parse_flags "$@"
[ "$DRY" = 0 ] || echo "dry run: nothing below is executed"

step "1/5 checks"
refuse_root
command -v cargo >/dev/null || [ "${FLAG_no_build:-0}" = 1 ] || { echo "cargo not found" >&2; exit 1; }
for size in $ICON_SIZES; do
  [ -f "$(icon_src "$size")" ] || { echo "no icon at $(icon_src "$size")" >&2; exit 1; }
done
if [ -e "$HOME/.cargo/bin/mailo" ]; then
  note "WARNING: $HOME/.cargo/bin/mailo would come first on a login shell's PATH and shadow $BIN_DEST;"
  note "         remove it (rm $HOME/.cargo/bin/mailo) or a shell runs the old build"
fi
need_sudo

step "2/5 build mailo (release, from $ROOT)"
if [ "${FLAG_no_build:-0}" = 1 ]; then
  note "--no-build: using $BUILT"
else
  run cargo build --release --locked --manifest-path "$ROOT/Cargo.toml" -p mail-app --bin mailo
fi
[ "$DRY" = 1 ] || [ -x "$BUILT" ] || { echo "no binary at $BUILT" >&2; exit 1; }

step "3/5 the binary, the desktop entry, the icon and the intents"
put 755 "$BUILT" "$BIN_DEST"
put 644 "$DIST/org.quire.Mail.desktop" "$DESKTOP_DEST"
gone "$OLD_DESKTOP_DEST"
for size in $ICON_SIZES; do
  put 644 "$(icon_src "$size")" "$(icon_dest "$size")"
done
gone "$OLD_ICON_DEST"
# What the desktop's intent router reads, and the activation that starts `mailo intents` for it.
put 644 "$DIST/intents/org.quire.Mail.toml" "$INTENTS_DEST"
put 644 "$DIST/skills/mail/SKILL.md" "$SKILL_DIR/SKILL.md"
put 644 "$DIST/skills/mail/skill.toml" "$SKILL_DIR/skill.toml"
SERVICE_TMP="$(mktemp)"
service_text >"$SERVICE_TMP"
put 644 "$SERVICE_TMP" "$SERVICE_DEST"
rm -f "$SERVICE_TMP"

step "4/5 the user units: the sync daemon's, and the one the bus starts the intents provider in"
put 644 "$DIST/$UNIT_NAME" "$UNIT_DEST"
PROVIDER_TMP="$(mktemp)"
provider_unit_text >"$PROVIDER_TMP"
put 644 "$PROVIDER_TMP" "$PROVIDER_UNIT_DEST"
rm -f "$PROVIDER_TMP"

step "5/5 caches and the user manager"
if command -v update-desktop-database >/dev/null; then
  run ${SUDO:+"$SUDO"} update-desktop-database "$(dirname "$DESKTOP_DEST")"
fi
if command -v gtk-update-icon-cache >/dev/null && [ -f "$PREFIX/share/icons/hicolor/index.theme" ]; then
  run ${SUDO:+"$SUDO"} gtk-update-icon-cache -q -t "$PREFIX/share/icons/hicolor"
else
  note "no icon cache to refresh under $PREFIX/share/icons/hicolor"
fi
if command -v restorecon >/dev/null && [ -n "$SUDO" ]; then
  # shellcheck disable=SC2046 # one path per size
  asroot restorecon -F "$BIN_DEST" "$DESKTOP_DEST" $(for size in $ICON_SIZES; do icon_dest "$size"; done) "$UNIT_DEST" "$PROVIDER_UNIT_DEST" "$INTENTS_DEST" "$SERVICE_DEST" "$SKILL_DIR/SKILL.md" "$SKILL_DIR/skill.toml"
fi
if manager_present; then
  # --global: every user's graphical-session.target wants the unit, whichever session starts it.
  asroot systemctl --global enable "$UNIT_NAME"
  run systemctl --user daemon-reload
else
  note "no systemd manager for $UNIT_DIR: the unit is in place but not enabled"
fi

step "done"
note "The sync daemon starts with your next graphical session. Now: systemctl --user start $UNIT_NAME"
note "Accounts: mailo account add you@example.org   (then open Mailo from the launcher)"
