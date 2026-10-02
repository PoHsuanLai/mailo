# mailo

A mail client in Rust: IMAP, POP3 and SMTP, a SQLite store, and a Dioxus desktop window over the
top. Built against two real accounts — a Gmail one and a university POP3 one — because most of
what it got wrong was invisible until it met a real mailbox.

```
mail-app      → mail-core                                the `mailo` binary: the window (ui) and the command line (cli)
mail-core     → mail-runtime, mail-store, mail-domain    the logic: sync, fetching, accounts, compose, rules, search…
mail-runtime  → mail-proto, mail-store, mail-mime        tokio; owns the I/O loop
mail-store    → mail-domain                              SQLite, FTS5, the outbox
mail-proto    → mail-mime, mail-domain                   sans-I/O protocol machines
mail-mime     → mail-domain                              parse, build, sanitize
mail-domain   → serde, uuid, chrono, thiserror           the vocabulary

latchkey      → interprocess                             find this user's agent, or start one
```

[`crates/latchkey`](crates/latchkey) is not about mail and is written to be taken away: it is
the per-user daemon lifecycle — where the socket goes on each platform, who is allowed to be
behind it, and how a client starts one — with no mail in it at all. It is here because this
project needed it and nothing on crates.io does it.

## How it is put together

Obviously the thing does I/O: `mail-runtime` opens sockets and owns a tokio loop, `mail-store`
writes SQLite, `mail-core` is everything the application does with them, and `mail-app` draws a
window and a terminal over that. What the layout buys is *where* it happens.

**The bottom three crates do none of it.** `mail-domain`, `mail-mime` and `mail-proto` never open
a socket, read a clock or spawn a task — that is the sans-I/O pattern, which has never meant "does
no I/O" but "the protocol layer does not perform it". `ImapSession` takes bytes and returns
`Progress::Need(..)` or `Progress::Done(..)`; something above it does the reading. IDLE is
interruptible because the machine is a value someone else drives, and a recorded transcript
replays because nothing in it wanted a socket. It is enforced mechanically rather than by
intention: `scripts/check-boundary.sh` fails if any of those three can reach tokio, rusqlite,
dioxus, reqwest or keyring.

**Two front-ends over one core.** `mail-core` has no window and no terminal in it: it never
depends on dioxus, quire (`ds`, `ds-settings`, `ds-blitz`) or anything else that draws, and the
same script fails if it does. `mail-app` holds the two front-ends side by side — `src/ui` is the
window, `src/cli` the command line, `src/main.rs` the router between them — and neither names the
other, so what they share has to live in `mail-core`. A few core modules still return terminal
prose; `scripts/core-prose-allowlist.txt` and `scripts/core-result-string-allowlist.txt` list
them, to be converted to typed outcomes.

**Enums for mail vocabulary, traits only for real seams.** An account is a value — incoming
protocol, outgoing protocol, auth — not a type. Adding Microsoft meant an enum variant, an
endpoints row and a preset function, and the compiler found every site that had to change.

## Running it

```sh
cargo build --release

./target/release/mailo account add you@gmail.com      # see the help for OAuth and manual hosts
./target/release/mailo watch                          # fetch continuously
./target/release/mailo                                # no arguments opens the window

./target/release/mailo ping                           # reach the daemon, starting one if needed
./target/release/mailo daemon --stop
```

`mailo` with no arguments opens the desktop window; anything else is the command line. One binary,
one store — a separate CLI would drift from what the window does.

Run `./target/release/mailo` with an unknown command to print the full list.

Clicking a new-mail notification from `mailo watch` opens that conversation in a new window (a
second one if a window is already open — a known gap). For the desktop to name and group the
notifications, install the desktop entry (below).

`mailo mailto:someone@example.org?subject=Hello` opens the window on a composer holding what the
link asks for (RFC 6068: `to`, `cc`, `bcc`, `subject` and `body`; every other field is ignored).
Nothing is sent until you send it. With the desktop entry installed, mailo can be chosen as the
system's mail handler, and a `mailto:` link clicked anywhere opens it this way.

## Installing

**From a package.** `./scripts/package.sh` builds a `.deb`, an `.rpm` and a Flatpak, each with
its own external tool (`cargo install cargo-deb`, `cargo install cargo-generate-rpm`,
`flatpak-builder`); a format whose tool is missing is skipped with a note saying which one.
`./scripts/package.sh deb` builds one. Then:

```sh
sudo apt install ./target/debian/mailo_*.deb                    # Debian, Ubuntu
sudo dnf install ./target/generate-rpm/mailo-*.rpm              # Fedora
flatpak install --user ./target/flatpak/mailo.flatpak           # anywhere
```

