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
```

## The two ideas it is built on

**Sans-I/O.** `mail-domain`, `mail-mime` and `mail-proto` never open a socket, read a clock or
spawn a task. A protocol machine takes bytes in and returns `Progress::Need(..)` or
`Progress::Done(..)`; the runtime owns the loop. That is what makes IDLE interruptible and
transcripts replayable, and it is enforced mechanically — `scripts/check-boundary.sh` fails if any
of those three crates can reach tokio, rusqlite, dioxus, reqwest or keyring.

**Enums for mail vocabulary, traits only for real seams.** An account is a value — incoming
protocol, outgoing protocol, auth — not a type. Adding Microsoft meant an enum variant, an
endpoints row and a preset function, and the compiler found every site that had to change.

## Running it

```sh
cargo build --release

./target/release/mailo account add you@gmail.com      # see the help for OAuth and manual hosts
./target/release/mailo watch                          # fetch continuously
./target/release/mailo                                # no arguments opens the window
```

`mailo` with no arguments opens the desktop window; anything else is the command line. One binary,
one store — a separate CLI would drift from what the window does.

Run `./target/release/mailo` with an unknown command to print the full list.

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
