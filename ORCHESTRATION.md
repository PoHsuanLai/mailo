# Orchestration

How parallel work is divided. Read `plan.md` for the design, `CONVENTIONS.md` for the rules.

The unit of parallelism is **one file behind a frozen signature, plus its tests** — not one
crate. The crate graph is a near-serial DAG (`domain → mime → proto → store → runtime → app`),
so crate-level fan-out would be four agents waiting on one. Fan-out happens *inside* a wave,
against signatures that already compile.

## The rule that makes this work

Every agent codes against the **interface freeze**: `crates/mail-domain/src/**`,
`crates/mail-proto/src/machine.rs`, `crates/mail-store/src/{lib,error}.rs`, and
`crates/mail-store/migrations/0001_initial.sql`. It compiles today, with `todo!()` bodies.

Because it compiles, a disagreement between two agents is a **build error in their own
sandbox**, not a surprise at integration time. That is the entire reason the freeze exists.

An agent that believes a frozen signature is wrong **stops and reports**. It does not work
around it and it does not change it. Four of the five errors corrected in `plan.md` items
26–30 were found exactly this way — by making the freeze compile.

## Status

| Wave | State |
|---|---|
| 1 — `mail-domain` | **done** |
| 2 — `mail-mime`, `mail-store` | **done** |
| 3 — `mail-proto` sessions | **done** — POP3, SMTP, IMAP, modified UTF-7, the replay harness |
| 4 — backends | **done** — `Pop3Backend`, `SmtpBackend`, `ImapBackend` |
| runtime | **done** — transport, drive loop, OAuth + PKCE, loopback listener, secrets, `AccountEngine`, the three-interval schedule |
| app | **done as far as it can be** — `mailo` CLI (list, show, search, reply, send, drafts, status, account add/list, sync) and the Dioxus shell |
| sending | **done** — drafts persist, `RemoteIntent::Send` queues, the engine routes `Submit` to SMTP, end-to-end over a real socket |
| shell compose | **done** — reply and reply-all open a composer, edits autosave to the draft row, Close saves and Discard does not, Send queues through the same `compose` module the CLI uses |
| shell reading | **done** — the HTML part is parsed out of the raw blob and rendered sandboxed, with `cid:` inline images resolved; neither had ever reached the iframe (F40, F42) |
| shell drafts, paging, sync | **done** — Drafts lists the draft table rather than an empty mailbox, "Show more" pages the list, unread badges come from `Store::count`, and a Sync button runs a pass off the UI thread |

526 tests across 48 targets, plus five `#[ignore]`d live tests against servers nobody here
wrote: a capability probe against NTU's Dovecot, two submission tests against a local `aiosmtpd`
(`scripts/live-smtpd.py`), and two sync tests against a local Twisted IMAP4 server
(`scripts/live-imapd.py`). `fmt`, `clippy -D warnings` and `scripts/check-boundary.sh` all
clean.

Sending was the last thing that existed only in pieces. `SmtpBackend` was written and unit-tested
and *unreachable*: no code path put a submission in the outbox, `RemoteIntent` had no variant for
one, the store silently discarded every draft it was given, and no account had an identity to
send from. Four separate gaps, each invisible from inside the component that had it, and all four
found by trying to send a message rather than by reading the code. See FINDINGS F36–F39.

### What still needs a person

Two things, and neither is removed by building more:

1. **A Google OAuth client id.** The browser flow is wired: `MAILO_OAUTH_CLIENT_ID=… mailo
   account add <address>` opens the authorize URL, catches the redirect on a loopback port,
   validates `state`, exchanges the code and stores the credential. What cannot be supplied is
   the client id itself — an installed-app credential registered with the issuer, which is
   deployment configuration and cannot live in a source tree. Without one the command prints the
   exact invocation to re-run.
