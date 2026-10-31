# mailo

mailo is a desktop mail client written in Rust. It keeps your mail in a local SQLite database,
talks to servers over IMAP, POP3, JMAP and Microsoft Graph, sends over SMTP, JMAP or Graph, and
draws its window with [quire](https://github.com/PoHsuanLai/quire) (Blitz and wgpu; there is no
webview). One binary, `mailo`, is both the window (no arguments) and a command line (anything
else), over the same store.

This is the 0.1.0 line. It is developed and tested mostly on macOS and Linux; the Windows build is
the least exercised.

## Features

**Accounts and protocols**

- IMAP, POP3 (with a leave-on-server choice), JMAP, Microsoft Graph, and local-only accounts that
  have no server.
- Sending over SMTP, JMAP `EmailSubmission` or Graph `sendMail`.
- OAuth sign-in for Google and Microsoft accounts, with an OAuth client id of your own: no
  client id ships with mailo. Give one in `MAILO_OAUTH_CLIENT_ID` (and `MAILO_OAUTH_CLIENT_SECRET`
  where the client has a secret), in porter's `clients.toml`
  (`$XDG_CONFIG_HOME/porter/clients.toml`), or in mailo's `oauth.json` in its config folder.
  Passwords and PLAIN/LOGIN-style SASL for the rest. Server settings are discovered from the
  address where possible.
- Offline use: actions are queued in an outbox and replayed when the server is reachable.
- `mailo watch` fetches continuously without a window, waiting on IMAP IDLE or JMAP's event stream where the server offers one and polling otherwise.

**Reading and writing**

- HTML mail is sanitised; remote images are blocked until you allow them (per message or per
  trusted sender). Read receipts are never sent unless you answer one or set Settings › Reading
  to ask or always.
- Threaded conversations, labels and folders, stars, archive, undo for most actions.
- Compose with signatures, templates, reply and forward, attachments, spell check, and
  `mailto:` links (RFC 6068).
- Send later (`mailo send --at`, taken back with `mailo unsend`), snooze, and follow-up reminders.
- One-click and mailto `List-Unsubscribe`.
- Import from mbox, Maildir, `.eml`, Apple Mail (an export or its `.emlx` files) and a
  Thunderbird profile; export; printing (PDF through the system print dialog); BIMI sender logos.

**Search and rules**

- Full-text search over the local store (SQLite FTS5), plus server-side search where the protocol
  has it.
- Rules that run in mailo. Where the server offers ManageSieve, rules can be pushed to it, and an
  away reply (`mailo vacation`) is set there. A JMAP account's away reply is its server's own
  vacation response, and a Microsoft account that reads through Graph gets Exchange's automatic
  replies. Gmail and Microsoft offer no ManageSieve, so those accounts have local rules only, and
  Gmail and Microsoft over IMAP have no vacation reply.

**Encryption**

- OpenPGP (RFC 4880/9580 via rPGP): generate and import keys, sign, encrypt, verify, Autocrypt,
  and key lookup through WKD.
- S/MIME: import PKCS#12 certificates, sign, encrypt, decrypt, verify, and a trust list.

**Contacts and calendar**

- Contacts: a local address book, vCard import and export, and CardDAV sync.
- Calendar invitations in mail: shown in the reader, answered on request with an iTIP `REPLY`,
  and saved as `.ics`. There is no CalDAV sync and no calendar view; mailo is not a calendar.

## Install

CI builds the packages (the `package` job of `.github/workflows/ci.yml`) on every push to master.
There is no tagged release yet, so take them from a run's artifacts.

| OS | Package |
| --- | --- |
| macOS | `mailo-<version>.dmg` holding `mailo.app` |
| Debian, Ubuntu | `.deb` |
| Fedora | `.rpm` |
| Windows | `mailo-x86_64.msi` |

```sh
sudo apt install ./mailo_*.deb          # Debian, Ubuntu
sudo dnf install ./mailo-*.rpm          # Fedora
```

The macOS and Windows packages are unsigned. macOS says the developer cannot be verified: open
mailo once from Finder with Control-click, Open. Windows SmartScreen warns about an unrecognised
app: More info, Run anyway.

`./scripts/package.sh` also builds a Flatpak (`packaging/flatpak/`) if `flatpak-builder` is
installed; CI does not publish one. `dist/install.sh` installs from a checkout into `/usr/local`
along with a systemd user unit, `mailo-watch.service`, that runs `mailo watch` from login
(`dist/install.sh --dry-run` prints every step; `dist/uninstall.sh` removes it again, and leaves
your mail and settings).

Passwords and tokens are kept in the OS keychain: the login keychain on macOS, the Credential
Manager on Windows, and on Linux the Secret Service (GNOME Keyring, KWallet, KeePassXC), which
must be running.

## Build from source

The minimum supported Rust is 1.92 (`rust-version` in `Cargo.toml`). The repository has no
`rust-toolchain.toml`; CI uses current stable. There is no `.cargo/config.toml`: a plain clone
builds, with quire, porter, anyview and latchkey fetched from GitHub at the revisions `Cargo.toml`
pins.

```sh
cargo build --release -p mail-app      # produces target/release/mailo
```

- **Linux:** a C compiler and fontconfig headers (`libfontconfig1-dev` on Debian and Ubuntu,
  `fontconfig-devel` on Fedora). Wayland, X11, xkbcommon and the GPU drivers are opened at run
  time. `cargo test` also wants Noto CJK fonts (`fonts-noto-cjk`), as print tests check which
  regional face CJK mail is set in.
