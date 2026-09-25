# mailo

A mail client in Rust: IMAP, POP3 and SMTP, a SQLite store, and a Dioxus desktop window over the
top. Built against two real accounts — a Gmail one and a university POP3 one — because most of
what it got wrong was invisible until it met a real mailbox.

```
mail-app      → mail-runtime, mail-domain, mail-mime     the window and the command line
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
writes SQLite, `mail-app` runs a webview and a terminal. What the layout buys is *where* it
happens.

**The bottom three crates do none of it.** `mail-domain`, `mail-mime` and `mail-proto` never open
a socket, read a clock or spawn a task — that is the sans-I/O pattern, which has never meant "does
no I/O" but "the protocol layer does not perform it". `ImapSession` takes bytes and returns
`Progress::Need(..)` or `Progress::Done(..)`; something above it does the reading. IDLE is
interruptible because the machine is a value someone else drives, and a recorded transcript
replays because nothing in it wanted a socket. It is enforced mechanically rather than by
intention: `scripts/check-boundary.sh` fails if any of those three can reach tokio, rusqlite,
dioxus, reqwest or keyring.

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
notifications, put `mailo` on your `PATH` and install the entry:
`install -Dm644 packaging/mailo.desktop ~/.local/share/applications/mailo.desktop`.

### The daemon is a prototype

`mailo ping` starts a background daemon on demand and talks to it — the `ssh-agent` pattern, so
nothing has to be installed or enabled first. What is finished is the *transport*: `latchkey`
decides where it lives and who is allowed to be it, and `ipc::wire` is a versioned line protocol
on top. What is not finished is the point of having one — holding IDLE connections in the daemon
rather than in `mailo watch`.

One agent per user is enforced by an advisory file lock rather than by looking at the socket,
which is not a detail: the first version asked the socket, and asking the socket cannot be done
without a race in either direction. `latchkey`'s README has the two of them written out.

### Known: the window does not re-render

See **FINDINGS F140**. In this environment a dioxus desktop window is polled once after mount and
never again: clicks reach their handlers and write their signals, and nothing redraws. It
reproduces in `cargo run -p mail-app --example rerender`, thirty lines with no `mail-app` code in
it, so it is not about how this project uses signals. Until it is resolved upstream the command
line is the surface that works, and `mailo watch` is how mail arrives.

## Building on Linux

The desktop window needs webkit2gtk. On Fedora:

```sh
sudo dnf install webkit2gtk4.1-devel gtk3-devel libsoup3-devel libxdo-devel
```

On Debian/Ubuntu: `libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev libxdo-dev`.

That is the default frontend, a WebKitGTK webview (the `webview` feature). A second one draws the
window with Blitz and wgpu through quire's `ds-native`, and needs only fontconfig
(`fontconfig-devel` on Fedora, `libfontconfig1-dev` on Debian/Ubuntu):

```sh
cargo build --release -p mail-app --no-default-features --features native
```

It is not finished: the composer opens but cannot be typed into, Print says it is not available
yet, and remote images stay blocked whatever you allow. The command line is the same under both.

The window draws with [quire](https://github.com/PoHsuanLai/quire), the shared design system.
`crates/mail-app` depends on a tagged quire release from GitHub, so a plain clone of mailo builds
on its own.

## The gates

```sh
cargo test --workspace            # ~1400 tests, no network
cargo clippy --all-targets
cargo fmt --all --check
./scripts/check-boundary.sh       # the sans-I/O boundary
./scripts/live-tests.sh           # real sockets against local servers
./scripts/live-window.sh          # drives the real window and reads what it drew
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