The Flatpak (`packaging/flatpak/`) has the network, the display, the GPU, the keyring and
notifications, and no files outside its own: attachments, import and export go through the
desktop's file chooser.

**By hand**, into your home directory:

```sh
cargo build --release -p mail-app
install -Dm755 target/release/mailo ~/.local/bin/mailo
install -Dm644 packaging/mailo.desktop ~/.local/share/applications/mailo.desktop
install -Dm644 packaging/icons/hicolor/scalable/apps/mailo.svg \
    ~/.local/share/icons/hicolor/scalable/apps/mailo.svg
update-desktop-database ~/.local/share/applications    # so the mailto: handler is found
xdg-mime default mailo.desktop x-scheme-handler/mailto  # optional: make mailo the mail handler
```

`~/.local/bin` has to be on your `PATH`, since the entry runs `mailo`. Passwords and tokens are
kept in the desktop's keyring (the Secret Service: GNOME Keyring, KWallet, KeePassXC), so one has
to be running.

### The daemon is a prototype

`mailo ping` starts a background daemon on demand and talks to it — the `ssh-agent` pattern, so
nothing has to be installed or enabled first. What is finished is the *transport*: `latchkey`
decides where it lives and who is allowed to be it, and `ipc::wire` is a versioned line protocol
on top. What is not finished is the point of having one — holding IDLE connections in the daemon
rather than in `mailo watch`.

One agent per user is enforced by an advisory file lock rather than by looking at the socket,
which is not a detail: the first version asked the socket, and asking the socket cannot be done
without a race in either direction. `latchkey`'s README has the two of them written out.

## Building on Linux

The window is drawn with Blitz and wgpu through quire's `ds-blitz`; there is no webview and no
script engine in it. The one system library it links at build time is fontconfig
(`fontconfig-devel` on Fedora, `libfontconfig1-dev` on Debian/Ubuntu). Wayland, X11, xkbcommon
and the GPU drivers are opened at run time.

```sh
cargo build --release -p mail-app
```

Print hands a PDF, made by quire, to the system's print dialog (the desktop portal).

`cargo test` needs Noto's CJK faces too (`fonts-noto-cjk` on Debian/Ubuntu, which is what CI
installs): the print tests check which regional face Chinese, Japanese and Korean mail is set in.

The window draws with [quire](https://github.com/PoHsuanLai/quire), the shared design system,
at tag v0.2.2. `crates/mail-app` depends on that tagged release from GitHub, so a plain clone of
mailo builds on its own. `docs/quire-0.2-upgrade.md` says everything about the move.

## macOS and Windows

mailo builds and is tested on both (`portable` in `.github/workflows/ci.yml`) with a plain
`cargo build --release -p mail-app`, needing only the C compiler Rust already needs there (Xcode's
command line tools, Visual Studio's build tools). Passwords and tokens go to the login keychain on
macOS and to the Credential Manager on Windows. Mail and settings go to
`~/Library/Application Support/mailo` on macOS; on Windows mail goes to `%LOCALAPPDATA%\mailo` and
settings to `%APPDATA%\mailo`.

**Installing.** Every push to master builds packages for all three platforms (the `package` job;
download them from the run's artifacts): a `.deb` and an `.rpm`, `mailo-<version>.dmg` holding
`mailo.app`, and `mailo-x86_64.msi`. **The macOS and Windows packages are unsigned.** macOS says the
developer cannot be verified: open it once from Finder with Control-click, Open. Windows SmartScreen
warns about an unrecognised app: More info, Run anyway. Both register mailo for `mailto:` links, to
be chosen as the mail handler in Mail's settings or in Windows' Default apps. To build them
yourself: `packaging/macos/bundle.sh`, or `cargo wix` with `packaging/windows/main.wxs` as the CI
job does.

What is not the same yet: on macOS a click on a notification opens nothing, and a `mailto:` link
starts mailo without handing it the link; on Windows the window is started with a console beside
it, and notifications are shown under Windows PowerShell's name.

## The gates

```sh
cargo test --workspace            # ~2200 tests, no network
cargo clippy --all-targets
cargo fmt --all --check
./scripts/check-boundary.sh       # the sans-I/O boundary
./scripts/live-tests.sh           # real sockets against local servers
cargo deny check licenses
```

`cargo test` never touches the network. Everything that does is `#[ignore]`d and run deliberately
by the scripts above.

## The documents

- **`plan.md`** — the design, and the phases, with what each one actually turned out to cost.
- **`FINDINGS.md`** — every defect worth remembering, what it cost, and why the tests did not
  catch it. It is the most useful file here. Four separate features turned out to be modelled,
  stored, tested and *unreachable* — a capability with no caller looks exactly like a working one
  from inside a repository.
- **`CONVENTIONS.md`** — the rules the code is written to.

## Licence

MIT or Apache-2.0, at your option.
