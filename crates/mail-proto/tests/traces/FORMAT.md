# Trace format

Every protocol test is a transcript. No test in this crate opens a socket; the replay harness
in `crates/mail-proto/tests/replay.rs` feeds a [`Machine`] or [`Backend`] from a file and
asserts what it writes back.

This format is part of the frozen interface. Four agents writing four sessions must not invent
four harnesses.

## Why a text format

Transcripts are read far more often than they are written, and they are the artifact that
survives a rewrite of the session behind them. They must diff cleanly in a pull request, be
hand-editable when probing an edge case, and be obvious when a test fails. Base64 exists for
the cases where bytes are not text, and nowhere else.

## Grammar

One directive per line. Everything after `#` at the start of a line is a comment. Blank lines
are ignored. The file is UTF-8; `\r\n` is appended by the harness and must not be written.

| Directive | Meaning |
|---|---|
| `S: <text>` | Feed `IoReady::Bytes(text + CRLF)` to the machine. |
| `C: <text>` | Assert the machine's next `IoNeed::Write` is exactly `text + CRLF`. |
| `S64: <base64>` | Feed exactly these bytes. No CRLF is appended. |
| `C64: <base64>` | Assert the next write is exactly these bytes. |
| `SPLIT` | Deliver the following `S:`/`S64:` line one byte per `feed`, asserting the machine asks to `Read` again between each. |
| `EOF` | Feed `IoReady::Eof`. |
| `WOKE` | Feed `IoReady::Woke`. |
| `TLS` | Assert the machine asked for `IoNeed::OpenTls`, then feed `IoReady::TlsOpen`. |
| `INTERRUPT` | Feed `IoReady::Interrupt`. |
| `EXPECT <need>` | Assert the next need is `flush`, `close`, or `sleep <secs>`. |
| `DONE` | Assert the machine returned `Progress::Done`. The harness hands the output back so the test asserts on it in Rust. |
| `FAIL <variant>` | Assert `Progress::Failed` with this `ProtoError` variant name. |

`DONE` carries no JSON, which is a deliberate change from the first draft of this format. A
JSON literal in the trace would have to be kept in step with the output type by hand, would not
be checked by the compiler, and would turn a field rename into a silent mismatch across every
transcript. `replay` returns `Ended<M::Out>` instead, and the test writes an ordinary
assertion — type-checked, and refactored along with the type.

A directive's argument follows the first colon (`S: +OK ready`, where the payload may contain
further colons). Directives that take no colon separate on whitespace (`FAIL Refused`,
`EXPECT sleep 5`).

A trace must end in `DONE` or `FAIL`; one that simply stops is rejected, so a truncated
transcript cannot pass by accident. A machine that asks for something the trace does not expect
fails the test with both the expected and the actual need printed, and every panic names the
line number it stopped at.

The harness lives in `tests/common/mod.rs` and is itself tested in `tests/harness.rs` — seven of
those tests assert that it REJECTS a bad trace. A harness that silently passes everything is
worse than none, because it produces green suites that prove nothing.

## Rules

**`SPLIT` is not optional decoration.** Every session must have at least one trace that splits
a multi-line response. Real sockets deliver a `FETCH` response across many reads, and a machine
that only ever sees whole responses in tests will deadlock in production the first time one
arrives in two pieces. Split the responses most likely to be large: IMAP `FETCH`, POP3 `RETR`.

**Scrub before committing.** Traces recorded against a live server contain real addresses,
subjects and message bodies. Replace them, keeping the *shape* — same header order, same
folder names, same UID magnitudes, same encodings. A scrubbed trace that has lost its
quirks has lost its value; the point is to preserve what the server actually did.

**Never edit a passing trace to make a failing test pass.** A trace records what a server did.
If the machine disagrees with it, the machine is wrong. The exception is a trace that was
mis-scrubbed, and that gets said out loud in the commit message.

**Name for the behaviour, not the command.** `imap/idle_interrupted.trace`, not
`imap/test3.trace`.

## Layout

```
traces/
  imap/     greeting, capability, login, select, fetch, store, idle, idle_interrupted, logout
  pop3/     capa, auth, uidl, retr, retr_split, dele, quit
  smtp/     ehlo, auth_plain, auth_xoauth2, submit, rejected
  oauth/    authorize, exchange, refresh, revoked
```

## Example

`pop3/uidl.trace`:

```
# A campus POP3 server, recorded 2026-09-22, addresses scrubbed.
# Note the server sends no CAPA response to an unauthenticated session -- that is real,
# not a recording error, and Pop3Session must cope with it.
S: +OK POP3 server ready
C: CAPA
S: -ERR unknown command
C: USER student
S: +OK
C: PASS <redacted>
S: +OK maildrop has 2 messages
C: UIDL
SPLIT
S: +OK
S: 1 0000000a4b2c1d3e
S: 2 0000000a4b2c1d3f
S: .
C: QUIT
S: +OK bye
DONE
```