2. **Live credentials, and a first real connection.** Everything up to the socket is tested
   against transcripts. What a real server does next is precisely what transcripts cannot say,
   and the phase-0 spike already corrected two assumptions that looked safe.

The Dioxus shell compiles and its decisions are tested without a window — which query a place
means, what the reader does with each body kind, which hover actions a thread offers. Whether it
is pleasant to use is not something a test reports, but nothing is blocked on that.

### Phase 5 without a Google OAuth client

IMAP does not need OAuth; Gmail does. The backend used to name `AUTHENTICATE XOAUTH2` itself, so
the whole IMAP path required a client registration — that is fixed (F45), and any password IMAP
server now works:

```
MAILO_PASSWORD='…' mailo account add you@example.com \
    --imap imap.example.com --smtp smtp.example.com [--login NAME]
mailo sync
```

**Not** a Microsoft 365 mailbox. Exchange Online switched off password authentication for IMAP,
POP and SMTP, so a work or school Outlook account cannot use this path however the password is
stored — those need `OAuthIssuer::Microsoft`, which is queued below and not written. `account
add` says so now rather than letting the failure arrive at the first sync (F63). Google is a
different case: an App Password still works, the account password does not.

Ports default to 993 and 465, both implicit TLS. There is no STARTTLS option on purpose: an
opportunistic upgrade is strippable by an active attacker and downgrades silently to a cleartext
password.

`tests/imap_end_to_end.rs` drives the whole stack against a real socket, including phase 5's
second clause — killing the connection mid-body-fetch, restarting against the same database, and
asserting the mailbox holds each message once. It also covers the CONDSTORE sweep: a server that
advertises it gets `CHANGEDSINCE`, one that does not never does.

What remains for phase 5 is Gmail specifically: the OAuth browser flow is wired and tested, but
the client id is an installed-app credential registered with Google, which is deployment
configuration and cannot live in a source tree.

### The shortest path to a working inbox

NTU needs no OAuth registration:

```
MAILO_PASSWORD='…' mailo account add <local-part>@ntu.edu.tw
mailo sync
mailo list
```

`sync` surveys the maildrop, fetches headers with `TOP` so nothing is marked read, then bodies
smallest band first, absorbing each into the store as it arrives.

Replying is the same three steps:

```
mailo reply <message-id> [--all] <<< 'text of the reply'
mailo send <draft-id>
mailo sync
```

`send` freezes the bytes and queues them; it never opens a connection. The message is safe across
a restart from the moment it is queued, and `mailo drafts` says where each one got to.

## Waves## Waves

Each wave lists the files an agent owns **exclusively**. No two agents in a wave write the
same file. Reading anything is always fine.

### Wave 1 — `mail-domain` bodies (4 agents, parallel)

| Agent | Owns | Fills |
|---|---|---|
| `fit` | `src/filter.rs`, `tests/filter.rs` | `Filter::fit` |
| `threading` | `src/threading.rs`, `tests/threading.rs` | `normalize_id`, `thread` (JWZ, RFC 5256) |
| `ops` | `src/op.rs`, `src/message.rs`, `tests/op.rs` | `Op::apply`, `Op::kind`, `ThreadSummary::derive` |
| `presets` | `src/presets.rs`, `src/draft.rs`, `tests/serde.rs`, `tests/fixtures/` | `preset_for`, `Draft::reply_to`, `Draft::forward_of`, the persisted-type round-trip suite |

Notes that decide whether these come back right:

- **`ops`** carries the hardest brief. `Applied.inverse` is computed from the *prior* state,
  not from inverting the op — undoing `Label(x, In)` on a thread that already had `x` is a
  no-op. Its proptest is `apply` → `inverse` → original state, and it is required.
- **`ops`** also owns the `caps` branches: `ArchiveMeans::LocalOnly` and
  `ServerLabels::LocalOnly` must produce `remote: None`, not a queued no-op.
