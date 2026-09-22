# Type-driven sans-I/O mail client

Dioxus desktop client. The first two accounts happen to be Gmail and NTU Webmail; those are
**presets that fill an `AccountPlan`**, not domain types. The product object model is
Notion-Mail-shaped: threads, flat labels, mailbox roles, views, actions. UI and sockets sit
outside the core. An account is classified by **incoming protocol, outgoing protocol, and
auth** — IMAP, POP3, SMTP, OAuth, password.

This is a Cargo workspace. `mail-domain` is written first; the rest exist as crates so types
never get dumped into one `lib.rs`.

> Revision note: this supersedes an earlier draft. The substantive corrections are listed in
> [Changes from the first draft](#changes-from-the-first-draft) at the end. Read that section
> if you have the old plan in your head.

---

## Principles

1. **Data first.** Every crate speaks `mail-domain` types. Wire identifiers stay behind
   `RemoteRef`; wire *syntax* stays in `mail-proto`.
2. **Sans-I/O core.** `mail-domain`, `mail-mime`, and `mail-proto` take values/bytes in and
   emit values/bytes out. Nothing below `mail-runtime` opens a socket, reads a clock, or
   spawns a task.
3. **The runtime owns the loop.** Protocol code never calls "give me bytes". It returns
   `Progress::Need(..)` and gets fed. This is what makes IDLE interruptible and traces
   replayable.
4. **Enums for mail vocabulary, traits for seams.** A trait earns its place only if two
   implementations are actually swapped at that boundary.
5. **Optimistic local apply, with an explicit reconciliation rule.** A local change is a
   `Patch`; the server's version arrives as an `Ingest`; pending changes are re-layered on
   top of server truth (see [Reconciliation](#reconciliation)).
6. **No `bool` in domain *state*.** This applies to struct fields, where a named enum
   documents meaning and forbids illegal states. It does *not* apply to predicate returns —
   `Filter::fit` returns `bool` so that `&&`, `!`, and `Iterator::filter` keep working.
7. **Time is an argument, not an ambient service.** Functions that need "now" take
   `now: DateTime<Utc>`. There is no `Clock` trait.

---

## Workspace

Workspace root: this repository.

```
mailo/
  Cargo.toml                 # [workspace] resolver = "3"
  crates/
    mail-domain/             # product types, Filter, Op/Change, RemoteRef, ProtoOp, presets
    mail-mime/               # parse / build / sanitize — pure, domain-typed
    mail-proto/              # sans-I/O machines: sessions AND backends
    mail-store/              # Store trait + SQLite
    mail-runtime/            # tokio, owns the I/O loop, secrets, notify
    mail-app/                # Dioxus 0.7 desktop binary
```

Six crates, not seven. `mail-profiles` is a lookup table over domain types — it becomes
`mail_domain::presets`. `mail-adapters` disappears because backends are machines of the same
shape as sessions, so they belong next to them in `mail-proto`. `mail-mime` is new and
necessary: MIME parse/build/sanitize are pure functions with no `IoNeed`, they are not
sessions, and **`mail-app` needs sanitization without depending on `mail-proto`**.

### Dependency graph

```
mail-app      → mail-runtime, mail-domain, mail-mime
mail-runtime  → mail-proto, mail-store, mail-mime, mail-domain
mail-store    → mail-domain
mail-proto    → mail-mime, mail-domain
mail-mime     → mail-domain
mail-domain   → serde, uuid, chrono, thiserror
```

`mail-domain`, `mail-mime`, and `mail-proto` must not depend on tokio, rusqlite, dioxus,
keyring, or reqwest. Enforce it in CI with `cargo tree -p mail-domain -i tokio` returning
nothing.

### Root `Cargo.toml`

```toml
[workspace]
resolver = "3"
members = [
  "crates/mail-domain",
  "crates/mail-mime",
  "crates/mail-proto",
  "crates/mail-store",
  "crates/mail-runtime",
  "crates/mail-app",
]

[workspace.package]
edition = "2024"
license = "MIT OR Apache-2.0"
rust-version = "1.85"

[workspace.dependencies]
mail-domain = { path = "crates/mail-domain" }
mail-mime   = { path = "crates/mail-mime" }
mail-proto  = { path = "crates/mail-proto" }
mail-store  = { path = "crates/mail-store" }
serde       = { version = "1", features = ["derive"] }
chrono      = { version = "0.4", features = ["serde"] }
uuid        = { version = "1", features = ["v4", "serde"] }
thiserror   = { version = "2" }
```

### Type ownership

| Crate | Owns |
|---|---|
| `mail-domain` | IDs, state enums, `Thread`/`Message`/`Draft`/`View`/`Filter`/`Op`/`Change`/`Patch`/`Ingest`/`AccountPlan`/`AccountCaps`/`RemoteRef`/`ProtoOp`/`SyncCursor`/`Retry`, preset table |
| `mail-mime` | `Parsed`, `Built`, `SanitizePolicy`, `SafeHtml` |
| `mail-proto` | `IoNeed`, `IoReady`, `Progress`, `Machine`, `ImapSession`, `Pop3Session`, `SmtpSession`, `OauthPkce`, `ImapBackend`, `Pop3Backend`, `FakeBackend` |
| `mail-store` | `Store` trait, SQLite schema, migrations |
| `mail-runtime` | `AccountEngine`, the tokio drive loop, `Secrets`, `Effect`, notifications |
| `mail-app` | Dioxus UI. No protocol types. |

`RemoteRef` and `ProtoOp` live in **`mail-domain`**, not in an adapter crate. They must, because
`mail-store` persists them (`remote_map`, `outbox`) and `mail-store` does not depend on
`mail-proto`. They are protocol-neutral vocabulary — "this message, over there" and "do this
remotely" — not wire syntax. Wire syntax stays in `mail-proto`.

---

## `mail-domain`

No traits. No sockets. No `async`.

### Identifiers

Newtypes over `Uuid`: `AccountId`, `ThreadId`, `MessageId`, `DraftId`, `LabelId`, `ViewId`,
`BlobId`, `IdentityId`, `ChangeId`, `OutboxId`.

### State enums

```text
MailboxRole   = Inbox | Archive | Sent | Drafts | Trash | Spam
MailboxSet    = bitset over MailboxRole            // a thread spans several
ReadState     = Unread | Read
Star          = Unstarred | Starred
Membership    = In | Out                           // label add/remove
Attachments   = None | Present { count: u32 }      // 0 unrepresentable
Pin           = Unpinned | Rank(i64)
Snooze        = Inactive | Until(DateTime<Utc>)
SortDir       = Asc | Desc
Threading     = Threaded | Single
LabelOrigin   = User | Provider                    // Gmail categories, IMAP folders
IsDefault     = Default | Alternate
Target        = Threads(Vec<ThreadId>) | Messages(Vec<MessageId>)
```

`Target` is **plural**. Bulk selection is a core mail interaction; a singular target forces N
actions and N outbox rows for "mark 40 as read".

`ThreadSummary.mailboxes` is a `MailboxSet`, not a single role. On Gmail a thread has messages
in Inbox *and* Sent simultaneously; a single role loses that and shows the thread in the wrong
place. Individual `Message`s still carry one `MailboxRole`.

There is no `Calendar` enum in v1. Calendar is a stated non-goal and `None | Invite` is a bool
in a costume.

### Account configuration vs. discovered capability

These are two different things with two different lifetimes and must not be one struct.

```text
AccountPlan = { address: String, incoming: Incoming, outgoing: Outgoing,
                auth: AuthPlan, identities: Vec<Identity> }        // configured, persisted

Incoming    = Imap { host, port, tls: Tls }
            | Pop3 { host, port, tls: Tls, leave: LeaveOnServer }
Outgoing    = Smtp { host, port, tls: Tls }
Tls         = Implicit | StartTlsRequired | Plaintext
LeaveOnServer = Keep | DeleteAfterFetch
```

`Tls` has **no opportunistic StartTLS variant**. Opportunistic StartTLS is strippable by an
active network attacker and silently downgrades to cleartext credentials. Either the server is
required to upgrade or you knowingly chose `Plaintext`.

```text
AccountCaps = { labels:   ServerLabels,     // Supported | LocalOnly
                threads:  ServerThreads,    // ProviderId | Jwz
                watch:    WatchMode,        // Idle | Poll { every: Duration }
                archive:  ArchiveMeans,     // DropInbox | MoveToFolder(String) | LocalOnly
                folders:  FolderRoles,
                condstore: Condstore,       // Supported | Absent
                move_ext:  MoveExt,         // Supported | Absent
                observed_at: DateTime<Utc> }              // discovered, cached, refreshable

FolderRoles = Vec<(String /* IMAP path */, MailboxRole)>
```

A preset supplies *expected* caps as a starting value. The runtime replaces them from
`CAPABILITY` / `LIST (SPECIAL-USE)` on connect. Keeping discovered capabilities inside the
persisted `AccountPlan` forces you either to ship presets that lie or to mutate the user's
saved config on every connect.

`Condstore` matters more than it looks: without it, noticing that a message was marked read in
another client requires `FETCH 1:* (FLAGS)` over the whole mailbox on every poll. With it, one
`CHANGEDSINCE` fetch.

`ArchiveMeans::MoveToFolder` is new — generic IMAP servers archive by `MOVE`, not by dropping
`INBOX` membership the way Gmail does.

### Auth

```text
AuthPlan  = OAuth { issuer: OAuthIssuer, scopes: Vec<String> }   // no client_id: see below
          | Password { username: Username, sasl: Vec<SaslMech> }

OAuthIssuer = Google | Microsoft
Username    = SameAsAddress | LocalPart | Literal(String)
SaslMech    = Plain | Login | CramMd5 | XOauth2       // ordered by preference
Identity    = { id, account, from: Address, reply_to: Option<Address>,
                signature: Option<String>, default: IsDefault }
```

`Username::Literal` exists because a login name is not always derivable from the address.
`SaslMech` is a list because some campus servers only offer `LOGIN`.

Secrets are not strings:

```text
SecretKey = { account: AccountId, purpose: SecretPurpose }
SecretPurpose = IncomingPassword | OutgoingPassword | OAuthRefresh
Credential = Password(String)
           | OAuth { access: String, refresh: String, expires_at: DateTime<Utc> }
```

Incoming and outgoing may need different credentials. An OAuth credential has an expiry and a
refresh token, and the runtime schedules refresh off `expires_at`.

### Content

```text
Address     = { name: Option<String>, email: String }
Body        = { text: Option<String>, raw: BlobId }      // sanitize at render, not at ingest
Attachment  = { name: String, mime: String, size: u64, blob: BlobId, inline: Inline }
Inline      = Attached | Embedded { cid: String }
Label       = { id, account, name, color, origin: LabelOrigin }
```

`Body` stores the **raw** HTML blob and no `html_safe`. Storing sanitizer output permanently
means an `ammonia` upgrade leaves every previously-ingested message sanitized under the old
rules. Sanitize on render via `mail-mime`, cache the result keyed by
`(BlobId, SanitizePolicy::VERSION)`.

### Threads and messages

```text
Message      = { id, thread, account, date, from, to, cc, bcc, subject,
                 in_reply_to, references, rfc_message_id, key: MessageKey,
                 read: ReadState, star: Star, mailbox: MailboxRole,
                 labels: Vec<LabelId>,
                 body: Body, attachments: Vec<Attachment> }

ThreadSummary = { id, account, subject, snippet, from, participants, last_date,
                  message_count,
                  read: ReadState,          // derived: Unread if any message Unread
                  star: Star,               // derived: Starred if any message Starred
                  mailboxes: MailboxSet,    // derived: union over messages
                  labels: Vec<LabelId>,     // derived: union over messages
                  attachments: Attachments,
                  snooze: Snooze, pin: Pin }

Thread        = { summary: ThreadSummary, messages: Vec<MessageId> }
```

The derivation is a named, tested function in this crate:

```rust
impl ThreadSummary {
    pub fn derive(id: ThreadId, messages: &[Message],
                  snooze: Snooze, pin: Pin) -> ThreadSummary;
}
```

`snooze` and `pin` are passed through rather than derived: they are thread-level state the
user set, and no message carries them.

It must be named because `Op::apply` on a `Target::Threads` fans out to every message in the
thread and then recomputes the summary — so applying an op needs the thread *and* its messages,
not "a loaded row".

### Message identity

```text
MessageKey = Rfc(String)        // Message-ID header, normalized
           | Gmail(u64)         // X-GM-MSGID
           | Synthetic([u8;32]) // blake3 of (Date, From, Subject, first 4KiB) when Message-ID absent
```

This is the deduplication key, and it is **not** `RemoteRef`. See below.

### Remote references and sync

```text
RemoteRef  = Imap { mailbox: String, uidvalidity: u32, uid: u32 }
           | Pop  { uidl: String }

MailboxRef = { account: AccountId, path: String }        // "INBOX" for POP3

SyncCursor = Imap { uidvalidity: u32, uidnext: u32, modseq: Option<u64> }
           | Pop                                          // no cursor; diff UIDL each poll

UidValidity = Same | Reset
```

**`RemoteRef → MessageId` is many-to-one, not a bijection.** On Gmail the same message exists
in `INBOX` and `[Gmail]/All Mail` under *different UIDs*, and again in `[Gmail]/Sent` if you
sent it. `remote_map` is keyed `(account, mailbox, uidvalidity, uid)` and several rows point at
one `MessageId`. Identity comes from `MessageKey`. Building the store on a bijection duplicates
every message on the first Gmail sync.

`SyncCursor` is **per mailbox**, not per account, and lives in a `sync_state` table keyed by
`MailboxRef`. When the server reports a different `UIDVALIDITY`, every `remote_map` row for that
mailbox is invalid — `UidValidity::Reset` is how an `Ingest` says so.

**A reset is a re-map, not a refetch.** The mapping is dropped; the messages are not. Recovery
fetches headers only, computes each [`MessageKey`], and repoints `remote_map` at the
`MessageId`s we already hold, downloading bodies only for keys we have genuinely never seen.
The distinction is the difference between an 8 MB reconciliation and re-downloading the whole
mailbox, and it is not hypothetical: Dovecot's default POP3 UIDL embeds `UIDVALIDITY`, so a
server-side configuration change invalidates every UIDL in the maildrop at once.

### Views and queries

```text
ViewKind  = Place { mailbox: MailboxRole } | PlaceLabel { label: LabelId } | Query
Property  = Date | Subject | From | Sender | Size | Attachments | Pin
View      = { id, name, kind: ViewKind, filter: Filter, sort: Sort,
              group_by: Option<Property>, threading: Threading,
              shown: Vec<Property>,      // columns
              hover: Vec<OpKind> }       // hover-strip buttons
Sort      = { property: Property, dir: SortDir }
Query     = { filter: Filter, sort: Sort, page: PageReq }
PageReq   = { after: Option<Cursor>, limit: u32 }
Page<T>   = { items: Vec<T>, next: Option<Cursor> }
```

`View.hover` holds `OpKind`, not `Op` — see [Ops](#ops-and-changes). Views never go on the wire;
they are local SQLite rows owned by this app.

### Filter

```text
Filter    = All | Nothing
          | And(Vec<Filter>) | Or(Vec<Filter>) | Not(Box<Filter>)
          | Account(AccountId)
          | InMailbox(MailboxRole)
          | Read(ReadState) | Starred(Star)
          | HasLabel(LabelId)
          | From(TextMatch) | To(TextMatch) | Subject(TextMatch) | Text(TextMatch)
          | Date(DateRange)
          | HasAttachment
          | Snoozed | SnoozeDue
          | Pinned

TextMatch = Contains(String) | Exact(String)
DateRange = { from: Option<DateTime<Utc>>, to: Option<DateTime<Utc>> }   // half-open [from, to)
```

Three deliberate changes from a naive design:

- **Full-text folds in as `Filter::Text`.** A sibling `Query.fts` field makes
  `Or(text_match, From(x))` inexpressible.
- **Predicates, not state mirrors.** `Snoozed`/`SnoozeDue` instead of `Snooze(Snooze)`;
  `HasAttachment` instead of `Attachments(Present { count })`, which would have meant "exactly
  N attachments". An enum that mirrors a state enum is rarely a useful predicate.
- **Relative dates resolve in the UI.** `Filter` only ever holds absolute instants, so a saved
  view means the same thing when it is reloaded.

```rust
pub struct MatchCtx<'a> {
    pub summary: &'a ThreadSummary,
    pub body_text: Option<&'a str>,
    pub now: DateTime<Utc>,
}

impl Filter {
    pub fn fit(&self, ctx: &MatchCtx<'_>) -> bool;   // bool, not a Fit enum
}
```

**`fit` and the SQL compiler are two implementations of one semantics and will diverge.** The
bug looks like "search misses a message" and is miserable to find. Mitigations, both required:

1. `MemoryStore::threads` is implemented *by calling `fit`* — never by a second hand-written
   matcher.
2. A proptest in `mail-store/tests/`: generate random `Filter`s and random `ThreadSummary` +
   body corpora, assert `fit(f, ctx) ⟺ id ∈ sqlite_query(f)` for every row.

### Ops and changes

```text
OpKind = Archive | Trash | Restore | Spam | MarkRead | MarkUnread
       | Star | Unstar | AddLabel | RemoveLabel | Snooze | Pin

Op     = Archive | Trash | Restore | Spam
       | SetRead(ReadState) | SetStar(Star)
       | Label(LabelId, Membership)
       | SetSnooze(Snooze) | SetPin(Pin)

Action = { target: Target, op: Op }
```

**Reply, Forward, and Send are not `Op` variants.** They were, and it forced `Op::Reply(Compose)`
into `View.hover`, where no `Compose` exists yet. The root cause was conflating "which action"
with "that action's payload". The fix: a reply *creates a draft*, and drafts have their own
lifecycle.

```rust
impl Draft {
    pub fn reply_to(msg: &Message, ident: &Identity, all: ReplyScope, now: DateTime<Utc>) -> Draft;
    pub fn forward_of(msg: &Message, ident: &Identity, now: DateTime<Utc>) -> Draft;
}
// ReplyScope = Sender | All
```

`OpKind` is the fieldless mirror used wherever you need "what button is this" — hover strips,
keybindings, undo labels.

Applying an op is pure and returns its own inverse:

```rust
pub struct Applied {
    pub forward: Patch,
    pub inverse: Patch,          // computed here, where the prior value is known
    pub remote: Option<ProtoOp>, // None when caps say this is local-only
}

impl Op {
    pub fn apply(&self, target: &Target, thread: &Thread, msgs: &[Message],
                 caps: &AccountCaps, now: DateTime<Utc>) -> Applied;
}
```

Note `inverse` is produced **at apply time, not by `Op::invert()`**. A standalone `invert` cannot
work: the inverse of `AddLabel(x)` on a thread that already had `x` is a no-op, and the inverse
of `Archive` depends on which mailboxes the thread was in. The prior state is only available
here.

```text
Change = MessageRead(MessageId, ReadState)
       | MessageStar(MessageId, Star)
       | MessageMailbox(MessageId, MailboxRole)
       | MessageLabel(MessageId, LabelId, Membership)
       | ThreadSnooze(ThreadId, Snooze)
       | ThreadPin(ThreadId, Pin)
       | MessageUpsert(Box<Message>)
       | MessageDelete(MessageId)
       | LabelUpsert(Label)
       | DraftUpsert(Box<Draft>)
       | DraftDelete(DraftId)

Patch  = { id: ChangeId, changes: Vec<Change> }
```

`Change` is **domain-level**, never a SQL row. The previous draft defined `Patch` as "local row
mutations", which put the SQLite schema inside `mail-domain`.

### Ingest — bulk facts from a sync

A sync fetch is not a `Patch`. It is large, it is not invertible, and it *is* the truth rather
than an optimistic guess. Same enum for both means a 300-line match in `Store::apply`.

```text
Ingest = { mailbox: MailboxRef,
           validity: UidValidity,
           cursor: SyncCursor,
           messages: Vec<Fetched>,                        // new or refetched
           flags: Vec<(RemoteRef, ReadState, Star)>,      // cheap flag-only sync
           labels: Vec<Label>,
           gone: Vec<RemoteRef> }                         // expunged on the server

Fetched = { remote: RemoteRef, key: MessageKey, raw: BlobId, message: Message }
```

`gone` is the fix for a real hole: nothing in the previous plan handled a message deleted from
another client, so it would never disappear locally.

### Drafts and sending

```text
Draft     = { id: DraftId, account, identity: IdentityId,
              to, cc, bcc, subject,
              in_reply_to: Option<MessageId>, forward_of: Option<MessageId>,
              text: String, html: Option<String>,
              attachments: Vec<PendingAttachment>,
              state: SendState, updated: DateTime<Utc> }

PendingAttachment = { name: String, mime: String, blob: BlobId }
SendState = Editing
          | Queued
          | Sending
          | Failed { reason: String, retry: Retry }
          | Sent { at: DateTime<Utc>, message: Option<MessageId> }
```

Attachments are `BlobId`, never a path: reading the file is I/O and must happen in the runtime
*before* the pure domain op. Drafts are persisted locally and, when
`ServerLabels::Supported`, `APPEND`ed to the Drafts folder — but local-first, so an offline
draft is never lost.

### Remote work and failure

```text
ProtoOp  = FetchCaps
         | ListFolders
         | FetchEnvelopes { mailbox: MailboxRef, since: FetchSince }
         | FetchBody { remote: RemoteRef }
         | SetFlags { remotes: Vec<RemoteRef>, read: Option<ReadState>, star: Option<Star> }
         | SetMailbox { remotes: Vec<RemoteRef>, role: MailboxRole }
         | SetLabels { remotes: Vec<RemoteRef>, add: Vec<String>, remove: Vec<String> }
         | Append { mailbox: MailboxRef, raw: BlobId, role: MailboxRole }
         | Submit { draft: DraftId, raw: BlobId }
         | Expunge { remotes: Vec<RemoteRef> }
         | Watch { mailbox: MailboxRef }

FetchSince = Beginning | After { cursor: SyncCursor }

Retry    = Now
         | After(Duration)
         | NeedsReauth                 // token revoked, password rejected
         | Fatal(String)               // message gone, permanent 5xx reject
```

`Retry` is the missing piece that made the outbox undesignable: backoff has nothing to decide on
without a retryable/fatal/needs-user split, and "reconnect this account" is a UI state that has
to come from somewhere. Every error type in the workspace exposes `fn retry(&self) -> Retry`.

### Presets (module, not a crate)

`mail_domain::presets` maps a domain string to an `AccountPlan` plus *expected* `AccountCaps`.

| Domain | Incoming | Outgoing | Auth |
|---|---|---|---|
| `gmail.com`, `googlemail.com`, Workspace | IMAP `imap.gmail.com:993` Implicit; expected caps: labels Supported, threads ProviderId, watch Idle, archive DropInbox, condstore Supported | SMTP `smtp.gmail.com:465` Implicit | OAuth `{ issuer: Google }`, `Username::SameAsAddress` |
| `ntu.edu.tw` | POP3 `msa`/`ccms`.ntu.edu.tw:995 Implicit, `LeaveOnServer::Keep` | SMTP `smtps.ntu.edu.tw:465` Implicit | Password, `Username::LocalPart`, sasl `[Login, Plain]` |

Host choice for `ntu.edu.tw` (student-id vs. name local-part) is preset logic kept next to the
table. Nothing in the domain is named after a school or a vendor except `OAuthIssuer::Google`,
which names an authorization server, not a mail provider.

### Threading

JWZ (RFC 5256) over parsed `Message-Id` / `In-Reply-To` / `References`, **written in this crate**
— roughly 300 lines. `ServerThreads::ProviderId` lets an adapter supply `X-GM-THRID` as a hint,
but JWZ still runs so POP3 and IMAP share one set of `ThreadId` rules. See
[Dependencies](#dependencies) for why this is not an external crate.

---

## `mail-mime`

Pure. `&[u8]` in, domain types out.

```rust
pub fn parse(raw: &[u8]) -> Result<Parsed, MimeError>;          // mail-parser
pub fn build(draft: &Draft, parts: &[(BlobId, &[u8])]) -> Result<Vec<u8>, MimeError>;  // mail-builder
pub fn sanitize(html: &str, policy: SanitizePolicy) -> SafeHtml; // ammonia

pub struct SanitizePolicy { pub remote_images: RemoteImages, pub version: u32 }
pub enum RemoteImages { Blocked, Allowed }
```

`SanitizePolicy::version` is bumped whenever the policy or the `ammonia` major changes, and it
is part of the cache key for rendered HTML. Both `mail-proto` (parsing fetched bytes, building
outgoing messages) and `mail-app` (rendering) depend on this crate; that is exactly why it
cannot live inside `mail-proto`.

---

## `mail-proto`

Depends on `mail-mime` + `mail-domain`. No sockets, no async runtime, no `std::net`, no
`std::time::Instant`. Tests are byte transcripts in `tests/traces/`.

### The one shape

```rust
pub enum Progress<T> {
    Need(Vec<IoNeed>),
    Done(T),
    Failed(ProtoError),
}

pub enum IoNeed {
    Write(Vec<u8>),
    Read,
    Flush,
    OpenTls { host: String, port: u16, mode: Tls },
    Sleep(Duration),
    Close,
}

pub enum IoReady {
    Bytes(Vec<u8>),
    Eof,
    TlsOpen,
    Woke,
    Interrupt,          // runtime asks the machine to wind down gracefully
}

pub trait Machine {
    type Out;
    fn start(&mut self) -> Progress<Self::Out>;
    fn feed(&mut self, ready: IoReady) -> Progress<Self::Out>;
}
```

Three things this shape buys that the previous `trait IoDrive { fn pump(&mut self, need) -> IoReady }`
could not:

- **It is implementable.** `pump` was synchronous while `TokioDrive` must `await`. The only
  reconciliations were `block_on` inside the reactor or making the entire backend chain `async fn`
  in trait, losing `dyn` and the generic. Returning needs to a runtime-owned loop has neither
  problem.
- **Completion and output exist.** The previous `Session::feed(IoReady) -> Vec<IoNeed>` had no way
  to say "the FETCH is finished" or to hand back the envelopes.
- **`IoReady::Interrupt` is the cancellation surface.** IMAP IDLE blocks for up to 29 minutes;
  when the user archives a thread the runtime must inject `DONE` and reuse the connection. There
  was previously no way to express this anywhere in the design.

Partial reads are natural: `feed(Bytes(..))` with an incomplete response returns
`Progress::Need(vec![IoNeed::Read])` again.

### Machines

| Machine | `Out` | Purpose |
|---|---|---|
| `ImapSession` | `ImapReply` | capability, auth, select, fetch, store, idle, logout |
| `Pop3Session` | `Pop3Reply` | USER/PASS or SASL, UIDL, RETR, DELE, QUIT |
| `SmtpSession` | `SmtpReply` | EHLO, AUTH PLAIN/LOGIN/XOAUTH2, MAIL/RCPT/DATA |
| `OauthPkce` | `Credential` | authorize URL, redirect consumption, refresh |
| `ImapBackend` | `ProtoOutcome` | `ProtoOp` → IMAP walk → `Ingest`/confirmation |
| `Pop3Backend` | `ProtoOutcome` | `ProtoOp` → POP3 walk → `Ingest`/confirmation |
| `FakeBackend` | `ProtoOutcome` | fixtures, no bytes at all |

```rust
pub enum ProtoOutcome {
    Ingested(Ingest),
    Caps(AccountCaps),
    Applied,                              // flags/labels confirmed on the server
    Submitted { remote: Option<RemoteRef> },
    Woken,                                // IDLE saw activity; runtime schedules a fetch
}

pub trait Backend {
    fn begin(&mut self, op: ProtoOp) -> Progress<ProtoOutcome>;
    fn feed(&mut self, ready: IoReady) -> Progress<ProtoOutcome>;
    fn caps(&self) -> &AccountCaps;
}
```

Backends are machines of the same shape as sessions — that is why there is no separate
`mail-adapters` crate and no `IoDrive` trait. A backend test is a byte transcript, identical in
kind to a session test.

There is **no `Session` trait beyond `Machine`**. `Session` was not a seam: nothing ever swaps
IMAP for POP3 at the session level, and the four impls have unrelated `Out` types.

### Backend behaviour

- **`ImapBackend`** — reads `AccountCaps`. `ServerLabels::Supported` maps `LIST` results and
  keywords (plus `X-GM-LABELS` when advertised) into labels. `ArchiveMeans::DropInbox` archives
  by removing `INBOX` membership; `MoveToFolder` uses `MOVE` when `MoveExt::Supported`, and
  otherwise **copies without expunging** — see the box below. `WatchMode::Idle` uses IDLE.
  `Condstore::Supported` fetches
  flags with `CHANGEDSINCE`. `\Seen` ↔ `ReadState`, `\Flagged` ↔ `Star`. Non-special folders
  become `LabelOrigin::Provider` labels.

> **`\Deleted` + `EXPUNGE` is forbidden on Gmail, structurally.**
>
> An earlier version of this plan gave `COPY` + `STORE \Deleted` + `EXPUNGE` as the archive
> fallback whenever `MOVE` is unavailable. On Gmail that is a data-destruction path. Gmail maps
> `EXPUNGE` through a per-account `expungeBehavior` setting which is `archive`, `trash`, or
> **`deleteForever`** — and there is no capability, no `STATUS` item and no other way to read it
> over IMAP, so we cannot tell which accounts are armed. On an account set to `deleteForever`
> that sequence destroys the user's mail irrecoverably, and our `Patch` undo restores only the
> local row.
>
> Worse, the fallback triggers precisely when `MOVE` is absent — which is the state F14 caught us
> in, because Gmail does not advertise `MOVE` before authentication.
>
> So `AccountCaps` gains `expunge: ExpungeMeans`, defaulting to `Forbidden` for
> `ServerLabels::Supported` accounts. Where expunging is forbidden, "move" is a copy plus a label
> change and nothing is ever deleted. Leaving a stray copy is a cosmetic failure; deleting
> someone's mail is not, and a capability we cannot observe is not one we may gate on.

- **`Pop3Backend`** — steady state is `UIDL` every poll, diff against `remote_map`, `RETR` what
  is new. **The first sync is a different algorithm**, because the measured NTU maildrop is 2372
  messages and 255 MB and a naive pass is both slow and destructive:

  1. `CAPA`, authenticate, then `STAT` + `UIDL` + `LIST`. Two multi-line responses buy a complete
     map of the maildrop, including an exact size per message, before fetching anything.
  2. **Headers via `TOP n 0`, newest first, pipelined.** `RETR` sets the seen flag on Dovecot and
     `TOP` does not, so a `RETR`-everything first sync would mark the user's entire mailbox read
     in their webmail. This pass alone makes every message listable, threaded and searchable by
     subject and sender.
  3. **Bodies in size bands, newest first.** The measured distribution is extreme: 90% of
     messages are 10% of the bytes, and the hundred largest are 80%. Fetching the ≤64 KiB band
     first completes in about a minute and covers everything anyone is realistically going to
     open; the long tail of attachments follows, or waits until asked for.
  4. Checkpoint by `QUIT` and reconnect every few hundred messages. Message numbers are
     session-scoped, so re-`UIDL` on every reconnect and key all persisted state on the UIDL.

  Every pass is independently resumable, and a message is durably stored before any `DELE`.
  `SetMailbox`/`SetLabels` produce no wire traffic and confirm immediately. `WatchMode` is always
  `Poll`. `LeaveOnServer` controls `DELE`.
- **`FakeBackend`** — fixtures to `Ingest`, no I/O, used from phase 3 onward.

SMTP is not a third incoming backend; it is `Outgoing`, driven by both.

### Generic IMAP folders

A plain IMAP account with `ServerLabels::LocalOnly` organizes by arbitrary folders
(`Work/2024`). `MailboxRole` is a closed six-variant enum on purpose, so the rule is explicit:
**special-use folders map to roles via `FolderRoles`; every other folder becomes a
`LabelOrigin::Provider` label.** Moving a thread to such a label is a `SetLabels` that the
backend translates into an IMAP `MOVE`.

---

## `mail-store`

SQLite (WAL) via `rusqlite` with `bundled` + `fts5`. Does disk I/O. Opens no sockets.

```rust
pub trait Store {
    fn threads(&self, q: &Query, now: DateTime<Utc>) -> Result<Page<ThreadSummary>, StoreError>;
    fn count(&self, f: &Filter, now: DateTime<Utc>) -> Result<u64, StoreError>;
    fn thread(&self, id: ThreadId) -> Result<Thread, StoreError>;
    fn message(&self, id: MessageId) -> Result<Message, StoreError>;

    fn apply(&self, account: AccountId, patch: &Patch) -> Result<(), StoreError>;
    fn ingest(&self, account: AccountId, ingest: Ingest) -> Result<Patch, StoreError>;

    fn enqueue(&self, op: ProtoOp, undo: &Patch) -> Result<OutboxId, StoreError>;
    fn outbox_due(&self, now: DateTime<Utc>) -> Result<Vec<OutboxEntry>, StoreError>;
    fn outbox_settle(&self, id: OutboxId, result: Settle) -> Result<(), StoreError>;
}
```

`threads` is **paginated** and there is a separate `count`. Returning `Vec<ThreadSummary>` for a
whole view is fatal at 100k threads, and it is a trait signature that propagates into every
caller; sidebar unread badges need counts without loading rows.

`ingest` returns a `Patch` describing what actually changed, so the UI can refresh precisely
instead of re-querying everything.

### Tables

`accounts` (`AccountPlan` as JSON + `schema_version`), `account_caps`, `identities`, `labels`,
`threads`, `messages`, `message_labels`, `thread_summary`, `blobs`, `views`, `drafts`,
`remote_map` (account, mailbox, uidvalidity, uid | uidl → message_id — **many-to-one**),
`sync_state` (`MailboxRef` → `SyncCursor`), `outbox`, `pending_changes` (message_id → outbox_id),
`messages_fts` (FTS5 over subject, from, text body).

Blobs are hash-addressed files under `~/.local/share/mailo/blobs/`; small parts may live inline.
Raw `.eml`, unsanitized HTML, and attachments all go here.

### Learning what changed elsewhere

The design had no mechanism for this, and it is not an edge case — it is every user with a phone.

**Gmail's IDLE never reports flag changes.** IDLE is a new-mail signal and nothing else, so a
message read or starred on another device never arrives through it. **And Gmail has no QRESYNC**,
only CONDSTORE, which RFC 7162 §1 is explicit about: a CONDSTORE-only client "still has to issue
a UID FETCH or a UID SEARCH" to discover expunges. Together those mean a watch loop alone can
never learn that a message was read, starred, or deleted somewhere else.

So a sync pass is three things, not one:

1. **New mail** — IDLE where offered, otherwise a poll. This is the only part IDLE gives us.
2. **Flag changes** — a timed `UID FETCH 1:* (FLAGS) (CHANGEDSINCE <modseq>)` where
   `Condstore::Yes`, which is cheap because the server returns only what moved. Where CONDSTORE
   is absent or its `HIGHESTMODSEQ` is observed not to advance — Dovecot 2.0.18 returned `1`
   forever while `EXISTS` climbed — this degrades to a full flag fetch on a longer interval.
3. **Disappearances** — a periodic `UID SEARCH ALL` diffed against `remote_map`. Without QRESYNC
   there is no cheaper way, and skipping it means a message deleted elsewhere stays forever.

`AccountCaps.watch` therefore describes only step 1. Steps 2 and 3 have their own intervals and
run regardless of whether IDLE is available.

### Reconciliation

The rule that makes optimistic apply safe, and the one the previous draft was missing entirely:

1. A user op writes `Patch` immediately and enqueues a `ProtoOp` plus its `undo` patch.
   `pending_changes` records which messages are affected.
2. An `Ingest` arrives carrying server truth. For each message, the store writes the server
   value, then **re-applies every still-pending `Change` on top of it** before committing.
3. On `Settle::Ok`, pending rows are cleared.
4. On `Settle::Failed(Retry::Fatal(_))` or `NeedsReauth` after the user declines, the `undo`
   patch is applied and the UI is told.

Without step 2, the next poll after starring a message flips the star back and the UI flickers.
This is the single most common bug in optimistic mail clients.

### Ordering

Per account, the outbox drains **serially**. Two ops on one thread plus an `Ingest` arriving
between them otherwise have no defined result.

### Migrations

`accounts` stores a serialized `AccountPlan`; the moment `AccountPlan` gains a field, every
stored account fails to deserialize and the app will not start. Required from day one:

- A `schema_version` table and numbered, forward-only migration steps.
- `#[serde(default)]` on every added field — *unless no safe default exists*, which is a case
  this design did not anticipate and `CONVENTIONS.md` §3 now spells out. A defaulted empty
  recipient list would turn an old `ProtoOp::Submit` into a message that goes nowhere while the
  outbox reports success (FINDINGS F37, F82).
- A test that builds a database at each prior version and migrates it. `tests/upgrade.rs` does
  this by applying a prefix of `MIGRATIONS` rather than checking in a binary fixture, so the
  fixture cannot drift from the migration it represents. It matters most for 0002, whose whole
  job is repairing data an empty database does not have.

---

## `mail-runtime`

Tokio. One task per account. **This crate owns every loop.**

```rust
pub struct AccountEngine<B: Backend> {
    plan:     AccountPlan,
    account:  AccountId,
    backend:  B,
    store:    Arc<SqliteStore>,
    secrets:  Arc<dyn Secrets>,
    schedule: Schedule,
    last:     LastRun,
}
```

As built, with two changes from the sketch above. There is no `caps` field: capabilities belong
to the backend, which is what discovers them, and a second copy here would be a second answer to
one question. `schedule` and `last` are the three intervals — new mail, flag changes,
disappearances — which the sketch predates.

One generic parameter, not five. `Store` and `Secrets` are process-wide singletons — one SQLite
file with one WAL connection (not a pool; see `SqliteStore`), one keyring — so making them type parameters (or worse,
`Box<dyn Store>` per account) misrepresents ownership. There is no `Clock` parameter; `now` is an
argument.

```rust
pub trait Secrets: Send + Sync {
    fn get(&self, key: &SecretKey) -> Result<Credential, RuntimeError>;
    fn put(&self, key: &SecretKey, value: &Credential) -> Result<(), RuntimeError>;
    fn forget(&self, key: &SecretKey) -> Result<(), RuntimeError>;
}
```

The drive loop, which is the only place `await` meets a protocol machine:

```rust
loop {
    tokio::select! {
        ready = transport.next(needs) => match backend.feed(ready) {
            Progress::Need(n)  => needs = n,
            Progress::Done(o)  => { settle(o)?; break }
            Progress::Failed(e) => { retry(e.retry())?; break }
        },
        cmd = commands.recv() => {
            // user acted mid-IDLE: wind the machine down and take the new op
            needs = match backend.feed(IoReady::Interrupt) { .. };
        }
    }
}
```

Per-account responsibilities: connect and refresh `AccountCaps`; watch (IDLE or interval) →
`FetchEnvelopes` → `Ingest` → store → notify UI; drain the outbox serially with backoff driven by
`Retry`; refresh OAuth credentials before `expires_at`; surface `NeedsReauth` to the UI.

The client id an OAuth account renews with is **not** in its `AccountPlan`. It is deployment
configuration — one per issuer per build channel, and not a secret, since an installed
application cannot keep one — so it lives in `signin::OAuthRegistry`, a JSON file at
`$XDG_CONFIG_HOME/mailo/oauth.json` that `account add` writes and every sync reads. Without it
the refresh token in the keyring cannot be spent and an OAuth account works for exactly one
token lifetime.

Secrets via `keyring` (Secret Service). Tokens and passwords never touch SQLite. TLS via
`rustls` + `tokio-rustls` + `webpki-roots`. Desktop notifications via `notify-rust`.

The OAuth loopback redirect listener lives here. It must send and verify a `state` parameter in
addition to PKCE.

---

## `mail-app`

Dioxus 0.7 desktop (`dioxus` + `dioxus-desktop`), WebKitGTK on Fedora.

- Sidebar: `View` list. Inbox is `Filter::InMailbox(Inbox)`. Badges come from `Store::count`.
- List: `store.threads(&query, now)`, paginated. **As built:** a "Show more" that grows the page
  rather than following `Page::next`. One query for the visible list is always self-consistent,
  where pages fetched at different moments are the inconsistency keyset pagination exists to
  prevent; the cursor is there when a mailbox is large enough to measure (FINDINGS F-notes on
  scale). Migration 0003 is what makes that page cost a page rather than a mailbox.
- Hover strip: `view.hover: Vec<OpKind>` → `Action { target, op }`.
- Reading: raw HTML from the blob through `mail_mime::sanitize` at render time, into a
  **sandboxed iframe** (`srcdoc`, no `allow-same-origin`). Remote images blocked by default.
- `cid:` **does not** resolve through a custom protocol handler, and this is the one place the
  shell departs from this document. A custom scheme requested from a `sandbox=""` document is
  refused as cross-origin, and the only way to permit it is `allow-same-origin` — which hands
  every future sanitizer bug access to the application's DOM, the single thing that iframe
  exists to prevent. The parts are inlined as `data:` URIs instead, which satisfies this
  section's actual reason: it forbids a path-shaped handler because that is "a directory-
  traversal bug driven by untrusted mail", and this resolves nothing at request time at all.
  The boundary moved to the media type, which is attacker-controlled, so the type in the message
  decides only *whether* to embed and never what is written. See FINDINGS F42.
- Compose: edits a `Draft`, autosaves through `Op`-free `Change::DraftUpsert`, sends via
  `SendState`.

Depends on `mail-runtime`, `mail-domain`, `mail-mime`. Not on `mail-proto`.

---

## Dependencies

Versions checked against crates.io on 2026-09-22.

| Crate | Version | Role | Notes |
|---|---|---|---|
| `mail-parser` | 0.11.9 | MIME parse | **feature `full_encoding`** — `big5`, `gbk`, `shift_jis`, `euc-kr` are gated behind it, and NTU mail is Taiwanese |
| `mail-builder` | 1.0.0 | MIME build | 1.0, not 0.5 |
| `ammonia` | 4.2.0 | HTML sanitize | |
| `rusqlite` | 0.40.2 | store | features `bundled`, `fts5`. Not 0.32. |
| `keyring` | 4.2.0 | secrets | Not 3. |
| `tokio` | 1 | runtime | |
| `rustls` / `tokio-rustls` / `webpki-roots` | current | TLS | |
| `dioxus` | 0.7.10 | UI | 0.8 is alpha; stay on 0.7 |
| `notify-rust` | 4.18 | notifications | |
| `serde` / `uuid` / `chrono` / `thiserror` | current | domain | |
| `imap-proto` | 0.16.7 | **IMAP response parsing** | MIT/Apache, 3.5M downloads, `nom::streaming`. Ships `parser/gmail.rs` |
| `oauth2` | 5.0.0 | Google PKCE | `default-features = false`; our runtime makes the HTTP call |
| `chardetng` | 1.0.0 | charset detection | for headers and bodies whose declared charset is wrong |

**`imap-codec` cannot parse Gmail, and the plan's fallback was not one.** `imap-types`'s
`MessageDataItem` has fourteen variants and no catch-all, and its FETCH parser is a
`delimited('(', separated_list1(..), ')')` — so an unrecognised attribute makes the closing
parenthesis fail and **the entire untagged FETCH becomes a parse error**, not merely a missing
field. `X-GM-MSGID`, `X-GM-THRID` and `X-GM-LABELS` are data items rather than headers, so there
is no way around it, and that kills `MessageKey::Gmail` and `ServerThreads::ProviderId`. Both
`io-imap` and `imap-next` depend on that same parser, so swapping one for the other changes the
loop wrapper and keeps all of the parsing risk. `imap-proto` parses Gmail's extensions today.

`io-imap` itself is genuinely sans-I/O and a clean architectural match for `Machine`; the
problem is underneath it. The phase-0 decision is therefore not "io-imap or imap-next" but
"`imap-proto` plus our own sessions" versus "fork `imap-codec` to add the Gmail data items".

**Write SASL ourselves** (~60 lines). `sasl` 0.5.2 is MPL-2.0 and disqualified; `rsasl` is
permissive and healthy but is a negotiation framework for a problem we do not have, and has no
XOAUTH2 — the one mechanism both Gmail and Microsoft actually need.

**Write modified UTF-7 ourselves** (~80 lines, no dependencies). `utf7-imap` 0.3.2 is MIT but
unmaintained since 2022, and its decoder ends in `base64::decode(..).unwrap()` reached from a
regex that accepts any bytes, so a mailbox named `&A-` panics the process. There is no `Result`
anywhere in its 189 lines. A decoder whose failure mode is the whole account going offline is
worse than none: on malformed input the correct behaviour is to treat the name as literal bytes
and carry on. Take its MIT test vectors.

**`OauthPkce` is not a `mail-proto` machine.** `oauth2` emits an `http::Request`, not bytes, and
`IoNeed` has no variant for one. A token exchange is stateless and has nothing to replay from a
transcript, so the purity rule buys nothing here. OAuth lives in `mail-runtime` beside the
loopback redirect listener.

**Write JWZ threading ourselves.** `mail-threading` 0.1.3 exists but has ~1,400 lifetime
downloads, one unknown author, and no activity since June 2026. That is an unacceptable
dependency for a core correctness algorithm that you will want to tune against your own corpus
anyway. It is ~300 lines in `mail-domain`.

**Write `Pop3Session` ourselves.** POP3 is small and no good sans-I/O crate exists. 200–400 lines.

**Commit to one implementation per protocol after phase 0.** The pimalaya `io-*` trio is the
largest schedule risk in the project: three 0.x crates, one maintainer, ~6k downloads each. The
mitigation is real — our own `IoNeed`/`IoReady` boundary means a swap does not touch
`mail-domain` — but that only holds if the boundary is *the* interface rather than a hedge.
Phase 0 evaluates `io-imap` against a real Gmail connection and the decision is written down,
with the swap cost to `imap-next` estimated.

Skipped: `imap` 2.4 (unmaintained), `melib` (GPL), `email-lib` (superseded by `io-*`),
Microsoft Graph, `io-gmail` REST (IMAP suffices for v1).

---

## Phases

### 0 — Spike (one day, thrown away)

A single dirty script, no crates, no types, deleted when done.

- OAuth into Gmail with your own installed-app client id; dump `CAPABILITY`, `LIST (SPECIAL-USE) "" "*"`,
  and `FETCH 1:5 (UID FLAGS ENVELOPE BODYSTRUCTURE X-GM-MSGID X-GM-THRID X-GM-LABELS)`.
- Confirm the same message in `INBOX` and `[Gmail]/All Mail` under different UIDs.
- Check whether `CONDSTORE`/`QRESYNC`/`MOVE` are advertised.
- POP3 to `ntu.edu.tw`: `UIDL`, `LIST`, `RETR 1`, and whether it wants `LOGIN` or `PLAIN`.
- Pump `io-imap` from tokio for ten minutes and see whether it is pleasant.

**Why this is first:** the previous plan froze `RemoteRef`, `AccountCaps`, `ProtoOp`, and
`SyncCursor` in phase 1 with zero contact with a real server, and deferred Gmail — the source of
every hard question in this document — to the *last* substantial phase. It also asked phase 2 for
"recorded traces" that cannot be recorded without first connecting. One day here removes both
problems and the dumps become the phase-2 fixtures.

**Done when:** raw transcripts are saved to `crates/mail-proto/tests/traces/` (credentials
scrubbed) and the `io-imap` decision is written down.

### 1 — Workspace + `mail-domain`

- Root workspace with all six members; stub `lib.rs` elsewhere.
- All types above, serde with `#[serde(default)]` discipline.
- `Filter::fit`, `Op::apply` (returning `Applied` with its inverse), `ThreadSummary::derive`,
  JWZ threading, the presets table.
- Table-driven tests plus a proptest that `apply` then `inverse` round-trips to the original
  state.

**Done when:** `cargo test -p mail-domain` covers filter, derive, threading, and archive/label
apply+invert. `cargo tree -p mail-domain -i tokio` is empty.

### 2 — `mail-mime` + `mail-proto`

- `parse` / `build` / `sanitize`; a reply round-trips with correct `In-Reply-To` / `References`.
- `Pop3Session` against the phase-0 transcript.
- `ImapSession` (or the chosen wrapper) against the phase-0 Gmail transcript.
- `SmtpSession` AUTH PLAIN and XOAUTH2 as byte scripts.
- `ImapBackend` / `Pop3Backend` as `Backend` machines, transcript-driven.

**Done when:** proto tests pass against **zero** live servers.

### 3 — `mail-store`

- Schema, migrations + a fixture-DB migration test, `apply`, `ingest`, FTS, `remote_map`,
  `sync_state`, outbox.
- `MemoryStore` implemented via `Filter::fit`.
- The `fit` ⟺ SQL proptest.
- `FakeBackend` ingesting 20 fixture messages.

**Done when:** a test ingests fixtures, archives one, labels one, searches, paginates, and the
reconciliation rule is exercised by an `Ingest` that contradicts a pending change.

### 4 — POP3 + SMTP live

Password from keyring, POP poll, SMTP send to self, `LeaveOnServer::Keep`, local archive.
First live preset: `ntu.edu.tw`.

**Done when:** a tiny CLI lists, opens, and replies through the POP3 backend.

**State:** the CLI does all three — `list`, `show`, `reply`, plus `send`, `drafts` and `sync` —
and the whole path is exercised end to end over a real socket in
`mail-runtime/tests/end_to_end.rs`. What has not happened is a pass against NTU with a real
password, which needs a credential this repository cannot hold. An unauthenticated probe
(`tests/live_probe.rs`, `#[ignore]`d) does confirm the preset against the live server: `TOP`,
`UIDL`, `PIPELINING` and `SASL PLAIN` are exactly what `msa.ntu.edu.tw` advertises, and the TLS
handshake completes.

### 5 — IMAP + OAuth live

Own Google OAuth client (installed app, PKCE + `state`, loopback redirect). IDLE, `CONDSTORE`
flag sync, `UIDVALIDITY` reset handling, expunge handling, labels. First live preset:
`gmail.com`.

**Done when:** the same CLI works through the IMAP backend, and killing the app mid-sync and
restarting produces no duplicates.

**Folders.** A pass fetches `INBOX` and `Sent`, not everything the server lists — see FINDINGS
F127. `Archive` waits on a message carrying a set of mailboxes rather than one, because on Gmail
the same message is in INBOX *and* in All Mail and a pass over All Mail would mark inbox mail
archived. `mailo account list` says which folders an account fetches.

**State:** both clauses are met against servers that are not Google's.
`mail-runtime/tests/imap_end_to_end.rs` drives the whole stack over a real socket — including the
restart clause, which drops the connection mid-literal, rebuilds the engine against the same
database and asserts each message is present once — and `tests/live_imap.rs` repeats `LOGIN`,
`SELECT` and `UID FETCH` against a Twisted `IMAP4Server`, which nobody here wrote. Password IMAP
works; `sync::pass` is one function used by both protocols, so "the same CLI" is not a claim but
a shared code path.

What is left is Gmail and Exchange specifically, and it is not code: an installed-app client id
is registered with the issuer, not shipped in a source tree. Everything up to that point is
exercised — the authorize URL, `state`, PKCE, the loopback redirect, and the token exchange and
refresh against a local endpoint (`tests/oauth_exchange.rs`).

Renewal was the piece that had no caller. `oauth::refresh` could exchange a refresh token from
the day it was written and nothing ever asked it to, and the client id needed to do so was read
from the environment at `account add` and then thrown away — so an OAuth account would have
fetched mail for one token lifetime and failed on every pass afterwards, permanently. That is
now `signin::OAuthRegistry` plus `signin::renew`, driven from `sync::signed_in`, and proved
end to end through the binary against a local token endpoint: expired token in, renewed token
in the keyring, and a second pass that does not ask the issuer again.

### 6 — Dioxus shell

Three panes, view list, paginated list, sandboxed HTML, compose and drafts, runtime channel →
Dioxus signals.

**Done when:** it is the daily driver for both accounts on Fedora.

**State:** everything the criterion implies that can be checked without being a user has been.
The shell composes, replies, sends, lists drafts, pages, syncs off the UI thread and renders
sandboxed HTML with inline images; its components are executed in tests rather than only
compiled. Scale is measured rather than assumed: search over ten thousand messages, a page of
the list, a two-hundred-message thread, and a sync writing while the window reads.

Running it as a person still finds things the tests do not, which is the argument for the
criterion rather than against it. F98 — every date in the application rendered in UTC, including
the attribution line quoted into outgoing replies — was invisible to forty-three view tests and
obvious within one minute of reading `mailo list` on a machine in `+0800`. F99, a `reply` that
offered a `send` no command could make work, came from the next minute.

The shell renders, its root holds focus, and the two journeys a mail client is for have been
watched happening in it: `j` opens a conversation and `e` archives it (F108); pressing Sync
fetches a message that was waiting on the server and the list grows from two rows to three; `j`
then `r` opens a composer addressed to the right person, and `Escape` closes it with what was
typed saved as a draft (F109). All from inside the live page, against a real database and a real
IMAP server. What remains of the criterion is the criterion: days of real mail.

The shell now has a keyboard (F103) — j/k to move, e to archive, s and u to toggle, r and a to
reply, Escape to close — and F104 closed the part F103 could not verify: `VirtualDom::handle_event`
delivers a key press to the real component tree over a real database, so `j` then `e` is asserted
to leave the inbox one conversation shorter, and `j`, `r`, `e` is asserted to archive nothing.
The same mechanism settled the other open question: a task spawned from an event handler does
run, so the Sync button was never broken; it is only a task spawned from a component body that
never starts.

Looking at the window turned out to be possible after all, and F100 is what it found: subjects
truncated to twenty characters beside an empty reader, `Sep 22` on mail that arrived an hour ago,
and "Load remote images" offered above every conversation in the mailbox. `dioxus-ssr` renders the
same components to HTML, `STYLE` inlines, and a headless browser that was already installed takes
the picture — see `ui::render_tests::render_the_shell_to_a_file`. It is the markup and the CSS
rather than the running application, and the same call is what finally lets the shell's output be
*asserted* instead of only executed.

Looking at the window is now one command rather than a technique rebuilt each round.
`scripts/live-window.sh` seeds a real store, starts the real binary, and has the page type into
its own controls and report what it then showed — no input tooling, no forced backend, and it
works with the screen locked, which is where the previous round stopped and should not have. The
seam it uses is kept out of release builds by `cfg(debug_assertions)`.

The criterion itself is not one this or any amount of work can satisfy from inside the
repository. "Daily driver" is a judgement about using it, with real mail, over days.

### 7 — The surface that is missing

Phases 1–5 built everything under the window and phase 6 built the window. Neither asked what a
person can *do* in it that is not reading, and the answer turned out to be: less than it looks.

Four of the five items below are the same shape as F128, F131 and F136 — a capability that is
modelled, stored, tested and unreachable. That shape has now appeared four times here, and it is
worth naming as the characteristic failure of a typed design rather than an accident: the
compiler proves the data is right and proves nothing about whether anything asks for it. A type
with no caller compiles, tests green, and reads from inside the repository exactly like a
feature.

**7a — Write a new message.** `Draft` carries `to`, `cc`, `bcc`, `subject`, `text`, `html` and
`identity`; `Composer` edits one and autosaves it; `compose::send` sends one. There is no way to
*make* one that is not a reply or a forward — `compose.rs` has `draft_reply` and `draft_forward`
and nothing else. Mail can only be written to someone who has written first, which is the
difference between a mail client and a mail reader, and it is the largest hole in the product.
Needs `compose::draft_new`, a New control with `c` in the keymap, and `mailo compose --to`.

The one decision is which account a new message leaves from, because unlike a reply there is no
message to infer it from. The account behind the selected place, changeable in the composer:
with two accounts configured a silent default is a message sent from the wrong address, which is
not a failure the sender sees.

**7b — Attach a file.** `Draft.attachments: Vec<PendingAttachment>` exists, `mail_mime::build`
assembles multipart from it, `sqlite/draft.rs` persists it, and nothing in `mail-app` has ever
constructed one. The worst of the four, because the send path looks like it supports attachments
right until someone needs one. Needs a file chooser, the bytes into a blob, and a size said out
loud before the server refuses the message rather than after.

**7c — Save an attachment.** Small, and mostly the same dialog as 7b. The reader already lists
names and sizes and `attach::save` already writes safely — refusing to overwrite, and sanitising
a name that arrived from a stranger. Today the pane prints the `mailo save` command for the user
to run, which was the honest thing to do before there was a dialog and is furniture after.

**7d — Labels, both directions.** `label:` searches, sync ingests, `Op::Label` applies locally
and `X-GM-LABELS` carries it to Gmail. `OpKind::AddLabel` renders a button captioned "Label"
that opens nothing. Needs a chooser over `Shell::labels`, which is already refilled on every
revision.

**7e — Snooze from the window.** The Snoozed place lists correctly and the vocabulary —
"tomorrow", "next week" — is already parsed for the CLI. Nothing in the window can snooze a
conversation.

IDLE is the sixth gap and is not here; it is a long-lived connection and belongs with the rest of
the concurrency work in phase 8.

**Done when:** a message can be written, addressed, given a file and sent without touching the
command line, and the window can label and snooze what it lists. Verified the way phase 6's
journeys were — through `VirtualDom::handle_event` against a real store, and through
`scripts/live-window.sh` against the real binary.

### 8 — Concurrency, and the frame budget

The argument for doing this in Rust at all, cashed in. It is deliberately after phase 7, because
a client that cannot write a message does not need to write it faster.

**What is true today**, read rather than assumed:

- `SqliteStore` holds one `Connection` behind a `ReentrantMutex` (`sqlite/mod.rs:42`), so every
  read waits behind every write *inside this process*. The comment above the pragmas says WAL
  "lets a reader run while a writer commits, which is what keeps the UI responsive during a
  sync". That is true of SQLite and false of this struct: with one connection, WAL only buys
  concurrency against a separate `mailo sync` process. The single case the comment names is the
  single case it does not cover.
- Rendering does I/O on the render thread. Every `use_memo` — the thread list, six badge counts,
  the label index, drafts — reads the store synchronously, and `Reader` additionally reads a blob
  and runs the whole MIME parse, sanitize and inline-embed per message. `reader.rs` already
  records the consequence: the open thread re-renders on every keystroke in the search box.
- Sync is strictly sequential: a `new_current_thread` runtime, `for account in accounts` with a
  `block_on` each (`sync.rs:163`), and within an account each mailbox does sync, then sweep, then
  bodies. Gmail's pass blocks NTU's entirely.

**The principle.** The sans-I/O core was argued for as a testing and boundary discipline. It is
also the concurrency story, and that is the larger dividend: a pure function has no shared
mutable state, so it parallelises without locks and caches without invalidation. The only things
in this application that need synchronising are the store and the sockets. Everything else —
`Filter::fit`, `ThreadSummary::derive`, MIME parse, sanitize, the protocol machines — is a value
going in and a value coming out.

**8a — Measure first.** Done, in `mail-app/tests/frame_budget.rs`, and it moved everything below
it. Release build, one keystroke in the search box with a conversation open:

| | 3,138 messages (real) | 10,000 messages |
|---|---|---|
| list of 50 | 0.36 ms | 0.19 ms |
| **six badge counts** | **4.65 ms** | **11.97 ms** |
| label index | 0.01 ms | 0.00 ms |
| drafts | 0.01 ms | 0.01 ms |
| reader, 40 messages | — | 0.89 ms |
| search | 0.05 ms | 13.01 ms |
| **one keystroke** | **5.07 ms** | **26.08 ms** |

Two things in that table were not what this plan predicted. The reader is *cheap* — 0.89 ms for a
forty-message conversation, against the paragraph above that called it the thing that stutters —
and the badge counts are the single largest cost at both sizes, six sequential `COUNT`s run on
the render thread on every revision. The mailbox this was measured against is fine today at 5 ms;
at ten thousand messages a keystroke costs 26 ms, which is a frame and a half at 60 Hz, and that
is what typing into the search box with a mailbox a year old will feel like.

So the order below is the measurement's, not the one this section was first written with: what is
on the render thread matters more than what the store costs, and the badges matter more than the
reader. `8d` and `8e` stay in the plan because a cache of a pure function is still free and still
correct — but they are no longer where the milliseconds are, and they are not first.

**8z — The window does not re-render, and it is first.** F140, and it is settled rather than
suspected: a hidden element clicked by the page reaches its Rust handler thirty-eight times in
twelve seconds while the component renders twice and a tokio timer ticks not at all. The event
loop is alive and dispatching; the dom is never polled again. `dioxus-desktop/src/waker.rs` shows
why — waking the dom sends `UserWindowEvent::Poll` through tao's `EventLoopProxy`, that event is
the only thing that calls `poll_vdom`, and the send result is discarded.

It reproduces in `examples/rerender.rs` with no `mail-app` code in it, and it cannot be worked
around from application code: the proxy is `pub(crate)`. So this item is not an implementation
task. It is a report to file upstream, and until it is answered the rest of this phase is mostly
theatre — there is no point moving work off the render thread when the render thread has stopped
rendering. What that costs today is written up in FINDINGS F140.

**8b — A reader connection per thread.** One writer, N readers, so WAL delivers what its own
comment already claims. This is the only item here scheduled ahead of its measurement, because it
is not an optimisation: it is the removal of a lock that should never have been the design, and
it is the precondition for everything else being worth doing.

**8c — Reads off the render thread.** `use_memo` → `use_resource`, with cancellation, so a query
superseded by the next keystroke is dropped rather than awaited.

**8d — Cache the pure render.** `reader::render` is `(BlobId, SanitizePolicy) → Reading`. Blobs
are content-addressed — `BlobStore::put` dedupes by hash — so the key is immutable by
construction and **the cache can never need invalidating**. That is the functional design paying
rent: in a client with a mutable message object this cache would be a bug farm, and here it is a
map. Bounded by bytes, outside the store.

**8e — Speculative render.** Once 8d exists, precompute the rows around the cursor while the user
reads, so opening a conversation costs a lookup. Other clients parse on open; this parses before
open. The same idea points the body backfill at what is visible instead of at arrival order.
This is the item that produces a feeling the alternatives do not have, and it is worth nothing
until 8b, 8c and 8d are in.

**8f — Parallel sync.** Accounts are independent by construction: `AccountId` partitions every
table. One task per account on a multi-thread runtime fetches both at once. Within an account,
mailboxes are independent too, and because `ImapBackend` is a sans-I/O *value*, four of them is
four values and four sockets with nothing shared to protect — the concrete payoff of "the runtime
owns the loop". Gmail permits fifteen connections.

The caveat is the reason this is not free: more connections is more ways to be throttled, and
`Retry::After` is honoured per account rather than per connection (F130). A shared limiter comes
first, or this turns a working client into a rate-limited one.

**8g — IDLE.** `AccountEngine::watch` has handled IDLE since phase 3 and has no caller outside
tests; F128 chose the poll loop deliberately and `sync.rs:114` says so. Last, because it converts
"up to five minutes late" into "instant", which is the smallest user-visible win on this list and
the one with the most failure modes — a connection held open for hours, through sleep, network
changes and a server that drops it silently.

**Not a database problem.** Asked and answered, so it is not asked again. The contention above is
reader-versus-writer inside one process, caused by one connection behind a mutex; the writers
themselves are a batched ingest, single-row UI patches and a rare outbox, and two of them only
meet when a terminal `sync` overlaps the window, which `busy_timeout` already covers. Turso was
considered and is disqualified on its own documentation rather than on taste — under MVCC
"indexes cannot be created and databases with indexes cannot be used", the whole database is
loaded into memory on first access, and the project is beta with MVCC experimental; its
full-text search is Tantivy behind `fts_match` rather than FTS5, so the CJK bigram work would be
rewritten. Worth revisiting if MVCC stabilises with indexes, since it is pure Rust and
file-format compatible. SurrealDB is a worse fit for a structural reason: SurrealQL replaces the
SQL layer, and with it the `fit` ⟺ SQL proptest that is the only thing keeping `MemoryStore` and
`SqliteStore` from silently disagreeing. A mail archive also wants the most durable file format
available, which is the one already in use.

**Done when:** the bench from 8a is re-run and says what changed; typing in the search box with a
large thread open drops no frames; a sync pass fetches both accounts at once; and opening a
conversation the user was about to open is a lookup.

---

## Test strategy

- **Domain** — table-driven unit tests plus proptests for `apply`/`inverse` round-trip and
  `ThreadSummary::derive`.
- **Proto** — byte transcripts in `mail-proto/tests/traces/`, recorded in phase 0, scrubbed.
  Backends and sessions test identically because they are the same shape.
- **Store** — tempfile SQLite; the `fit` ⟺ SQL proptest; migration tests against checked-in
  fixture databases.
- **Reconciliation** — a dedicated suite: pending change + contradicting `Ingest`, fatal outbox
  failure + undo, `UIDVALIDITY` reset mid-flight.
- **Live** — behind `#[ignore]`, in `mail-runtime`.

---

## Microsoft accounts — the preset is written; a spike is not

**Done since this section was drafted:** `OAuthIssuer::Microsoft`, the endpoints row, the preset,
and `mailo account add <address> --microsoft` for a tenant on its own domain. Adding it required
exactly what this section predicted — an enum variant, an endpoints row and a preset function —
and the compiler found the single site that had to change. `Incoming::Imap`, `Outgoing::Smtp` and
`ImapBackend` are untouched.

Two corrections to the draft below, from writing it:

- **A custom tenant domain cannot be recognised from the address.** `you@yourcompany.com` says
  nothing about Microsoft, so only `*.onmicrosoft.com` is matched automatically and everything
  else needs `--microsoft`. The alternative is autodiscover, which points the client at a host
  the user never named.
- **Submission is STARTTLS on 587, not implicit TLS on 465.** Exchange Online does not offer
  implicit TLS for SMTP AUTH, which is the one place its shape differs from Gmail's.

What remains is the spike, and it is unchanged: the three tenant-policy questions below are facts
about an organisation, not about a protocol, and no amount of code answers them.

## Queued — Microsoft accounts (after phase 6)

Two more accounts are wanted: a work Outlook mailbox and a school one. Both are managed
Microsoft 365 tenants, which is a materially different case from a personal Outlook.com mailbox
and, for us, a better one.

**The protocol fits the design unchanged.** Exchange Online supports IMAP and SMTP over OAuth
using SASL XOAUTH2, in exactly the encoding Gmail uses:

```
base64("user=" + address + "\x01auth=Bearer " + token + "\x01\x01")
```

So the `SmtpSession` XOAUTH2 path written for Gmail works for Microsoft with no change, and the
delegated scopes are just values in a preset:

| Protocol | Delegated scope |
|---|---|
| IMAP | `https://outlook.office.com/IMAP.AccessAsUser.All` |
| POP | `https://outlook.office.com/POP.AccessAsUser.All` |
| SMTP | `https://outlook.office.com/SMTP.Send` |

plus `offline_access` for refresh tokens. In domain terms that is `OAuthIssuer::Microsoft`, a
preset row, and nothing else: `Incoming::Imap`, `Outgoing::Smtp` and `ImapBackend` are untouched.
This is the first outside test of the claim that a provider is a value rather than a type, and
it passes.

**The admin-consent burden does not apply to us.** Microsoft's documentation requires tenant
admin consent *and* a `New-ServicePrincipal` registration in Exchange Online PowerShell — but
that is for the **client credentials** flow, where an application accesses mailboxes with no user
present, using the `IMAP.AccessAsApp` family of permissions. We are an interactive desktop
client using the **authorization code** flow with delegated permissions, where the user signs in
themselves. That path needs neither step.

**What can still block it is tenant policy, not protocol**, and a work tenant and a school tenant
may answer differently:

1. The tenant may restrict user consent to third-party applications, in which case an
   administrator has to approve the app once even for delegated access.
2. IMAP and POP can be disabled per mailbox or organisation-wide (`Set-CASMailbox`).
3. SMTP AUTH is disabled by default in many tenants and is enabled per mailbox.

None of that is discoverable from documentation — it is a fact about each organisation. A small
spike answers all three, as phase 0 did for Gmail and NTU, and it is harder only because there is
no app-password equivalent: it needs an Entra application registration first.

**If a tenant forbids the protocols outright**, the fallback is Microsoft Graph, which is REST
rather than SMTP and is a second backend, not a preset row. `Outgoing` is an enum that the
codebase never matches with a `_` wildcard, so adding `Outgoing::Graph` would produce a compile
error at every site that must change — which is the reason the vocabulary is enums. Nothing needs
doing today beyond not writing code that assumes submission is always SMTP.

Personal Outlook.com is a worse case and is not planned: basic authentication was retired there
on 2024-09-16, and recently-created personal mailboxes are reported to have SMTP client
authentication permanently off, failing even under OAuth.

## Non-goals (v1)

Microsoft Graph / Exchange as a v1 protocol. Calendar and invites. CardDAV (local frecency contacts from message
history are in scope; a protocol is not). OpenPGP / S/MIME. Nested labels. Proton. Incoming
protocols beyond IMAP and POP3. Multi-device sync of local-only state (views, pins, snoozes).

Unified inbox is `Filter` without an `Account` clause. Note honestly that a conversation you are
on from both accounts appears twice, because `ThreadId` is per account. Cross-account thread
merging is a v2 problem.

---

## Risk notes

- **pimalaya `io-*` are 0.x, single-maintainer, blocking-coroutine shaped.** Largest schedule
  risk. Phase 0 decides; the `IoNeed`/`IoReady` boundary keeps `mail-domain` unaffected by a swap
  to `imap-next` + `lettre`.
- **Gmail's folder/label duality** is the main modelling risk and is why phase 0 exists. The
  many-to-one `remote_map` and `MessageKey` deduplication are the defenses.
- **`fit` vs. SQL divergence** — two implementations of one semantics. Defended by `MemoryStore`
  reusing `fit` and by the proptest. If the proptest is ever skipped, this bug ships.
- **Serialized `AccountPlan` in SQLite** — a field addition bricks startup without migrations.
  Cheap to build now, expensive to retrofit after real accounts exist.
- **POP3 has no push.** Poll interval and a later "purge server" action are product decisions,
  not protocol types. Server quota is a fact you observe, not a type.
- **Gmail OAuth client id is yours.** Personal use of an unverified installed-app client is fine;
  distributing to other people is a separate Google verification problem, and restricted Gmail
  scopes make it a real one.
- **Untrusted HTML in WebKit.** Sanitize at render, sandboxed iframe without `allow-same-origin`,
  remote images blocked by default, `cid:` keyed on `BlobId`. Never render mail in the app origin.
- **TLS to campus servers.** If NTU presents a chain `webpki-roots` rejects, resist adding a
  global "accept invalid certs" switch. If it becomes unavoidable, it is per-account, explicit in
  `AccountPlan`, and loud in the UI.

---

## Changes from the first draft

Recorded so the diff is reviewable rather than archaeological.

**Corrected — these were wrong, not merely improvable:**

1. `IoDrive` deleted. `pump(IoNeed) -> IoReady` was synchronous and could not be implemented over
   tokio; it also had no cancellation path for IDLE. Replaced by `Progress` + a runtime-owned
   loop + `IoReady::Interrupt`.
2. `RemoteRef ↔ MessageId` is **not** a bijection. Gmail stores one message in several mailboxes
   under different UIDs. `remote_map` is many-to-one; identity is `MessageKey`.
3. `RemoteRef` and `ProtoOp` moved to `mail-domain`. The ownership table put them in
   `mail-adapters` while the dependency graph gave `mail-store` — which persists both — no way to
   see them.
4. `Patch` split into `Patch`/`Change` (domain-level, invertible, optimistic) and `Ingest` (bulk
   server truth, with `gone` for expunges). It was previously undefined and described as "row
   mutations", putting the SQL schema inside the domain.
5. `imap-codec` 2 does not exist (1.0.0); `rusqlite` 0.32 → 0.40, `mail-builder` 0.5 → 1.0,
   `keyring` 3 → 4, `io-oauth` 0.2 → 0.3.

**Added — missing surfaces:**

6. `SyncCursor` per mailbox, `UidValidity` reset, `Condstore`, expunge/`gone`.
7. The reconciliation rule for optimistic apply, `pending_changes`, and undo-on-fatal.
8. `Retry` (`Now`/`After`/`NeedsReauth`/`Fatal`) so the outbox has something to decide on.
9. Drafts as first-class (`Draft`, `SendState`, `PendingAttachment`), `Identity`, signatures.
10. Pagination (`PageReq`/`Page`) and `Store::count`; plural `Target` for bulk selection.
11. `AccountCaps` split from `AccountPlan`; `FolderRoles` and the generic-IMAP folder rule.
12. `MessageKey`, `SaslMech`, `Username::Literal`, `Credential` with expiry, per-purpose
    `SecretKey`.
13. SQLite migrations and migration tests.
14. Serial outbox ordering per account; OAuth `state` alongside PKCE; `cid:` keyed on `BlobId`.

**Simplified:**

15. `Session` trait deleted — it was not a seam and could not express completion or output.
16. `AccountEngine<B, S, K, C, D>` → `AccountEngine<B>`; `Store`/`Secrets` are shared singletons.
17. `Clock` trait deleted; `now` is an argument.
18. `Fit = In | Out` → `bool`. The no-bool rule is about state fields, not predicate returns.
19. `Op::Reply`/`Forward`/`Send` → draft constructors + `SendState`; `OpKind` added for hover
    strips and keybindings. `Op::invert()` → `Applied { forward, inverse }`, because an inverse
    needs the prior state.
20. `Calendar` dropped (non-goal, and a bool in a costume). `Filter` predicates replaced state
    mirrors: `Snoozed`/`SnoozeDue`/`HasAttachment`/`Pinned`; FTS folded in as `Filter::Text`.
21. Seven crates → six: `mail-profiles` became `mail_domain::presets`, `mail-adapters` merged
    into `mail-proto`, `mail-mime` added so `mail-app` can sanitize without `mail-proto`.
22. `html_safe` no longer persisted; sanitize at render, cache by policy version.
23. `Tls` has no opportunistic StartTLS variant.
24. `mail-threading` dropped as a dependency; JWZ written in `mail-domain`.

**Resequenced:**

25. Phase 0 spike added, ahead of type design, producing both the facts and the trace fixtures.

**Found while writing the interface freeze** (`crates/mail-domain/src/`, `CONVENTIONS.md`,
`crates/mail-store/migrations/0001_initial.sql`) — these are corrections to *this* document,
caught because the freeze has to compile:

26. `Fetched.parsed: mail_mime::Parsed` was a **dependency cycle**: `Ingest` lives in
    `mail-domain`, and `mail-mime` depends on `mail-domain`. Parsing happens in `mail-proto`,
    so `Fetched` carries a finished domain `Message`.
27. `ThreadSummary::derive(id, messages)` could not produce `snooze` and `pin` — they are
    thread-level state the user set, and no message carries them. They are now arguments.
28. `Message` had **no `labels` field**, so `ThreadSummary.labels` had nothing to union over.
    Labels live on messages; the thread's set is the union.
29. `Change::ThreadLabel` became `Change::MessageLabel`. A change must name a message to be
    precisely invertible, since the thread's labels are derived.
30. Added a materialized `thread_summary` table. Every derived field of `ThreadSummary` was
    specified but had nowhere to live, and recomputing a list query by aggregating over
    messages per row does not survive a real mailbox. It is a cache and can be rebuilt.
