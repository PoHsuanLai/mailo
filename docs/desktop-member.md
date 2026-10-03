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
`Open(["mailo:thread/<id>"], {"activation-token": token})` opens the conversation in that window, where
the person is already reading, and raises the window: the request keeps the token
(`handoff::ActivationToken`) and the window hands it to quire's
`use_window_handle().focus_with_token(token)`, which on Wayland is xdg-activation's `activate` for the
window's surface (the only way a compositor lets a window that exists take the keyboard; on X11, with
no token, or without xdg-activation it is a plain focus). A token is good once, so of several URIs in
one `Open` only the first request carries it. Only when no window answers does `mailo open` open one.
The name has a dot because a D-Bus name needs one and the desktop entry's id (`mailo`, the
window's `app_id`) has none; it is the Flatpak's app id too. There is no D-Bus activation file:
nothing yet needs a closed mailo to be started by a method call, because the watch starts it.

## What it answers

`mailo intents` is mailo as a provider for the desktop's intent router (docket's `intentd`), the
way a companion, the launcher or `quire-do mail` ask it to do something without its window. It owns
`org.quire.Mail` on the session bus and serves `org.quire.IntentProvider1` at
`/org/quire/IntentProvider1` (`crates/mail-app/src/intents`). The router starts it by D-Bus
activation (`dbus-1/services/org.quire.Mail.service`, installed by `dist/install.sh` and by the
packages) and it stays until the bus goes away; a second one finds the name taken and exits 0.

The manifest, `dist/intents/org.quire.Mail.toml`, installed under `$XDG_DATA_DIRS/quire/intents/`,
declares two kinds, `mail.thread` and `mail.draft`, and these actions:

| Action | Effect | Takes | Undo |
| --- | --- | --- | --- |
| `mail.thread.search` | read | `query`, in mailo's own search language | |
| `mail.thread.read` | read | one conversation; the answer is its text, labelled untrusted mail | |
| `mail.thread.archive`, `star`, `unstar` | undoable write | conversations | token |
| `mail.thread.label`, `unlabel` | undoable write | conversations, `label` (must exist) | token |
| `mail.thread.snooze` | undoable write | conversations, `until` | token |
| `mail.draft.create` | undoable write | `to`, `subject`, `body`, `from`, any of them | token discards it |
| `mail.message.send` | outbound | `to`, `body`, `subject`, `from`; `DryRun` shows the message | token takes it back to a draft |

The conversation actions are `mail_core::act`, which the window acts with too: the same patch, the
same work queued for the server, the same undo. An undo token names an entry of the provider's own
`UndoStack`, or a draft to discard or unsend (`stack-7`, `discard-<id>`, `unsend-<id>`); the stack is
in memory, so a token does not outlive the provider, and it is not the window's Cmd+Z stack, which is
the window's process. A send is queued as the window queues one, so it can be taken back until the
outbox has delivered it, which with a watch running is soon.

Decisions:

- **No dependency on docket.** The router's crates are in a repository that is not public and mailo
  is. The wire is small, so `intents/wire` writes out the JSON forms the router uses (adjacently
  tagged enums, ids and units as bare strings and numbers), `intents/serve.rs` serves the interface
  with zbus, and tests hold both to the router: the served introspection equals
  `IntentProvider1.xml`, which is the router's file; each form is pinned to the router's text; and the
  manifest is checked by `docket-eval --check-app` where a docket checkout is at hand.
- **A process of its own, not the window.** Activation has to work with no window, and a closed mailo
  has to be able to archive mail. So `Context` answers a private window looking at nothing, and
  `Summon` declines; a window that answers its own `Context` is a later step (the name is owned by
  one process).
- **Only the router calls.** Every member but `Summon` is refused for a caller that does not own
  `org.quire.Intents1`, so a process that knows mailo's bus name gets no way round the gate.
- **The window hears what the provider writes** through its look at the store's data version (above).

## What it reads

The look is the desktop's: `$XDG_CONFIG_HOME/quire/appearance.toml` and `style.css`, read and
watched through `ds_settings::use_environment` with `AppName::QUIRE`, the store sill and detent
use. A theme, accent or motion change restyles a running window. mailo writes none of it. Files
mailo kept for itself are brought over once, when the desktop has none (`ui::appearance::adopt`):
`mailo/appearance.toml` and `mailo/style.css` are copied, else the theme in `appearance.json`
seeds the desktop's file; nothing in mailo's directory is written or deleted.

## Building against quire master

`crates/mail-app/Cargo.toml` names quire by commit (a tag again once quire's next release is cut). To build against a checkout, put this in an
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
- `e-intents.sh`: the installed D-Bus service starts `mailo intents` on a bus of its own, and only the
  router's name may call it. No window or mail server.
- `all.sh` runs them in turn.

## Who fetches when a watch runs

The window and the watch would be two writers on one SQLite file, and two writers sometimes end a
pass with `database is locked` (`busy_timeout` is five seconds). So where a watch runs
(`mail_core::ipc::watching::running`, asked each time, since a watch may start or stop while a window
is open) the window leaves scheduled fetching to it and keeps only what a person asks for
(`ui/fetching/delegate.rs`):

| Event | With a watch | Without |
| --- | --- | --- |
| a poll or a timer's wake | not run; asked again one interval later | run |
| a push from the server | dropped; the window holds no IDLE connection of its own | run |
| Sync, a folder opened, a send that came due, signing in again | run | run |
| an account that has never fetched anything | run | run |

The last row is the watch's blind spot: it reads its accounts when it starts, so an account made
since is the window's to fetch the first time. A watch that stops hands fetching back within one
interval, because the deferred poll is asked again and then runs.

The mail the watch stores is not the window's doing. The window looks every two seconds at whether
another connection has committed to the store (`SqliteStore::data_version`, SQLite's
`PRAGMA data_version`, which its own writes do not move) and moves its revision when one has, so a
list redraws when the watchwrote.

