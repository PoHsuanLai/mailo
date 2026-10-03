# mailo as a member of the desktop

mailo is not a standalone app on a desktop that runs the session (sill's, or any other that
reaches `graphical-session.target`). This page says what it does there, what it reads, what it
emits, and where each part is tested. Nothing here needs the shell to know anything about mailo.

## What starts at login

`dist/mailo-watch.service`, a systemd user unit `WantedBy=graphical-session.target`, runs
`mailo watch`: one sync engine for every account, IDLE where the server offers it, with or without
a window. `dist/install.sh` installs it into `/etc/systemd/user` and enables it for every user's
graphical session (`systemctl --global enable`); `dist/uninstall.sh` reverses it.

`mailo watch` takes a per-user lock (`mail_core::ipc::watching`, a latchkey file lock under the
name `mailo-watch`), and a second one exits 0 saying so: a person typing it in a terminal while the
unit runs must not make every message announce twice, and a unit that starts while one runs by
hand must not restart in a loop.

## What it emits

| To | How | Where |
| --- | --- | --- |
| The notification server | `org.freedesktop.Notifications.Notify`: app name `mailo`, icon `mailo`, summary the sender, body the subject, action `default`, hints `category=email.arrived`, `desktop-entry=mailo`, `x-mailo-thread`, `x-mailo-account` | `mail-core/src/notify/desktop.rs` |
| The dock | `com.canonical.Unity.LauncherEntry.Update("application://mailo.desktop", {count, count-visible})`, every account's unread inbox conversations, polled every 2 s and said again every minute | `mail-app/src/session.rs`, `ui/launcher/` |

With a watch running, the window leaves the launcher count to it (`mail_core::ipc::watching::running`):
a dock forgets a count when its sender leaves the bus, so a count sent by a window would vanish when
it closed. With no watch, the window shows the count of its own Space, as before.

Do Not Disturb is sill's (`notifications.dnd`): the server withholds the banner and keeps the
notification in its centre, and exposes no property for a client to read, so mailo posts ordinary
notifications and has nothing to check.

## What a click does

The server emits `ActivationToken(id, token)`, then `ActionInvoked(id, "default")`. One listener
in the watch (`notify/click.rs`) hears both for the notifications it showed and starts
`mailo open <thread>` with the token in `XDG_ACTIVATION_TOKEN` and `DESKTOP_STARTUP_ID`.

`mailo open <thread>` first asks a running window (`ui/handoff`): the window owns
`io.github.PoHsuanLai.mailo` on the session bus and answers `org.freedesktop.Application`, and
`Open(["mailo:thread/<id>"], {"activation-token": token})` opens the conversation in a window of its
own (a conversation already open is raised). Only when no window answers does `mailo open` open
one. The name has a dot because a D-Bus name needs one and the desktop entry's id (`mailo`, the
window's `app_id`) has none; it is the Flatpak's app id too. There is no D-Bus activation file:
nothing yet needs a closed mailo to be started by a method call, because the watch starts it.

## What it reads

The look is the desktop's: `$XDG_CONFIG_HOME/quire/appearance.toml` and `style.css`, read and
watched through `ds_settings::use_environment` with `AppName::QUIRE`, the store sill and detent
use. A theme, accent or motion change restyles a running window. mailo writes none of it. Files
mailo kept for itself are brought over once, when the desktop has none (`ui::appearance::adopt`):
`mailo/appearance.toml` and `mailo/style.css` are copied, else the theme in `appearance.json`
seeds the desktop's file; nothing in mailo's directory is written or deleted.

## Building against quire master

`crates/mail-app/Cargo.toml` still names quire by tag. To build against a checkout, put this in an
uncommitted `.cargo/config.toml` (the file is in `.git/info/exclude` in the integration worktree):

```toml
[patch."https://github.com/PoHsuanLai/quire"]
ds = { path = "../quire/crates/ds" }
ds-settings = { path = "../quire/crates/ds-settings" }
ds-blitz = { path = "../quire/crates/ds-blitz" }
ds-lint = { path = "../quire/crates/ds-lint" }
ds-harness = { path = "../quire/crates/ds-harness" }
```

Against quire master the one source change was `Toolbar::onpick`, which now hands the pick and
its anchor over (`Picked<T>`), and pdfrum moved to the rev quire's pinned block names.

## The scenarios

`dev/scenarios/` runs the real binaries against the real sill on a nested compositor, with a
private bus, a scratch HOME and a fake IMAP server (`scripts/live-imapd.py`, which pushes a
message dropped into `$MAILO_EXTRA_MAIL` to whoever is idling). Credentials in a scenario are
plain files in the scratch tree: a debug build reads `MAILO_TEST_SECRETS_DIR` instead of the
keyring (`mail-runtime/src/secrets.rs`, compiled out of release builds).

- `a-dark-mode.sh`: the desktop's appearance file changes, a running window turns dark.
- `b-new-mail.sh`: a message arrives; a banner and a badge.
- `c-click-opens.sh`: the banner's click opens the message, with and without a window running.
- `d-daemon-survives.sh`: the window closes; the daemon and the badge carry on.
- `all.sh` runs them in turn.