- **macOS:** Xcode's command line tools. `./packaging/macos/bundle.sh` makes `mailo.app` and the
  `.dmg` in `target/macos/`.
- **Windows:** Visual Studio's build tools. CI builds the release with
  `--no-default-features`, then `cargo wix --package mail-app` with `packaging/windows/main.wxs`
  makes the MSI (WiX Toolset v3).

The default features (`quire-desktop`, `anyview`) add links to the quire desktop's own services;
the macOS and Windows packages are built without them.

The checks CI runs:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo nextest run --workspace          # or cargo test --workspace; no network needed
./scripts/check-boundary.sh            # the sans-I/O boundary
./scripts/check-copy.sh                # the window's words live in crates/mail-ui-model/src/copy
cargo deny check licenses
```

Tests that need sockets are `#[ignore]`d and run by `./scripts/live-tests.sh` against local
servers.

## Using it

```sh
mailo account add you@example.org     # `mailo account` and the help list OAuth and manual hosts
mailo watch                           # fetch continuously, with notifications
mailo                                 # open the window
```

Run `mailo` with an unknown command to print the full command list.

### Default mail app (`mailto:`)

`mailo mailto:someone@example.org?subject=Hello` opens a composer holding what the link asks for
(`to`, `cc`, `bcc`, `subject`, `body`; other fields are ignored). Nothing is sent until you send it.

- **macOS:** the installed `mailo.app` is registered for `mailto:`. At a launch with an account,
  mailo asks once whether to become the default mail app, and Settings > General shows the current
  handler with a Make Default button; the system confirms the change. A link opened while mailo is
  running is handed to that window. A copy run from a terminal is not a bundle and cannot be chosen.
- **Linux:** the desktop entry (`packaging/org.quire.Mail.desktop`, installed by the packages) registers the
  handler. Choose it in your desktop's settings, or with
  `xdg-mime default org.quire.Mail.desktop x-scheme-handler/mailto`. A running window is reached over D-Bus.
- **Windows:** the MSI registers mailo for `mailto:`; choose it in Default apps. There is no
  hand-off to a running window, so a link starts a new mailo.

### Notifications

`mailo watch` and the window raise a notification for new mail in the inbox that is still unread.
Do Not Disturb is the system's.

- **Linux:** freedesktop notifications. Clicking one opens that conversation in the running window
  (or starts it), passing the click's activation token so a Wayland compositor lets it take focus.
- **macOS:** the notification centre. A click opens its conversation only for notifications the
  window itself shows; a banner from `mailo watch` run in a terminal opens nothing.
- **Windows:** toasts are shown under Windows PowerShell's name, since an unregistered app id
  shows nothing at all.

On a desktop running quire's intent router, `mailo intents` lets an agent or `quire-do mail`
search, read, archive, star, label and snooze conversations and write or send mail, without the
window (`docs/desktop-member.md`).

## Crates

The workspace is nine crates under `crates/`. The sans-I/O crates never open a socket, read a
clock or spawn a task, and `scripts/check-boundary.sh` fails CI if one of them reaches tokio,
rusqlite, dioxus, reqwest or keyring.

Published libraries (each package is named `mailo-*`; its folder keeps the `crates/mail-*` name):

| Crate | For |
| --- | --- |
| `mailo-domain` (`crates/mail-domain`) | The vocabulary: accounts, messages, folders, rules, drafts, identifiers. Pure data and functions. |
| `mailo-mime` (`crates/mail-mime`) | Parsing and building MIME, iMIP, `mailto:`, authentication results and read receipts. |
| `mailo-crypto` (`crates/mail-crypto`) | OpenPGP and S/MIME (behind features), and the mark-certificate check behind BIMI logos: bytes in, values out. |
| `mailo-archive` (`crates/mail-archive`) | mbox, Maildir, `.emlx`, Thunderbird and Takeout archives, read and written without touching a file. |
| `mailo-proto` (`crates/mail-proto`) | Sans-I/O protocol machines for IMAP, POP3, SMTP, ManageSieve, and JMAP's requests and responses: bytes in, "what I need next" out. |
| `mailo-pim` (`crates/mail-pim`) | vCard, iCalendar invitations and the WebDAV/CardDAV conversation, as text in and values out. |
| `mailo-editor` (`crates/mail-editor`) | The composer's rich-text document as pure edits: operations, undo, markdown shortcuts, slash commands, and writers to HTML and `format=flowed`. |
| `mailo-reader` (`crates/mail-reader`) | How a received message becomes what is read: HTML sanitised and lowered to blocks, plain text and quoted tails, the script a message is written in, and the printed page. |

Not published, app-side: `mail-store` (SQLite, FTS5, the outbox), `mail-runtime` (tokio and
sockets), `mail-core` (everything the application does, with no window or terminal),
`mail-client` (the one door to mail: values already read, kept read as the store changes, and the
calls the window makes), `mail-ui` (the window, with no store in its dependencies), `mail-appkit`
(macOS only: the one place that needs `unsafe`, for the AppKit delegate), `mail-edge` (the
process's runtime, environment and Profile, as `mail-core` is handed them), `mail-cli` (the command
line), `mail-intents` (what the desktop's intent router asks of mailo over D-Bus) and `mail-app` (the
`mailo` binary: it makes the client and hands it to the window, and starts the command line or the
intents provider).

## Contributing

Read [CONTRIBUTING.md](CONTRIBUTING.md), the rules the code is written to. A contribution should
pass the checks above.

## Licence

MIT or Apache-2.0, at your option. See `LICENSE-MIT` and `LICENSE-APACHE`.
