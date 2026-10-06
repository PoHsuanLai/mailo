#!/usr/bin/env bash
# Remove what dist/install.sh put in place: the unit (disabled first), the desktop entry, the icon,
# the intents manifest and its D-Bus service, the mail skill, and the binary. Your mail, settings
# and keyring are not touched. Run as yourself; sudo asks once. Safe to run again.
#
#   dist/uninstall.sh --dry-run    print every step, change nothing
#   dist/uninstall.sh              remove the system files
set -euo pipefail
# shellcheck source=install-lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/install-lib.sh"

usage() { sed -n '2,9p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; }
parse_flags "$@"
[ "$DRY" = 0 ] || echo "dry run: nothing below is executed"

step "1/3 checks"
refuse_root
need_sudo

step "2/3 the sync daemon"
if manager_present; then
  # Stopped for this session, disabled for every session; both are fine when it is not there.
  run systemctl --user stop "$UNIT_NAME" || true
  if [ -e "$UNIT_DEST" ]; then asroot systemctl --global disable "$UNIT_NAME" || true; fi
fi
gone "$UNIT_DEST"

step "3/3 the entry, the icon, the intents, the skill and the binary"
gone "$DESKTOP_DEST"
gone "$ICON_DEST"
gone "$INTENTS_DEST"
gone "$SKILL_DIR/SKILL.md"
gone "$SKILL_DIR/skill.toml"
[ ! -d "$SKILL_DIR" ] || asroot rmdir "$SKILL_DIR" || true
gone "$SERVICE_DEST"
gone "$BIN_DEST"
if command -v update-desktop-database >/dev/null && [ -d "$(dirname "$DESKTOP_DEST")" ]; then
  run ${SUDO:+"$SUDO"} update-desktop-database "$(dirname "$DESKTOP_DEST")"
fi
manager_present && run systemctl --user daemon-reload

step "done"
note "Kept: ~/.local/share/mailo (your mail), ~/.config/mailo, and the keyring entries."