- **`threading`** must handle messages with no `Message-ID` at all. The subject fallback is a
  repair for broken clients, not a general rule; joining two unrelated threads because they
  share a subject is worse than failing to join.
- **`fit`** should expect its work to be re-checked in wave 2 by the parity proptest.

### Wave 2 — `mail-mime` and `mail-store` (3 agents, parallel)

| Agent | Owns | Fills |
|---|---|---|
| `mime` | `crates/mail-mime/**` | `parse`, `build`, `sanitize`, `SanitizePolicy` |
| `sqlite` | `crates/mail-store/src/sqlite/**`, `src/migrate.rs`, `src/blob.rs` | `SqliteStore`, migrations, blob store |
| `parity` | `crates/mail-store/src/memory.rs`, `src/sql.rs`, `tests/parity.rs` | `MemoryStore`, the `Filter` → SQL compiler, the parity proptest |

- **`sqlite`** owns `ingest` and therefore the reconciliation rule (`plan.md` §Reconciliation).
  It is the subtlest code in the project; its tests must include an `Ingest` that contradicts a
  pending change, a `UidValidity::Reset` mid-flight, and a fatal outbox settle that applies the
  undo patch.
- **`parity`** implements `MemoryStore` by *calling* `Filter::fit` — never by writing a second
  matcher. The proptest asserting `fit(f, ctx) == (id ∈ sql(f))` is the point of this brief.
  Two hand-written matchers diverge, and the symptom is "search silently misses a message".

### Wave 3 — `mail-proto` sessions (4 agents, parallel)

**Blocked on:** phase-0 traces, and the replay harness (`tests/replay.rs`), which is shared
infrastructure and is written once before the fan-out.

| Agent | Owns |
|---|---|
| `pop3` | `src/pop3.rs`, `tests/traces/pop3/**` |
| `imap` | `src/imap.rs`, `tests/traces/imap/**` |
| `smtp` | `src/smtp.rs`, `tests/traces/smtp/**` |
| `oauth` | `src/oauth.rs`, `tests/traces/oauth/**` |

Every session needs at least one `SPLIT` trace. A machine that only ever sees whole responses
in tests deadlocks in production the first time a `FETCH` arrives in two reads.

`imap` additionally owns the IDLE-interrupt path: `IoReady::Interrupt` must emit `DONE` and
finish. That trace is named `imap/idle_interrupted.trace` and is not optional.

### Wave 4 — backends and runtime (3 agents, then serial)

| Agent | Owns |
|---|---|
| `imap-backend` | `src/backend/imap.rs` |
| `pop3-backend` | `src/backend/pop3.rs`, `src/backend/fake.rs` |
| `runtime` | `crates/mail-runtime/**` |

`runtime` is the only crate that may `await`, and the only one that may name `tokio`. It owns
the drive loop, and the loop must `select!` the machine against the command channel — that is
what `IoReady::Interrupt` exists for.

### Phases 4–6 — serial, and not fully delegable

Live POP3, live IMAP + OAuth, and the Dioxus shell need real credentials, a browser flow, and
a human looking at a window. Agents can prepare; they cannot finish these.

## Agent brief template

Every brief states all five, or the agent will invent the missing one:

1. **Files you own exclusively.** Everything else is read-only.
2. **Signatures you may not change.** Name them. If one is wrong, stop and report.
3. **What done means.** The tests that must exist, and the cases they must cover.
4. **How you are verified.** The exact command.
5. **What to read first.** `CONVENTIONS.md` §0 (design style) always, plus the relevant
   `plan.md` section and any other `CONVENTIONS.md` section the brief leans on.

## Verification

Every agent runs all four before reporting, and reports honestly if one fails:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
./scripts/check-boundary.sh
```

Use `cargo fmt -p <your-crate>` to format while a wave is running; `cargo fmt --all` rewrites
files other agents have open.

A failing test reported as passing costs more than the bug did, and costs it later, when three
other agents have built on top of it.
