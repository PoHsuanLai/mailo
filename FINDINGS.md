# Open findings against the interface freeze

Agents report frozen-signature problems rather than working around them (CONVENTIONS.md §1).
This is the queue. Nothing here is applied while wave 1 is still running, because the files
involved are owned by agents in flight.

## Status

| # | Finding | Decision | State |
|---|---|---|---|
| F1 | `Filter::To` cannot work | Add `recipients` to `ThreadSummary` | **fixed** — `ThreadSummary.recipients`; `Filter::To` now answers |
| F2 | `Filter::Text` cannot mean substring | `Text` becomes word/phrase match; `fit` tokenizes to match FTS5 | **handed back** to the `fit` agent |
| F3 | Contradicting doc comments | `Filter::Text`'s scope wins; `MatchCtx` comment corrected | **handed back** with F2 |
| F4 | `cargo tree -i` exits 101 on absence | `scripts/check-boundary.sh` checks output, not exit status | **fixed** |
| F5 | `cargo fmt --all` clobbers in-flight files | Per-crate `fmt` during a wave | **fixed** in CONVENTIONS §10 and ORCHESTRATION |
| F6 | `threading::*` missing from `lib.rs` flat re-export | Re-export them | **fixed** |
| F7 | `ThreadInput` docs claim fields are pre-normalized | Soften the doc; impl normalizes defensively anyway | **fixed** |
| F8 | `Message` cannot express `Reply-To` | Add `reply_to: Vec<Address>` | **fixed** — `Message.reply_to`; `Draft::reply_to` honours it |
| F9 | `AuthPlan::OAuth.client_id` has no honest source | Remove from the persisted type; runtime maps issuer → client id | **fixed** — dropped from `AuthPlan::OAuth` |
| F10 | Three serde forms worth changing before first release | `o_auth` → `oauth`; `Synthetic` → hex string; keep `Duration` as-is | **fixed** — `oauth` tag, `Synthetic` as hex; `Duration` left as-is |
| F13 | `filter.rs` past the 400-line threshold | Lift the 250-line fold table to `src/filter/fold.rs` | **fixed** |
| F11 | `looks_like_ntu_id` byte-slices a char count | Correct today (ASCII predicate); add a guard comment | **fixed** — guard comment |
| **F12** | **`Op::apply` cannot build a `ProtoOp`** | **`Applied::remote` becomes `Option<RemoteIntent>`; `Store::enqueue` resolves it** | **fixed** — `RemoteIntent`, `Store::enqueue` resolves it |

A consequence of F2 worth writing down, because it will read as a bug: `Filter::Text` folds
diacritics (it goes through FTS5 `unicode61 remove_diacritics 2`), while `From`, `To` and
`Subject` do not (they go through SQL `LIKE`, which is ASCII-only without ICU). The asymmetry
is intended and follows from what SQLite can actually do.

## From the `fit` agent (wave 1, landed)

### F1 — `ThreadSummary` has no recipients, so `Filter::To` cannot work
`fit` implements `To` as always-false and documents it. Matching `To` against `participants`
(senders) answers a different question and produces false positives, so always-false is the
honest reading of the current type.

`to:` search is table stakes for a mail client. Fix: add `recipients: Vec<Address>` to
`ThreadSummary`, derived as the union over messages like `mailboxes`, plus a column on
`thread_summary`. Touches `message.rs` (owned by the `ops` agent right now), `0001_initial.sql`,
and the serde fixtures. **Apply after wave 1.**

Until then this is a wave-2 hazard: the SQL compiler must also compile `To` to false, or the
parity proptest fails — correctly.

### F2 — `Filter::Text` cannot mean substring
`fit`'s `TextMatch::Contains` is a raw substring test. `messages_fts` is FTS5 with `unicode61`,
which matches *tokens*; it cannot express `Contains("xamp")`. These will never agree, so the
parity proptest fails on any `Text` case with a partial-word needle.

The fix is to make the pure function model what the store can actually do, not the reverse:
`Filter::Text` means full-text **word/phrase** match, and `fit` tokenizes to match FTS5.
Decide before wave 2 starts, since both sides depend on it.

### F3 — `Filter::Text` and `MatchCtx` doc comments contradict each other
`Filter::Text` says "subject, participants, and the plain-text body"; `MatchCtx` says "subject,
sender and snippet". `fit` follows `MatchCtx` and does not consult `participants`. Decide which
is right and fix the other comment. One-line change either way.

### F4 — `cargo tree -p <crate> -i tokio` exits 101 when tokio is absent
Absence is the pass condition, but the exit status is failure, so the CI form must be
`! cargo tree -p mail-domain -i tokio 2>/dev/null | grep -q .` or equivalent. Update
CONVENTIONS.md §10 and ORCHESTRATION.md.

### F5 — `cargo fmt --all` is not safe to run mid-wave
It rewrites the whole workspace, including files other agents are editing. Agent briefs should
say `cargo fmt -p <crate>` during a wave, with `--all --check` only as the final gate.


## From the `threading` agent (wave 1, landed — verified)

11 integration + 15 lib tests pass. I checked the two claims that mattered: cycle refusal is
structural (`descends_from` walks the parent chain before every link, with a step cap as belt
and braces) and the thread-merge tie-break is `.min()` over the class, so it is a property of
the set rather than of arrival order. F6, F7 above.

The design decision worth recording: **grouping comes from a union-find partition, not from the
JWZ forest.** The forest legitimately splits one conversation in two — when a link is refused as
cyclic, and when a container is re-parented as better evidence arrives. The partition keeps them
together and makes the result independent of input order. The forest then only decides what a
thread's *root* is.

## From the `presets` agent (wave 1, landed — verified)

14 serde tests plus 28 frozen fixtures pass. The fixture generator refuses to overwrite, so the
corpus is append-only mechanically rather than by convention — better than what the brief asked
for. F8–F11 above.

**The campus host heuristic was a guess.** A local part shaped like a student id went to one
host and anything else to another; `spike/out/` was empty, so nothing had checked it. See F142:
the heuristic is gone, with the preset.

## Deferred deliberately

`Op::apply` and the `ops` agent's files are still in flight, so the F1/F8 `message.rs` edits and
the F9/F10 serde changes land together afterwards, as one coherent interface revision with one
fixture regeneration — rather than three separate edits racing a running agent.


## From the `ops` agent (wave 1, landed — verified)

13 tests pass including the required proptest, which it mutation-checked two ways (emit a label
change unconditionally; restore to `Inbox` instead of each message's own mailbox) — both caught
and shrunk. `remote: None` confirmed at the single construction site, `_caps` confirmed unused.

### F12 — the interface error this wave was meant to find

`Op::apply` cannot build a `ProtoOp`, for two independent reasons:

1. Every `ProtoOp` a user op could produce is addressed by `Vec<RemoteRef>`, and `RemoteRef` is
   not reachable from a `Message`. The mapping is `remote_map` in `mail-store` and `plan.md` is
   explicit that it is **many-to-one** — so it cannot even in principle be recomputed in the
   domain.
2. `ProtoOp::SetLabels` needs server-side label **names** (`Vec<String>`). `Op::Label` carries a
   `LabelId` and `mail-domain` has no label table to resolve it against.

The agent did not invent empty vectors. `Applied::remote` is `None` for every op, which means
**wave 4's runtime would have had nothing to enqueue** — found now rather than at integration.

That `caps` is a dead parameter is the cleanest evidence the signature is wrong: no capability
changes the *local* mutation, so `caps` was only ever going to discriminate the remote side.

**Decision — adopt the agent's fix.** `Applied::remote` becomes `Option<RemoteIntent>`, a domain
type addressed in local ids:

```rust
pub enum RemoteIntent {
    SetFlags   { messages: Vec<MessageId>, read: Option<ReadState>, star: Option<Star> },
    SetMailbox { messages: Vec<MessageId>, role: MailboxRole },
    SetLabels  { messages: Vec<MessageId>, add: Vec<LabelId>, remove: Vec<LabelId> },
}
```

`Store::enqueue` takes a `RemoteIntent` and resolves it to a `ProtoOp` — `remote_map` for the
refs, `labels` for the names — because that is the only place both tables exist. `ProtoOp` is
untouched, which matters: it is a persisted outbox schema.

This also **revives `caps`**: `apply` still decides whether remote work is *warranted*
(`LocalOnly` → `None`), which makes the two currently-vacuous `LocalOnly` tests meaningful.

Not really a choice — the only alternative is passing `remote_map` into `apply`, which breaks
the sans-I/O layering the whole design rests on.

### Judgment calls worth knowing

- **Empty `messages` panics.** A degenerate summary would have to invent a subject, sender, date
  and `AccountId`; a fabricated row in a list is worse than a loud failure at the call site.
  Programmer error, not malformed mail, so CONVENTIONS §5 permits it.
- **`forward` and `inverse` get different `ChangeId`s.** A `ChangeId` names one application
  *event*, not a reversible pair — applying the undo is a second event that must itself be
  recordable and re-layerable. Sharing one id would make "which patch is this row" ambiguous the
  moment an undo lands.
- **Snippet: 140 `char`s**, whitespace-collapsed, no ellipsis (the renderer's call), cut by chars
  so it cannot split a multi-byte character.
- **`Op::Restore` also lifts out of `Spam`** — there is no separate not-spam op. `Sent` and
  `Drafts` are left alone: they record where a message came from, not where the user filed it.
- **`Message::labels` is treated as a set**, so undo restores a label to the end rather than its
  original index. Nothing in `plan.md` suggests order is meaningful.


## Interface revision — applied

F1, F7, F8, F9, F10, F11, F12 and F13 landed together as one pass, with one fixture
regeneration, after wave 1 released every file. All gates green: `fmt --check`, `clippy -D
warnings`, `cargo test --workspace` (60 tests), `scripts/check-boundary.sh`, and the migration
applies to real SQLite.

**The frozen fixtures did their job.** Exactly two broke — `credentials.json` on the
`o_auth` → `oauth` rename, and `message_keys.json` on `Synthetic` becoming hex. Nothing else
did, because `reply_to` and `recipients` carry `#[serde(default)]` and the dropped `client_id`
is simply an ignored unknown field. Both were replaced rather than edited-to-pass: these are
deliberate pre-release schema changes with no stored data behind them. Had either surprised us,
the right response would have been to revert the type, not the fixture.

**Tests added for behaviour the revision unblocked**, since a type change nobody exercises is
not a fix: `server_capabilities_queue_remote_intent` (the positive half of the capability
contract, asserting the intent names exactly the changed messages),
`an_op_that_changes_nothing_queues_nothing`, `reply_goes_to_reply_to_when_the_sender_set_one`
(the mailing-list case), `derive_rolls_up_recipients_without_bcc`, `to_matches_thread_recipients`
and `remote_intent_round_trips`.

The two `LocalOnly` tests that the `ops` agent flagged as vacuous now discriminate for real.

## Still open

- ~~**The campus host heuristic.**~~ Closed by F142: no preset names an institution.
- ~~**F2's parity limit.**~~ Closed in phase 9.3. Measured first: 1017 BMP code points
  tokenized differently on the two sides — marks the domain kept inside a word and SQLite split
  on (Hebrew points, Arabic harakat, Indic signs), compatibility folds only SQLite made (`µ`,
  `ſ`, `ς`, `ϐ`), and letters newer than SQLite's tables. The store no longer has a tokenizer of
  its own: it indexes and queries with `search_tokens`, whose fold table is generated from SQLite,
  and `tests/fold_table.rs` checks all 1.1 million code points in two seconds.

## Phase 0 — the spike, run 2026-09-22

Run against a real Gmail account and a campus POP3 server. Transcripts are in `spike/out/`, which is
gitignored: they contain real subjects, addresses and a full message body.

### Confirmed — the claim the store is built on

**One message really does have different UIDs in different mailboxes.** Five for five:

```
X-GM-MSGID 1876965017123110734:  INBOX uid=32460   All Mail uid=62603   DIFFERENT
```

`remote_map` many-to-one, identity from `MessageKey`, is correct. Had this come back `SAME`,
the first Gmail sync would have duplicated every message in the mailbox.

`UIDVALIDITY` is 1 for INBOX and 12 for All Mail — per mailbox, not per account, as designed.

### F14 — capabilities must be read AFTER authentication

Gmail advertises a reduced `CAPABILITY` pre-auth: `XLIST`, and no `CONDSTORE`, `MOVE` or
`SPECIAL-USE`. The spike script asked before `LOGIN` and reported a far less capable server than
Gmail is — `SELECT` returned `* OK [HIGHESTMODSEQ 3737642]`, which only a CONDSTORE server sends,
and `LIST` returned `\All \Drafts \Sent \Junk \Trash \Flagged \Important`.

So: `AccountCaps` discovery belongs after the auth step, not before it. The script is fixed and
now reports both lists. **Nothing in `plan.md` said when caps are discovered; it should.**

### F15 — folder names are IMAP modified UTF-7

Real folders in the account include `&V4NXPpD1TvY-` and `&kc2JgZD1TvY-` (RFC 3501 §5.1.3). These
become `LabelOrigin::Provider` labels, so without a decoder the sidebar shows mojibake. Nothing
in the design mentions it. Encoding and decoding are both needed — a `SELECT` of a non-ASCII
folder has to re-encode the name.

### F16 — the campus preset guessed the wrong SASL mechanism

`CAPA` on the campus server: `SASL PLAIN`, `USER` — and **no `LOGIN`**, no `CRAM-MD5`, no `STLS`.
The preset offered `[Login, Plain]`, so the first live connect would have failed on a mechanism
the server does not implement. Corrected to `[Plain]`. No STLS is expected and fine: we connect
with implicit TLS on 995.

The host heuristic held for this account — the default host worked — but only one shape had
been tested, so the other host remained a guess.

### F17 — the campus mailbox is large (amended: it is mostly a hundred attachments)

`STAT` reports **2372 messages, 267,508,676 bytes** (~255 MB). POP3 offers no server-side search
and no partial body fetch, so a first sync means 2372 `RETR` round trips and a quarter of a
gigabyte over the wire. `plan.md` treats POP3 as the simple case; at this size the first sync is
a product problem — it needs to be resumable, and it should fetch newest-first so the inbox is
usable before it finishes. `TOP` (headers only) is worth checking for in `CAPA`.

UIDLs are 16 hex characters whose first 8 encode the arrival index (`0000000166aaf64b`,
`0000000266aaf64b`). They are opaque to us and stay opaque; noted only because the shape makes
them look sequential, and nothing should ever rely on that.


## POP3 research, 2026-09-22 — verified against our own transcript

Every number below was recomputed from `spike/out/pop3.trace`, not taken on trust.

### F17, amended — my spike script asked the wrong questions

`CAPA` on the campus server actually advertises:

```
CAPA TOP UIDL RESP-CODES PIPELINING AUTH-RESP-CODE USER SASL PLAIN
```

**`TOP` and `PIPELINING` are both there.** My script only grepped for auth mechanisms, so F17
recorded TOP as an open question when the transcript had already answered it. The script now
checks for the capabilities that decide sync strategy, which matter more than the auth list.
The server is Dovecot.

### F18 — the 255 MB is a hundred attachments, and `LIST` tells us for free

Recomputed from the transcript:

| | |
|---|---|
| messages ≤ 64 KiB | 2137 of 2372 (90.1%) — but only **10.0% of the bytes** (26.6 MB) |
| the 100 largest | **80.5% of the bytes** (215 MB) |
| median / p99 / max | 11 KB / 2.8 MB / 9.9 MB |

`LIST` hands us exact sizes before any `RETR`. **Size-aware ordering is a bigger lever than
`TOP`**: fetching the ≤64 KiB band newest-first completes in under a minute and covers every
message anyone is realistically going to open. The plan's "UIDL, diff, RETR what is new" is
right for steady state and wrong for the first sync.

### F19 — a naive first sync would mark the entire mailbox read, in their webmail

Dovecot gates its seen-flag update on the `RETR` path, so **`TOP` does not set `\Seen` but
`RETR` does**, and the campus server runs `pop3_no_flag_updates` at its default. A first sync that simply
`RETR`s 2372 messages would silently mark the user's whole mailbox as read in the campus webmail —
user-visible, not ours to undo, and discovered by reading Dovecot's source rather than any RFC.

A headers-first pass is therefore not only faster, it is the only non-destructive option.
`TOP n 0` specifically: every documented `TOP` bug found in the wild is a failure to return the
requested *body lines*, never wrong headers, so `TOP n 0` sits in the corner implementations get
right.

### F20 — UIDL instability is structural here, and the defence already exists

These UIDLs are Dovecot's default `%08Xu%08Xv`: eight hex of IMAP UID plus eight of
`UIDVALIDITY`. Every UIDL in the maildrop shares the suffix `66aaf64b`, so **one `UIDVALIDITY`
change invalidates all 2372 at once** — not hypothetical, Plesk 18.0.73 shipped a changed
`pop3_uidl_format` and caused exactly this. RFC 1939 also concedes UIDL uniqueness is not
guaranteed, so `(account, uidl)` is not a safe primary key.

Our design already defends this by deduplicating on `MessageKey`, and `SqliteStore::write_ingest`
already re-maps rather than refetching — checked, not assumed. Only `plan.md`'s wording said
"dropped and refetched"; corrected.

### F21 — the message model has no headers-only state

Headers-fetched-body-not-yet is a **normal, first-class state on POP3**, not an error. `Body`
currently requires `raw: BlobId`, so a headers-only message is unrepresentable. Per
`CONVENTIONS.md` §0 this wants an enum rather than an `Option` field:

```rust
pub enum Body {
    /// Headers only. Normal during a POP3 first sync; the body has not been fetched yet.
    Absent,
    Present { text: Option<String>, raw: BlobId },
}
```

Ripples into `messages.body_raw` (currently `NOT NULL`), `mail-mime`, and the fixtures.
**Decided, not yet applied** — batched with F14/F15 for the IMAP pass.

### F22 — `AccountCaps` has no POP3 side

It needs `top` and `pipelining`, read after auth per F14, and both latchable to false on
observed misbehaviour rather than only on absence from `CAPA`. A server that advertises `TOP`
and then truncates is a documented failure mode; advertisement is a hint, behaviour is the fact.

### Still unmeasured

The header-pass size estimate (~7–9 MB) rests on one measured message. A short `TOP n 0` run
over ~50 messages would turn that into a measurement and prove `TOP` works on this server in
practice rather than by advertisement.

## IMAP traps, 2026-09-22 — from the bug trackers, not the RFCs

Sourced to Mozilla/GitHub bug numbers and vendor KBs. The researcher corrected two of its own
earlier verdicts and withdrew a set of URLs it had cited from a *fork* rather than upstream; it
also refused to cite a cluster of repositories it could not establish as real projects. Both are
recorded because the discipline matters more than any single item.

### F23 — a capability check is necessary but not sufficient

Servers lie in both directions, and this is documented rather than folklore:

- **UW IMAP** advertises `UIDPLUS` and returns a bare `OK APPEND completed` with no `[APPENDUID]`.
  Thunderbird trusted the advertisement and **deleted the wrong draft** (Mozilla 400043).
- **Courier at GoDaddy** returns `[APPENDUID]` while *not* advertising `UIDPLUS` — the inverse
  lie. Thunderbird fell back to a header `SEARCH`, that server answered empty, and every save
  produced a duplicate draft (Mozilla 460085).
- **Oracle Messaging Server** advertises `IDLE` then answers `NO`; the client assumed the `+`
  continuation, sent `DONE` anyway, and wedged the session (Mozilla 344205, fixed only in TB 65).
- **Dovecot ≥1.2** sends a reduced capability list in the greeting and the full one in the tagged
  `OK` of `LOGIN` (Mozilla 401293) — the same shape as the Gmail pre-auth bug in F14, on another
  server entirely.
- **iCloud** runs behind a proxy that indiscriminately injects `XAPPLEPUSHSERVICE` into
  capability responses (isync `drv_imap.c`).

**Rule:** every capability-gated path needs a fallback triggered by *behaviour*, not by the
string. "APPEND returned OK without APPENDUID" must route to the fallback even though `UIDPLUS`
was advertised. This generalises F22 from POP3 to the whole design: advertisement is a hint,
behaviour is the fact, and `AccountCaps` must be latchable to false on observation.

### F24 — never adopt a server-volunteered UIDNEXT across an APPEND

After an `APPEND` a server volunteered `* OK [UIDNEXT 460]`; mbsync adopted it and fetched from
460, but the appended message had actually been given 458. Result: "lost track of 1 pushed
message" and **re-duplication on every later sync**. Snapshot `UIDNEXT` as it was *before* the
`APPEND` and fetch from there, with `UID FETCH n:*` rather than a magic upper bound — and note
`UID FETCH *:*` chokes DavMail/Exchange, where plain `UID FETCH *` is correct.

### F25 — CONDSTORE can be advertised and useless

Dovecot 2.0.18 returned `HIGHESTMODSEQ 1` forever while `EXISTS` climbed, because MODSEQ tracking
only began once a client explicitly asked for it. **Sanity-check that HIGHESTMODSEQ advances**;
treat frozen as "fall back to a full sync".

Conversely, an oversized `VANISHED (EARLIER)` is *legal* and must not be treated as a server bug:
given a mod-sequence below `<minmodseq>`, a server MUST report all expunged messages. This was
litigated on the Dovecot list and the reporter conceded.

Gmail advertises `CONDSTORE` but **not** `QRESYNC`, which matches our own spike.

### F26 — mUTF-7 decoding must never fail hard, and never case-fold

Two corrections to the F15 plan:

- **`UTF8=ACCEPT` does not mean the server stopped speaking mUTF-7.** Dovecot 2.4 with
  `mail_utf8_extensions=yes` still hands Outlook mUTF-7 folder names. The decoder is
  mode-dependent *and* defensive.
- **On decode failure, treat the name as literal bytes** — do not drop the connection, which is
  what go-imap did. A folder we cannot name is a cosmetic problem; a dropped connection is not.
- **Never case-fold mUTF-7**: its base64 alphabet is case-sensitive.

This strengthens the decision in F15 to write it ourselves. The one available crate panics on
malformed input; a decoder that cannot fail gracefully is worse than none, because the failure
mode is the whole account going offline.

### F27 — flags are not RFC-clean

A flag containing `]` violates RFC 3501, and **Gmail produces them anyway**. Python's `imaplib`
documents accepting them since 3.6 "since this improves real-world compatibility". Our atom/flag
parser must survive `]` inside a flag rather than treating the response as malformed.

### F28 — sequence numbers are a data-loss vector; we already avoid them

osTicket lost mail to exactly this (their own comment: the fetcher "uses message sequence
numbers", with a standing TODO to move to UIDs), and hMailServer numbered messages by position in
one shared per-folder container so an expunge by any session silently renumbered every other one.
`RemoteRef::Imap` is keyed on `uid` + `uidvalidity` and never on a sequence number, so this class
of bug is designed out. Recorded so it stays that way.


## Found while building, 2026-09-22

Each of these was caught by the compiler, by clippy, or by a test — not by review. That is the
argument for the enum-heavy design and the four gates, so they are recorded as evidence.

### F29 — `ProtoOp` could not ask for headers

Clippy noticed that `Pop3Backend`'s `Headers` state was never constructed. The cause was that
`ProtoOp` had `FetchBody` and nothing else, so the `TOP`-first strategy the research recommends
was **unexpressible**. Added `ProtoOp::FetchHeaders`. Where `TOP` is unavailable the backend now
refuses rather than falling back to `RETR`, because that fallback would silently mark the message
read — the exact harm the operation exists to avoid.

### F30 — a backend is one connection, not two

`plan.md` described `ImapBackend` as "IMAP session **and** SMTP submit". That would put two
sockets behind one `Machine` and make the drive loop's single-transport shape a lie. Submission
is now its own backend; the incoming backends refuse `ProtoOp::Submit` and the runtime routes it.

### F31 — `ProtoOutcome` could not return a fetched message

An `Ingest` carries fully-built `Message` values, and a protocol machine cannot build one: that
needs a `MessageId`, a `ThreadId` and a stored `BlobId`, none of which the wire supplies. Added
`ProtoOutcome::Fetched { remote, raw }`, which also keeps the blob store out of `mail-proto`
entirely.

### F32 — a session holds its commands, so a backend would hold the password

`Pop3Session::new` takes its command list at construction. A backend owning a session would
therefore have to own the credential to build each one. Backends take a factory closure instead,
which owns both the credential and the auth sequence; the backend never sees a password, and its
`Debug` is hand-written so the closure cannot put one in a log.

### F33 — POP3 capabilities also differ after authentication

RFC 2449 permits `CAPA` to answer differently in the AUTHORIZATION and TRANSACTION states — the
same trap F14 caught on Gmail's IMAP, on another protocol. `FetchCaps` now asks, authenticates,
and asks again, believing the second answer.

### F34 — modified UTF-7: a short run decoded to nothing

My own first implementation returned an empty string for a run too short to hold one UTF-16 unit
— exactly the `&A-` that panics the crate we rejected — so the mailbox name silently vanished
rather than falling back to literal text. Caught by the hostile-input test in the same commit.


### F35 — `View.group_by` could not express two obvious groupings

Reported by the session designing the shell, not found by a compiler: `group_by: Option<Property>`
could not say "group by read state" or "group by label", because `Property` is
`Date | Subject | From | Sender | Size | Attachments | Pin`.

Grouping and columns look like one vocabulary and are two. Nobody renders a column of "unread",
and nobody groups by "size". The field was typed as the thing we had rather than the thing we
needed — the same mistake `Op::Reply(Compose)` was, and fixed the same way: split the axis
rather than widen the enum that was already right for its own job.

`View.group_by` is now `Option<GroupKey>`, where `GroupKey` is
`Property(Property) | Read | Star | Label(LabelId) | Mailbox`. `Property` is unchanged and still
serves `shown` and `Sort`. `Label(LabelId)` also gives "is or is not tagged this way", which flat
labels could not express at all.

Adopted on the type argument rather than on the product citation that came with it: grouping a
mailbox by read state is obviously wanted whatever any particular client did, and the
citation's subject has since shut down.


### F36 — Drafts were accepted, reported applied, and dropped

`SqliteStore::write_change` matched `Change::DraftUpsert(_) | Change::DraftDelete(_) => None`.
`MemoryStore` inserted into a `BTreeMap`. Both satisfied `Store`; `apply` returned `Ok` in both
cases; the composer would have shown a saved draft and lost it on the next read.

The parity proptest — which exists precisely to catch two stores disagreeing — could not see it.
It compares answers, and there was no question to ask: the `Store` trait had no `draft` method.
An untestable divergence is not a divergence the test suite is weak on; it is one the *interface*
hides, and the fix is at the interface. `Store` now has `draft`, `drafts` and `set_send_state`.

`set_send_state` is deliberately not a `Change`. Everything in a `Patch` is invertible, because
the undo stack replays the inverse. "Sending" becoming "Sent" is not the user's edit and must not
be reversible: an undo that put a delivered message back into `Queued` would send it twice.

### F37 — `Bcc` would have been delivered to everyone

`mail_mime::build` wrote a `Bcc` header from `Draft.bcc`, and a passing test asserted it did.
That test was right for the caller it was written for — the copy saved to `Drafts`, which only
the sender reads, and which must keep the record of who was blind-copied. It was catastrophic
for the caller that did not exist yet: the bytes handed to SMTP.

Both halves fail silently, in opposite directions. Strip `Bcc` from the headers and forget the
envelope, and the blind recipient simply never receives the message — no bounce, no error, and
the sender sees a successful send. Leave it in the headers, and every ordinary recipient is told
in confidence exactly who was copied in confidence. Nothing in either case reports a problem.

The bug was reachable only because one function served two callers who want different bytes, and
the difference was left implicit. It is now `Disclosure::{Full, HideBlind}`, an argument neither
caller can supply by accident, and `posting()` returns the envelope and the message *together* —
`mail_from`, `rcpt_to` (To + Cc + Bcc, deduplicated) and bytes built with `HideBlind`. Deriving
the envelope by re-parsing the message is the exact shape of the first failure, so the type makes
it impossible to hold one without the other.

Found by reading the builder before wiring submission, not by a test.

### F38 — No account had an identity, so nothing could have been sent

`presets.rs` leaves `AccountPlan.identities` empty and explains why in a comment: an `Identity`
needs an `IdentityId` and an `AccountId`, and minting either inside `preset_for` would make a
pure lookup impure and invent an account id no row matches. The same comment says the
account-creation flow builds the default identity and pushes it on.

The account-creation flow did not. `account::add` wrote the account row, the capabilities row
and the keyring entry, and never touched `identities` — neither the table nor the plan's copy.
Every configured account therefore had nowhere to send from, which nothing noticed because
nothing could send at all.

The comment was not wrong when it was written; it described a contract that the other half was
expected to honour. A contract stated in prose on one side of a boundary and forgotten on the
other is how this happens. The nearest thing to a type-level fix would be for `Preset` to carry
something that *cannot* be stored without an identity, which is more machinery than one call site
justifies — so instead `account add` builds it, `tests/compose.rs` asserts a fresh account can
reply, and the prose now sits next to the code that keeps it.

No display name on the generated identity: deriving one from the local part produces "S1234567"
on the user's own outgoing mail, and a name the user did not choose is worse than none.

### F39 — Identities live in two places, and the foreign key picks the winner

`identities` is a table, and `AccountPlan.identities` is a JSON copy of the same list inside
`accounts.plan`. Both are written at account creation. Only one of them is enforced: `drafts.
identity` is a foreign key into the table, so a draft that exists at all has a row there, while
the plan's copy is a snapshot that nothing keeps current.

`compose::identity_of` first read the plan, which is the side that can go stale. It now reads the
table — the side the constraint guarantees — so the lookup cannot disagree with the row that
allowed the draft to be saved in the first place. `tests/compose.rs` empties the plan's copy and
asserts that replying and sending still work.

The duplication itself is still there and is the real defect; this is the safe reading of it, not
a fix. Collapsing it means deciding whether `AccountPlan` should carry identities at all, which
is a question for whenever a second identity per account becomes real.

### F40 — The sandboxed iframe had never been given anything to render

`ui.rs` called `reading(&message.body, None, policy)` — `None` for the HTML part, on every
message, with a comment saying the part was not stored separately yet. It is not stored
separately, and it does not need to be: it is inside the raw message, which is already in the
blob store because it is the only copy that is byte-for-byte what the server sent.

So every HTML message rendered as its plain-text alternative, and a message with no text
alternative — which is most marketing mail and a good deal of ordinary mail from phones —
rendered as nothing at all. The sandboxing, the sanitizer, the remote-image consent and the
comment warning never to reparent the iframe were all correct and all unreachable.

`reader::html_of` parses the raw bytes at render time and returns `None` for a missing blob,
unparseable bytes, or a plain-text message alike, because the reader's response to all three is
the same: show the text part. Rendering nothing for mail that every other client displays is
worse than rendering it roughly.

Still parsed per render rather than cached. Sanitized HTML must never be persisted — an
`ammonia` upgrade would leave every previously-ingested row sanitized under the old rules — and
persisting the unsanitized part would duplicate bytes the blob already holds. If a profile ever
says this is slow, the cache belongs in memory, keyed by message and policy version.

### F41 — Two tests that asserted nothing

Recorded because the habit matters more than the two tests. `the_reply_button_answers_the_newest
_message_in_the_thread` built its "thread" with `ThreadId::generate()` on the second message, so
the thread had one message and "reply to the newest" was trivially true. It passed on the first
run, which is what made it worth checking.

The store writes the `ThreadId` the caller hands it; JWZ threading happens in
`mail_runtime::assemble` on the way in. A fixture that mints a fresh id therefore produces a
one-message thread silently. The test now asserts the thread really has two messages before
asserting which one it picked — the assertion that the assertion is meaningful.

The same check, applied to the submission end-to-end tests, is what the previous commit's
"reintroduce the bug and watch it fail" pass was for. A test that has never failed has not been
shown to be a test.

### F42 — `cid:` survived the sanitizer and then resolved to nothing

The sanitizer allows `cid:` deliberately, with a comment explaining that it names this message's
own part and is safe in both image modes. Nothing downstream resolved it. Every inline image in
every HTML mail therefore rendered as a broken image — the sanitizer's care about the scheme
was entirely wasted, because the scheme never meant anything.

`plan.md` asks for "a custom protocol handler keyed on **`BlobId` only** — never a path", and
that cannot work in this reader. The iframe has `sandbox=""` with no `allow-same-origin`, which
gives the document an opaque origin; a custom scheme requested from an opaque origin is treated
as cross-origin and refused. Making it work would mean adding `allow-same-origin`, which hands
every future sanitizer bug direct access to the application's DOM — the single thing that iframe
exists to prevent.

So the bytes are embedded as `data:` URIs instead. This satisfies the plan's stated *reason* more
strongly than the handler would have: the plan forbids a path-shaped handler because it is "a
directory-traversal bug driven by untrusted mail", and this resolves nothing at request time —
no handler, no lookup, no filesystem, and a `cid` compared only against the parts of the message
that wrote it.

The security boundary moved rather than vanished, and it is now the media type. The type declared
in the message is attacker-controlled, and `data:text/html` in an `href` is script execution, so
the declared type decides only *whether* to embed; what is written into the document is the
matching entry from a four-item allowlist. `image/svg+xml` is not on it: an SVG is a document
that can carry script, and nothing here guarantees the reference came from an `<img>`, because
the sanitizer permits `cid:` wherever a URL is allowed.

Order matters and is asserted: sanitize, then resolve. Reversed, the sanitizer would be judging
a `data:` URI this code produced rather than the `cid:` the sender wrote.

A deviation from the plan, recorded as one.

### F144 — `ImgSrc` cannot carry a `BlobId`

The sketch of the block tree had `ImgSrc::Inline(BlobId)`. A `BlobId` names a blob in the store. `mail-mime` is pure, and `scripts/check-boundary.sh` forbids it from reaching `mail-store`, so the parser cannot build that variant: the part it is holding is bytes, not a stored name. F42 already closed the path that would have used the id. A custom scheme handler cannot be fetched from the sandboxed frame's opaque origin, and a path-shaped handler is directory traversal driven by untrusted mail. The id was the key for that handler. With the handler gone, the variant had nothing to point at.

What the parser builds is `ImgSrc::Inline(Inlined)`, a `data:image/...;base64,...` URI constructed only from a part whose declared type `embeddable` accepts, spelled with the allowlist's media type and not the message's. `ImgSrc::Remote` is a `SafeUrl` the reader has allowed. `ImgSrc::Blocked { host }` keeps the host and drops the URL, so a placeholder can name who would be told the mail was opened without the document holding a fetchable address.

`Document` still has no serde derive. A serde form is a persisted schema, and render output must not be persisted: a cap or a mapping change would otherwise freeze yesterday's blocks in the database. Re-parse the message.

### F43 — A comment that asserted the bug could not happen

`Shell::close_composer` dropped the composer's widgets, and its doc comment said discarding was
safe "because every edit the composer makes is saved to the store before it can be lost". That
was true of no code. The Close button called it directly, so everything typed since the last
explicit Save was gone — a paragraph of writing, lost to the button whose label most implies
safety.

I wrote both the comment and the bug, in the same commit, and the comment is what made it hard
to see: it reads like an invariant being documented rather than one being assumed.

Close now saves and then closes, and refuses to close if the save fails — a recipient that does
not parse must not cost the user the paragraph. Discard is a separate button for deliberate
abandonment, and `close_composer`'s comment now says plainly that it loses unsaved work and names
the one caller allowed to reach it without saving.

The lesson is narrower than "comments lie". It is that a comment explaining why a hazard is safe
should name the code that makes it safe, so that the claim can be checked — and if it cannot name
one, the hazard is real.

### F44 — The IMAP sync walk threw away everything it learned

`ImapBackend`'s envelope job returned `empty_ingest` — no messages, no flags, and a cursor of
`uidvalidity: 0, uidnext: 0` — with a comment explaining that turning responses into `Message`s
needs ids and blob storage from above this crate.

That reason is true and it is about *bodies*. It is not a reason to discard the UIDs, the sizes,
the flags or the `UIDVALIDITY`, none of which need any of those things. So an IMAP sync
authenticated, selected the mailbox, fetched every envelope — and stored nothing, reported
`headers_fetched: 0`, and returned `Ok`. A silent, complete no-op that looked like a clean sync
of an empty mailbox.

The transcript tests could not see it: they assert which commands the backend emits, and the
commands were right. What was wrong was what it did with the answers, which only a server
answering can show.

The walk now fills the `Ingest`'s cursor and flags from the untagged responses and records the
survey, which `Backend::surveyed` hands the runtime — the same seam POP3 already used, and for
the same reason: on a first sync the store knows nothing, so "what should I fetch" cannot be
answered by asking the store.

`RFC822.SIZE` was added to the fetch items in the same change. The runtime fetches bodies
smallest band first, and with no size every message fell in the first band, which quietly turned
the band ordering back into arrival order.

### F45 — `ImapBackend` named its own authentication, eleven times

`SessionFactory` took only the commands, and the backend prepended `ImapCommand::
AuthenticateXoauth2` at every call site. `ImapSession` has supported `LOGIN` with a password all
along, and `ImapAuth` carries a `Credential` that may be either — but nothing could reach that
path, because the backend had already chosen.

Every IMAP server except Gmail is password IMAP, so the effect was that the entire IMAP path
required an OAuth client registration to use at all. `mail-app` said so out loud — "password
IMAP is not wired up" — and that read like a missing feature in the app rather than a decision
frozen three layers down.

POP3 had the shape right from the start: the factory takes `Authenticate::{First, No}` and owns
the mechanism, "because it is the only thing holding the credential and the only thing that knows
the mechanism". `Authenticate` now lives in `backend/mod.rs` and both protocols share it — one
question, asked twice.

### F46 — The two sync branches had drifted, and one of them was half a pass

`mail-app`'s `sync::one` matched on the incoming protocol and then wrote the pass out twice. The
POP3 arm fetched envelopes, then headers, then bodies smallest band first, then drained the
outbox. The IMAP arm fetched envelopes and returned.

So an IMAP account never downloaded a message body and never sent anything it had queued — and
reported success, because the part it did do succeeded. The two arms were written at different
times for different reasons and nothing compared them; they are three screens apart in one file,
which is exactly far enough not to notice.

There is now one `pass` function, generic over `Backend`, and both arms call it. Phase 5 asks
that "the **same** CLI works through the IMAP backend", and the surest way to make two paths the
same is for there to be one.

The sync line also reports what it sent, which it never had, because until recently there was
nothing that could send.

### F47 — Nothing could configure a server the preset table did not know

`account add` refused any domain without a preset: "Manual setup is not written yet". Combined
with F45 — the IMAP backend naming XOAUTH2 for itself — this meant the only reachable IMAP
account in the entire program was Gmail, and Gmail needs an OAuth client id that cannot live in a
source tree. Fixing F45 made password IMAP possible; without this it was still unreachable.

`presets::manual` builds a plan from a hostname and nothing else, which means assuming things no
spike has measured. Everything it assumes is chosen to be safe when wrong: implicit TLS on 993
and 465 rather than STARTTLS, because an opportunistic upgrade is strippable and a server that
does not offer implicit TLS refuses the connection instead of quietly falling back to cleartext;
capabilities at their least capable, so the first real connection can only add to them; and
`ArchiveMeans::LocalOnly`, because archiving into a folder we have not confirmed exists loses the
message.

Both servers or neither. Naming only `--imap` is refused rather than completed with a guessed
`smtp.` hostname, which is how mail leaves through a server the user never chose.

### F48 — Eleven hooks called from event handlers, and one honest correction

`use_context` is `use_hook(|| consume_context())` — a hook, bound by the rules of hooks.
`consume_context` is the same lookup without the hook, and its documentation says so explicitly:
"can be called from anywhere the Dioxus runtime is active — inside event handlers, async tasks,
spawned futures, or other non-hook contexts". `ui.rs` called `use_context` at eleven sites, nine
of them inside event handlers, memo closures or a spawned future. Those nine are now
`consume_context`; the two at component tops, which are in the render path, stay as they were.

The correction is to the other half of the same commit. `Composer` returned early, above both of
its hooks, and I described that as a live bug. It is a broken rule and it is now fixed — but I
wrote the test that would catch it, put the early return back, and watched the test pass.
`use_hook` indexes from zero on every render, and since the early return preceded *every* hook in
that function, there was no later hook left to misalign. The rule was broken; nothing downstream
of it was.

The fix stays, because "harmless given the current body" stops being true the moment someone adds
a third hook below. The test stays too, renamed to claim only what it demonstrates. What does not
stay is the assertion that it guarded a crash.

This is the second time in this session that writing the failing case changed what I believed
(the first was F41). The rule that produced both: a test that has never failed has not been shown
to be a test, and a comment claiming a hazard is handled should name the thing that handles it.

### F49 — The components had never been executed

Every test of the shell stopped at `view.rs` and `reader.rs`, which are free of Dioxus on
purpose. Nothing had ever run `App` or `Composer` — so a panic inside `rsx!`, a context that was
not provided, or a store query that failed on a real database would all have waited for the first
person to open the window.

`VirtualDom::new(App).with_root_context(store)` runs them with no window at all. Four tests now
render the app against a seeded database, re-render it, and open and close the composer.

They need a tokio reactor, which is itself worth knowing: the composer's autosave is a
`tokio::time::sleep` inside `use_future`, so any render that polls tasks depends on one being
present. `dioxus-desktop` supplies it at runtime; the tests say `#[tokio::test]` for the same
reason.

The window was also launched for real against a scratch database, twice, and stayed up.

### F50 — Every header fetch destroyed the cursor the survey had just written

`Ingest.cursor` was a plain `SyncCursor`, so every ingest had to state one whether or not it knew
anything. The runtime passed `SyncCursor::Pop` for header and body batches — on both protocols,
because there was nothing else to pass — and `write_ingest` wrote it to `sync_state`
unconditionally.

A sync pass surveys, which records the real `UIDVALIDITY`, `UIDNEXT` and `HIGHESTMODSEQ`, and
then fetches headers, which overwrote all three with `Pop` a few milliseconds later. So an IMAP
account resurveyed its entire mailbox on every pass, for ever, and could never resume — and the
CONDSTORE path could never start, because the modseq it needed had been thrown away before
anything read it.

The type was the defect. `Option<SyncCursor>` says the thing that is actually true: `None` is not
a missing value, it is the claim *this batch learned nothing about the mailbox's position*. A
header fetch was handed a list and collected it; it never asked the server what exists. Only a
survey can answer that, so only a survey writes it.

Widening to `Option` is backward-compatible for the frozen corpus, since `Option<T>` accepts
`T`'s own representation — checked rather than assumed.

### F51 — The CONDSTORE chain had three links and two were missing

`ImapBackend` has emitted `(UID FLAGS) (CHANGEDSINCE n)` since it was written, gated on a modseq
and the account's capabilities. `AccountEngine::trusted_modseq` returned `None` unconditionally,
behind a comment saying the IMAP backend "is not written yet" — true when written, stale for
several commits. And `Store` had no way to read a cursor back at all: `sync_state` had been
written by every ingest since the store was created and read by nothing.

So the flags sweep refetched every flag on every poll, on every server, regardless of what it
supported. Correct and slow, which is the right way round to be wrong — but not what the plan
asks for, and not what the backend was already built to do.

`Store::cursor` reads it, `mailbox_state` parses `HIGHESTMODSEQ`, and `trusted_modseq` gates on
CONDSTORE *and* a non-zero stored modseq. It deliberately does not add its own "did it advance"
check: Dovecot 2.0.18 froze `HIGHESTMODSEQ` at 1 while `EXISTS` climbed, and the defence against
that belongs where it already is — withdrawing the capability — not in a second, quieter rule
that would let the two disagree.

### F52 — `remote_map` grew a row per message per sync, on every protocol

`remote_map`'s primary key is `(account, mailbox, uidvalidity, uid, uidl)`, and its own `CHECK`
constraint guarantees that exactly one of `uid`/`uidl` is `NULL` — so **every row has a NULL in
its key**. SQLite treats NULLs as distinct in `UNIQUE` and `PRIMARY KEY` comparisons, so no two
rows ever conflicted, `INSERT OR REPLACE` had nothing to replace, and the table grew without
bound: one row per message per pass, for ever.

The consequences are worse than size. `refs_for` returns every address of a message, and it was
returning the same UID three, ten, a hundred times — so a single "mark read" would send that UID
to the server once per sync that had ever run.

POP3 had the identical hole, with *two* NULLs in the key. It never showed because the POP3
end-to-end test syncs once. Nothing in the suite had ever synced the same mailbox twice, which is
the only thing that makes this visible and is what a real client does every five minutes.

Migration 0002 collapses what has accumulated and adds a unique index over
`COALESCE(uidvalidity, -1), COALESCE(uid, -1), COALESCE(uidl, '')`. The sentinels are outside the
domain of real values — a UID is a positive integer, a UIDL a non-empty string — so they cannot
collide with one. The insert names that index in an `ON CONFLICT` clause.

Found by an assertion about row counts that I expected to be trivially true.

### F53 — UIDVALIDITY reset could never fire

`plan.md` phase 5 names "UIDVALIDITY reset handling". The store has implemented it since it was
written: `UidValidity::Reset` drops every `remote_map` row for the mailbox. The backend reported
`UidValidity::Same` unconditionally — I wrote that line — so the branch was unreachable.

A server that restores a mailbox from backup, migrates it, or has a folder deleted and remade
with the same name must change `UIDVALIDITY`. Every UID held then names a different message or
none, and keeping them is not a stale cache: it is marking the wrong mail read and attaching
bodies to the wrong headers.

The backend cannot decide this — it is sans-I/O and has never seen what was stored. It reports
what the server said; `UidValidity::between` compares that against the stored cursor in the
runtime. Conservative in three places, because a reset refetches the whole mailbox: no stored
cursor is a first sync, a zero on either side means the server did not say, and a POP cursor has
no `UIDVALIDITY` at all.

The same commit fixed a related mistake of mine: survey references carried `uidvalidity: 0` with
a comment claiming it was filled in later. It was not. `remote_map` keys on that column, so every
IMAP row was being written under a mailbox generation that never existed.

### F54 — The expunge sweep asked the right question and threw the answer away

`Job::Listing` shared an arm with `Job::Envelopes` and `Job::Flags`, all three running the same
`parse_fetches`. Envelopes and flags arrive as `* n FETCH (...)`; a listing is `UID SEARCH ALL`,
which answers `* SEARCH 101 102`. So the parser matched nothing, `gone` came back empty, and
every expunge sweep concluded that nothing had disappeared.

The third of the three sync intervals therefore ran on schedule, opened a connection, asked the
server for the full list, and learned nothing from it — for ever. A message deleted on a phone
stayed in the client until the database was thrown away. Gmail offers no QRESYNC and IDLE reports
new mail only, so this listing is the *only* mechanism there is; RFC 7162 says so outright, and
the backend's own comment quotes it.

Splitting the arm is half the fix. The other half is that the backend can only report what still
exists — what we *hold* is the store's knowledge — so `Store::remote_refs` says what is mapped in
a mailbox and the runtime diffs the two. That diff compares addresses rather than messages: a
message still present under another mailbox's UID keeps that row, and only the mapping in this
mailbox goes.

An empty `* SEARCH` is allowed to expunge everything, because an emptied mailbox is a real thing
a server says. That is the one case worth being deliberate about — the opposite reading deletes a
user's mail whenever a response fails to parse — so there is a test asserting a sweep that finds
everything still present deletes nothing.

### F55 — Archive-by-move checked for MOVE and then did nothing with it

`ArchiveMeans::MoveToFolder` issued `UID COPY` and stopped. Below it sat an `if` testing
`ExpungeMeans::Allowed` and `MoveExt::Supported` whose body was a comment and nothing else — the
capability was read, the branch was taken, and no command came of it.

So archiving on a non-Gmail server copied the message into `Archive` and left the original in the
inbox. The comment called a stray original "a cosmetic problem rather than lost mail", and the
first half of that is wrong: the next survey reports the message as still in the inbox, server
truth wins once the outbox has settled and the pending row is dropped, and the user's archive
quietly comes undone. It is the same shape as a star flipping back, with nothing left to protect
it.

`UID MOVE` (RFC 6851) is the fix, and it is worth being precise about why it is allowed here when
`ProtoOp::Expunge` is refused outright. The prohibition is on `\Deleted` + `EXPUNGE`, because
Gmail routes expunging through a per-account setting that may be `deleteForever` and cannot be
read over IMAP — so a client completing a move that way can permanently destroy mail on an
account whose owner never agreed to it. `MOVE` is atomic, names no flag, and is not that dance in
disguise; it is the primitive the dance was always a poor imitation of.

Where the server does not advertise `MOVE`, it stays `COPY` and stop. That is still the right
trade, but the comment now says what it costs instead of calling it cosmetic.

The prohibition itself is now a test rather than a convention: a full working session — sync,
archive with MOVE, archive without, flags, sweep — is asserted to contain no `\Deleted` and no
`EXPUNGE` anywhere in what was sent.

### F56 — Every account ran for ever on a guess about its server

`ProtoOp::FetchCaps` was implemented, careful, and never sent. `ProtoOp::ListFolders` likewise.
`ProtoOutcome::Caps` was constructed by the backend and matched by nothing above it. And
`account_caps` had exactly one writer — `account add`, storing `preset.expected_caps` — and no
updater.

So what the client believed about a server was whatever a preset guessed before it had ever
connected, permanently. Three fully-implemented features were unreachable at once because of it:

- **CONDSTORE** could never be found, so the flags sweep refetched every flag for ever (F51 fixed
  the chain; this is why the chain would still never have started).
- **MOVE** could never be found, so archive-by-move always took the `COPY`-and-stop branch (F55).
- **`SPECIAL-USE` folder roles** stayed empty, so `FolderRoles` was an empty list for every
  account and filing targeted a path nobody had confirmed exists.

The design was right and the wire was missing. The backend's own comment says it: "Gmail's
pre-auth list omits CONDSTORE, MOVE and SPECIAL-USE, so believing the first answer reports a far
less capable server than it is — F14, measured against a real account." F14 was found by
measuring a real account, and then nothing ever asked.

`AccountEngine::refresh_caps` runs both walks and persists; `sync::pass` calls it when the stored
answer is more than a day old. Daily because a server gains and loses extensions across upgrades
and an account moved between providers keeps its row — often enough to notice, rare enough to
cost nothing.

This is the fourth instance this session of the same shape: a subsystem built correctly, tested
at its own boundary, and never called by anything (F44, F51, F55, F56). The unit tests all
passed, because each one was asking the component whether it worked rather than whether anything
used it.

### F57 — An audit for uncalled subsystems, and what it found

F44, F51, F55 and F56 were all the same shape: a subsystem built correctly, tested at its own
boundary, and called by nothing. Four instances found one per round, each by accident. That is a
mechanically checkable property, so rather than wait for a fifth, every `ProtoOp` and
`ProtoOutcome` was swept for callers above `mail-proto`.

The result:

| variant | callers | verdict |
|---|---|---|
| `Expunge` | 0 | correct — refused by design, never sent |
| `Append` | 0 | a real gap: a draft is never uploaded to the server's Drafts folder, so it exists on one machine only |
| `Watch` / `Woken` | 0 | **IDLE was never used on any server that offered it** |

Everything else had callers. `Applied` and `Submitted` have no dedicated handler, which is fine:
the outbox drain treats any success as success.

### F58 — `IDLE` was documented to wake on news and only ever woke on interrupt

`ImapCommand::Idle`'s own doc says it "parks until the server says something or the caller
interrupts". Only the second half was implemented. `IoReady::Interrupt` moved the session to
`IdleEnding` and sent `DONE`; an untagged `* 3 EXISTS` announcing new mail was appended to the
transcript and the session went on parking.

So IDLE was a sleep with extra steps, and on a server that offers it — which is Gmail, and most
of the rest — new mail would have waited for the next poll anyway. The feature the `WatchMode`
enum exists to distinguish did not distinguish anything.

The session now ends IDLE in protocol when an untagged response is news: `EXISTS` and `RECENT`
are new mail, and `EXPUNGE` and `FETCH` are a change made elsewhere, which a watcher wants just
as much — a message read on a phone should not wait for a poll either. `DONE` rather than
dropping the socket, so the connection stays reusable.

`AccountEngine::watch` is the caller the sweep said was missing. It returns whether the server
actually signalled, so `WatchMode::Poll` reports "do not wait on me" rather than imitating a
watch by sleeping — the absence of push is not a slower push, and hiding that from the scheduler
would be the same mistake in a different place.

The regression test is bounded at five seconds. Removing the news check makes the session park
for ever, and a test that hangs blocks a run instead of reporting one.

### F59 — `Append` was refused alongside `Submit`, and drafts never left the machine

The IMAP backend matched `ProtoOp::Append { .. } | ProtoOp::Submit { .. }` together and answered
"submission is a separate backend". True of `Submit`, which goes to an entirely different server
over SMTP. False of `Append`, which uploads a message into a folder over *this* connection and is
as much an IMAP operation as `FETCH`.

So a draft composed here existed on one machine. Starting a reply at a desk and finishing it on a
phone — which is most of what a Drafts folder is for — could not work.

`APPEND` is the only command here that sends a literal, which is why it needed a phase of its
own. The server answers `+` and only then may the bytes go; writing them ahead of the
continuation means a server that rejected the `APPEND` line is now reading a message as though
it were commands. A tagged reply arriving *instead* of the `+` is the ordinary case of no such
mailbox or over quota, so `AppendPending` joins the phases that can receive one — without that it
would have read as a desynchronised connection rather than as the server saying no.

The Drafts path comes from the server's own `SPECIAL-USE` reply. An account whose capabilities
name no Drafts folder gets `Ok(false)` and keeps the draft local: uploading into a path nobody
confirmed is how a message lands somewhere the user will never look.

### F60 — The destructive-command assertion could never fire

`nothing_this_client_sends_can_destroy_mail` asserted `!upper.contains("\\\\DELETED")` — two
literal backslashes, which no IMAP command contains. It had been vacuous since F55 added it, and
it was the test standing guard over the one rule in `CONVENTIONS.md` whose violation destroys
mail permanently.

Found by writing a different test with the same mistake: the `APPEND` flags assertion failed
against a command that was correct, which sent me looking for the other place I had over-escaped.

Fixed, and then proved: injecting a `+FLAGS (\Deleted)` into the archive path now fails the test
with the offending command quoted. This is the fourth time in this session that an assertion
turned out not to assert what it claimed (F41, F48, and the POP3 repeat-pass comment). Three of
the four were found by deliberately reintroducing the bug. The fourth was found by accident,
which is the argument for doing it deliberately every time.

### F61 — A sweep for assertions that cannot fail

F60 was found by accident: an over-escaped string made a *different* test fail against a correct
command, which sent me looking for the same mistake elsewhere. Accident is not a method, and the
property is checkable, so every assertion in the workspace was swept for the ways one can be
vacuous.

**Over-escaped literals** (four backslashes in source, two in the string): none remaining beyond
F60's.

**Vacuous iteration** — `for token in case.drop { assert!(…) }` asserts nothing when the list is
empty. The sanitizer's 26-case table has five cases with an empty `drop` and two with an empty
`keep`, but none with both, so every case still asserts something. Sound, and now known to be
rather than assumed.

**Tautological disjunctions**, which is where the two real findings were:

- `assemble.rs` claimed to check that an unparseable `Date` falls back to the server's, and
  asserted `all(|t| t.last_date == now() || t.last_date.timestamp() > 0)`. The second half is
  true of every date after 1970, so it passed whatever the fallback did — including not falling
  back at all. It now finds the one message with a bad date by subject and asserts its date
  equals the server's exactly. Breaking the fallback now fails it; before, it did not.

- `drive.rs` asserted a closed peer reports `contains("closed") || contains("io")`. `"io"` is a
  substring of `"connection"`, so any error mentioning a connection passed — which is most of
  them. It now names the message. Changing `UnexpectedEof`'s text to "io failure on the
  connection" fails the new assertion and would have passed the old one.

The pattern in both: a disjunction added to make a test tolerant of an answer the author was not
sure of. Tolerance is the right instinct and a substring is the wrong implement — it widens the
assertion to things that share three letters rather than to the alternatives actually meant.

### F62 — The wire parsers had never seen input nobody chose

I said the remaining defect classes needed "a server I didn't write", and that was half wrong. A
*malicious or broken* server is simulable, and the code that meets one first is the parsers —
which had no property-based testing at all. `proptest` was a dependency of `mail-domain` and
`mail-store`; `mail-proto`, the only crate that reads bytes off a socket, had none.

Every other test in that crate replays a transcript someone chose, which answers "does this work
against a server behaving as expected". It cannot answer what a deployment asks immediately: what
happens when the bytes are wrong — a middlebox, an old server, a TLS error page delivered on port
143, or an attacker.

Two properties, which are the ones that make a parser safe to point at the internet:

1. **No input panics.** A panic in a mail client is a crash on *receiving mail*, and the sender
   chooses when.
2. **Every input terminates.** A machine that neither finishes nor asks for more has hung the
   connection, and a hang is harder to diagnose than a crash because nothing is reported.

Neither asserts the parse is correct — that is what the transcript tests are for. They assert the
failure mode is a clean error rather than a crashed or wedged client.

The generator mixes uniform random bytes with real protocol fragments, because uniform bytes are
rejected at the first byte of almost every parser and test the rejection and nothing past it. The
fragments — an unterminated literal, `{-1}`, a bare continuation, a literal announcing 999999
bytes — get past the front door, which is where there is state to corrupt.

**Nothing failed.** The parsers are sound under this, which is a result rather than a
disappointment, and I checked the properties can fail before believing them: a panic injected into
`mutf7::decode` fails the totality property, and turning `IoReady::Eof` into "ask for more bytes"
fails all three termination properties with "the session neither finished nor failed".

### F63 — I advised a setup that cannot work, against my own plan

`plan.md` has said since it was written that Exchange Online needs OAuth with SASL XOAUTH2. Two
rounds ago I suggested, in as many words:

```
mailo account add you@work.example --imap outlook.office365.com --smtp smtp.office365.com
```

with `MAILO_PASSWORD` set. Microsoft switched off Basic Authentication for IMAP, POP and SMTP on
Exchange Online, so that cannot authenticate — and the user has two managed Microsoft 365
mailboxes, which is exactly who that advice was for.

The failure mode is the reason this is worth code rather than a correction. The password is
accepted by the keyring, stored, and rejected at the first sync by a server that says nothing
about why. Everything up to the socket looks right, so the natural reading is that the client is
broken.

`presets::password_warning` says it at `account add`. Advice rather than a refusal: a tenant may
have re-enabled something, and the user knows their own account better than a table does. Google
gets different wording because the outcome differs — an App Password still works there, so
"use OAuth" would send someone to build something they do not need.

Matched on domain boundaries, not `ends_with`: `evil-office365.com` ends with `office365.com`.
Here that would only produce a misleading warning, but it is the same mistake that trusts the
wrong host when it appears in a security decision, and it is not worth writing the weaker version
even once.

### F64 — `OAuthIssuer::Microsoft`, and two things the plan had wrong

The plan predicted that adding a provider would cost "an `OAuthIssuer` variant, a preset row, and
nothing else", and called it "the first outside test of the claim that a provider is a value
rather than a type". The claim held: adding the variant produced exactly one compile error, in
the endpoints table, and `Incoming::Imap`, `Outgoing::Smtp` and `ImapBackend` were untouched.

Writing it corrected the plan twice.

**A custom tenant domain cannot be recognised from an address.** The draft assumed a preset row
keyed on domain, as Gmail and the campus server were. But a work mailbox is `you@yourcompany.com`, and nothing
in that string says Microsoft — only the `*.onmicrosoft.com` fallback names itself. My first
attempt matched `office365.com`, which is not a domain anyone receives mail at. The real options
are autodiscover or asking, and autodiscover points the client at a host the user never named, so
it is `--microsoft`.

**Submission is STARTTLS on 587.** Exchange Online does not offer implicit TLS on 465 for SMTP
AUTH, which is the single place its shape differs from Gmail's — and the one detail that would
have failed on first connection with everything else correct. `StartTlsRequired`, never
opportunistic: a failure to upgrade aborts rather than sending a bearer token in cleartext.

Folder roles are left empty rather than guessed. Exchange Online localises them per mailbox, so
"Sent Items" is a guess that files mail into a folder that may not exist; `refresh_caps` (F56)
fills them from `LIST (SPECIAL-USE)` on the first connection.

### F65 — Advice that does not survive being followed

`account add` told the user to re-run with a client id and printed the command to use. For a
Microsoft account on a custom domain it dropped `--microsoft`, which is the one piece of
information the preset table does not have — so following the instruction verbatim would fail to
find any preset at all.

Small, and the same shape as F63 one round earlier: guidance produced by the tool, never executed
by anyone. There is now a test that parses the suggested command back and asserts it reproduces
the account it describes.

### F66 — A refusal the user cannot act on is a refusal that wastes their afternoon

The excuse I gave last round was that tenant policy is "a fact about your organisation, not the
protocol". True, and it does not follow that nothing can be done. When a tenant forbids IMAP or
SMTP AUTH, the server says so in a documented, versioned code — and the client was relaying that
verbatim, where it is indistinguishable from "wrong password".

That distinction is the whole value. `535 5.7.139` means an administrator has switched SMTP
client authentication off for the tenant: the credential is correct, and no amount of retyping
it, re-adding the account or registering a new OAuth application will change anything.
`534-5.7.9` means the account has two-factor authentication and needs an App Password, which is
five minutes of work — but reads as "your password is wrong", which sends someone to reset a
password that was right.

`diagnose::explain` maps only codes the provider publishes. Nothing here matches on prose a
server might reasonably reword, because a confident wrong explanation is worse than the raw text
it replaced, and the unrecognised case — which is most of them — shows the server's own words
unchanged.

It is advice and never a decision: `Retryable` still owns whether anything is retried. A
diagnosis that quietly altered retry behaviour would be a second policy disagreeing with the
first, and the disagreement would surface as mail that does not send for reasons neither policy
states.

### F67 — The same substring mistake, a third time

`explain_text` matched `5.7.139` with `contains`, so `5.7.1399` was diagnosed as a tenant with
SMTP AUTH disabled. Caught by this file's own "invents no explanation" case, which is why that
test was written before the implementation was trusted.

Three appearances now, in three disguises:

- `ends_with("office365.com")` treating `evil-office365.com` as Microsoft (F63)
- `contains("io")` matching every error mentioning a `connection` (F61)
- `contains("5.7.139")` matching `5.7.1399` (here)

Each time the intent was a *token* — a domain, a word, a status code — and the implement was a
substring. The fix is always a boundary, and the general lesson is that `contains` is the wrong
default for anything with a grammar. Worth a convention if it appears a fourth time.

### F68 — I could have connected to a real server all along

Every round I said the remaining defects needed "a server I didn't write", and never checked
whether I could reach one. I can: an unauthenticated capability probe needs no credential, and it
is exactly what `plan.md`'s phase 0 specifies.

The campus server on 995 answers `+OK Dovecot ready.` and advertises `TOP UIDL RESP-CODES PIPELINING
AUTH-RESP-CODE USER SASL PLAIN` — confirming the campus preset's `top: Supported::Yes`,
`pipelining: Supported::Yes` and `sasl: [Plain]` exactly, including its comment that the server
"does NOT offer LOGIN or CRAM-MD5". That comment was written from a spike the user ran; this is
the first time this session verified a preset against the thing it describes.

`outlook.office365.com:993` answers `AUTH=XOAUTH2 LOGINDISABLED SASL-IR UIDPLUS MOVE ID UNSELECT
CHILDREN IDLE NAMESPACE LITERAL+`, which checks the Microsoft preset written one round earlier
from documentation: `WatchMode::Idle` correct, `Condstore::Absent` correct, OAuth-only confirmed.
It also advertises `MOVE`, which the preset pessimistically calls absent — correct by design,
since `refresh_caps` replaces a guess with what the server says.

The limit was never reaching a server. It was authenticating to one, and I had generalised the
second into the first for fifteen rounds.

### F69 — `LOGINDISABLED` was ignored, so we would send a password to a server that had refused it

RFC 3501 §6.2.3: a client MUST NOT issue `LOGIN` when the server advertises `LOGINDISABLED`.
Nothing here looked at it. Exchange Online advertises it, so every Microsoft 365 account is on
a server where the client would have sent a password that had already been declined — and
then reported the rejection as though the credential were wrong, which is F63's failure mode
arriving by a different route.

Found by reading the capability line off the real server, not from the specification, which is
the argument for the probe in F68.

The first fix did not work and its own test proved it. `ImapTranscript::capabilities` holds
`imap-proto` atoms rendered with `Debug`, so the entry is `Atom("LOGINDISABLED")` and
`eq_ignore_ascii_case("LOGINDISABLED")` never matches. The backend's existing check used
`contains`, which does match — and would equally match `REMOVE` for `MOVE`, or `UIDPLUS` for
`UID`. That is the fourth appearance of F67's mistake, so it is now a rule in `CONVENTIONS.md`
rather than a finding, and both call sites go through `imap::has_capability`.

### F70 — `mailo sync` would have panicked on its first TLS connection

The worst defect in this session, found on the first attempt to make a real TLS connection.

rustls 0.23 refuses to choose when more than one crypto provider is compiled in, and **panics**
rather than returning an error. Two are compiled in here and neither is removable: `mail-runtime`
selects `ring` in its own `Cargo.toml`, while `reqwest` and `keyring` bring `aws-lc-rs`. Nothing
called `CryptoProvider::install_default`.

So every TLS connection this program exists to make — POP3 on 995, Gmail on 993, Exchange on 993 —
would have aborted the process while building the session. Not failed: aborted. The one account
the user can connect to today would have crashed the binary on the first `mailo sync`.

**518 tests passed throughout.** Every fake server in this repository listens on loopback with
`Tls::Plaintext`, which is the one setting no real account uses, so the entire TLS path was
unexecuted. It was not under-tested; it was untested, and the coverage of everything around it
made that invisible.

The install goes in `Transport::wrap` rather than `connect`, because `upgrade` builds a session
too and `wrap` is the only place that touches rustls at all.

### F71 — And the regression test for it was vacuous, twice in one session

The first version connected to port 1, which refuses at TCP before rustls is reached. It passed
with the fix removed. The listener now accepts and then stays silent, so the handshake fails on
its own terms and the provider is installed on the way there — verified by removing the install
and watching it panic.

That is the second time in this session a test written *specifically* to guard a bug did not
exercise the bug (F60 was the first). Both were caught by the same habit — reintroduce the defect,
watch the test fail — and in both cases the test had looked obviously correct. The habit is worth
more than any single test it has validated.

### F72 — Submission checked against a server nobody here wrote

The excuse this time was "I can't authenticate". Partly true — Google and Microsoft need a client
registration — and, again, over-generalised. `aiosmtpd` is a real SMTP server implementation, it
supports `AUTH`, and it installs into a scratchpad virtualenv with no system changes.

It accepted this client's `EHLO`, its `AUTH PLAIN` — meaning the base64 decoded to credentials a
server written by other people recognised — its envelope commands, and its `DATA`. Both
recipients arrived as separate `RCPT TO` commands. A wrong password came back as
`535 5.7.8 Authentication credentials invalid` and was reported as an authentication failure
rather than a delivery or a crash.

The part worth the effort is dot-stuffing, checked by reading what the server *received*. Three
lines that break careless clients — a line containing only `.`, a line beginning with `.`, and a
line beginning with `..` — arrived byte for byte. Disabling `stuff_line` and re-running shows why
it matters: the bare dot ends `DATA` early, the server reads the rest of the message as SMTP
commands, and the exchange deadlocks. Against the fake in `submission_end_to_end.rs` that same bug
is invisible, because the fake unstuffs whatever it is handed.

`scripts/live-smtpd.py` is committed so this is repeatable, and both tests are `#[ignore]`d and
skip when nothing is listening — a test that fails because a developer has not started a daemon
is a test people learn to ignore.

No authentication attempts were made against the campus server or any other third party. A failed login against
a university's production server risks the user's own address being rate-limited, which is not
mine to spend.

### F73 — A sync against a real IMAP server, and three bugs in my own fixture

The excuse was "IMAP has no pip-installable equivalent" to `aiosmtpd`. Wrong: Twisted ships
`IMAP4Server`, a real IMAP4rev1 implementation written by people who were not thinking about this
client, and it installs into the same scratchpad virtualenv.

The client's `LOGIN`, `SELECT`, `UID FETCH` and literal handling pass against it.

**Correction (F88).** This entry originally said `ENVELOPE` and `BODYSTRUCTURE` passed too. They
did not, and the claim did not survive being re-run from a clean server. The body in the fixture contains `A1 OK not really` and a stray
`)`, which are what break a parser that scans for a tagged response or counts parentheses instead
of honouring the literal's byte count.

Every failure along the way was in the fixture I wrote, and saying so precisely matters more than
the passing result:

1. **`UID FETCH 1:*` → `BAD ... Can't iterate; last value not set`.** Twisted hands the raw
   `MessageSet` to the mailbox and expects the *mailbox* to say what `*` means. Isolated with
   Python's own `imaplib`, which got the identical error against the same server — so the client's
   syntax was never in question.
2. **An `ENVELOPE` of all-NIL addresses**, which `imap-proto` rightly refused: a 3-element address
   is not an address. `twisted.mail.imap4.getEnvelope` looks up `from`, `to`, `date` in **lower
   case**, and the fixture returned an uppercased dict, so every lookup silently returned `None`.
3. The same case bug in the `names` filter.

Worth recording from (2): one unparseable response failed the entire walk. Here that was correct
— the response really was malformed — but a real server sending one odd `FETCH` would break a
whole sync rather than skipping a message. Deliberately not changed: recovering response framing
in the presence of literals means guessing where the next response starts, and guessing wrong
corrupts mail rather than dropping it. Recorded as a known trade-off, not fixed on a hunch.

### F74 — The OAuth token exchange had never run

`begin` was well covered: the authorize URL carries PKCE, `state` and `access_type=offline`, and
a mismatched `state` is refused. `exchange`, `refresh` and the `send` adapter between `oauth2`
and `reqwest` had **no tests at all** — the three functions that speak HTTP, and precisely the
code that runs the first time anyone supplies a client id.

By this point in the session that description alone was reason enough to look: every other
never-executed path here has turned out to be broken (F44, F51, F55, F56, F69, F70).

This one is not, which is worth stating as plainly as a defect would be. The exchange sends
`grant_type=authorization_code`, the code, and a `code_verifier`, and sends **no**
`client_secret` — an installed application has none, and offering an empty one is how a client
gets rejected. A refresh sends `grant_type=refresh_token`. An `invalid_grant` comes back as an
error rather than a credential. All four now run against a `TcpListener` in-process, so no
network and no client id.

The one behaviour worth the trouble: Google omits the refresh token on a repeat authorization,
and `refresh` keeps the stored one rather than overwriting it with nothing. A client that gets
that wrong loses the account an hour later, and the cause is invisible by then. Verified by
replacing the fallback with `unwrap_or_default()` and watching the test fail.

`Endpoints` is public and substitutable to make this testable, and that is not only a test seam:
Microsoft runs sovereign clouds on different hosts (`login.microsoftonline.us`), and a deployment
behind an inspecting proxy needs the same. `Pending` now carries the endpoints resolved when the
request began, rather than looking them up again when it completes — a second lookup is a second
answer, and the code came back from whichever host was actually asked.

### F75 — Search took four and a half seconds on a ten-thousand-message mailbox

"I cannot judge usability" is true of taste and false of scale, and every test in this repository
held two or three messages. That is enough to check *what* a query returns and says nothing about
what it costs.

`Filter::Text` compiled to a **correlated** subquery:

```sql
EXISTS (SELECT 1 FROM messages m JOIN messages_fts f ON f.rowid = m.rowid
        WHERE m.thread = ts.thread AND messages_fts MATCH ?)
```

It mentions `ts.thread`, so SQLite runs it once per row of the outer query. The FTS5 index was
used — ten thousand times. Searching a ten-thousand-message mailbox took **4.57 seconds**.

Without the correlation SQLite evaluates the match once, materialises the threads it hit, and
probes that: `ts.thread IN (SELECT m.thread FROM messages_fts f JOIN messages m ...)`. Same
answer, same thread-as-corpus semantics, and the parity proptest agrees. **14ms**, a factor of
325.

### F76 — Every page of the list sorted the whole mailbox

`EXPLAIN QUERY PLAN` on the list query: `SCAN ts` then `USE TEMP B-TREE FOR ORDER BY`. The
existing `thread_summary_date` index leads with `account`, and the query that draws the list does
not constrain one — a unified inbox is `Filter::InMailbox(Inbox)` with no account clause, which
is exactly what `Filter::All`'s own documentation describes. SQLite cannot use an index whose
leading column is unmentioned, so it scanned and sorted ten thousand rows to return fifty.

Migration 0003 adds `thread_summary(last_date DESC, thread DESC)`, matching the ORDER BY, so the
rows arrive in order and the scan stops when the page is full. Measured both ways:

| | first page at 1k | first page at 10k | page 41 |
|---|---|---|---|
| without | 672µs | 5.18ms | 4.0ms |
| with | 140µs | 237µs | 138µs |

The point is the second column: without it, a page costs what the *mailbox* costs; with it, a
page costs what a page costs. Page 41 matching page 1 is keyset pagination finally doing what it
was written for.

### F77 — I nearly reported the wrong cause

The first run "with the index" was not: `cargo fmt` had already reformatted the `MIGRATIONS`
list, so the edit that was meant to register migration 3 matched nothing and silently did
nothing, while `EXPECTED_VERSION` was bumped. The migration test caught the mismatch — `left: 2,
right: 3` — but only after I had read an improvement off a run where the index did not exist and
was about to attribute it.

The fix was to measure again, deliberately, with the index and then without. Which is the same
habit as reintroducing a bug to see a test fail: a number is evidence of nothing until you have
seen it move for the reason you claim.

### F78 — Every message in an open thread was parsed twice on every render

`reader::render` called `html_of` and `inline_parts`, and each of them read the blob and ran the
whole MIME parser. So opening a conversation parsed every message in it twice — and the shell
re-renders the open thread on *every* revision: a keystroke in the search box, a hover action, a
sync landing.

Measured on a 200-message thread and on five 2MB messages, which is what a deck turns an ordinary
thread into:

| | 200 messages | five 2MB messages |
|---|---|---|
| parsing twice | 10.7ms | 64.3ms |
| parsing once | 3.3ms | 27.8ms |

Better than the 2× the description suggests, because the duplicated blob read went with it.

`inline_parts` is gone — `render` reads both halves off one `Parsed` — and `html_of` went with
it once the tests moved to `render`, which is what the shell actually calls. Asking the narrower
question was testing a step the application no longer takes.

Neither number was failing a threshold before the change. This was found by writing the test that
made the cost visible at a size anyone would actually have, which is the same move as F75: at two
messages, twice nothing is nothing.

### F79 — Ingest is healthy, and saying so is part of the job

Measured because it is the number a user watches, not because anything suggested a problem: 2500
messages — the maildrop this project was designed around, plus a margin — absorb in **368ms**,
147µs each. Absorbing repeated batches into one growing thread goes from 23ms into an empty
thread to 36ms into an 1800-message one, which is sub-linear and fine.

`refresh_summary` reloads every message of a touched thread, and `write_ingest` collects touched
threads into a `BTreeSet` so each is refreshed once per ingest rather than once per message. That
is the difference between this result and a quadratic one, and it was already right.

Recorded because "I looked and it was fine" is information. A findings file that only contains
defects says nothing about what was examined.

### F80 — Every keystroke recounted every badge

`use_memo` subscribes to every signal it reads. The badge memo read `shell` — to get the sidebar's
places, which never change after construction — and so re-ran on every write to `shell`,
including `shell.write().search = e.value()` on each character typed.

Six indexed counts per keystroke. About half a frame on a ten-thousand-message mailbox, to
recompute numbers that could not have moved: typing in the search box changes which threads are
*listed*, never how many are unread.

The filters are now resolved once in a `use_hook`, and the memo depends on `revision` alone.

The test took three attempts and only the third tests anything. Re-rendering the component does
not re-run a memo, so the first two versions passed with the subscription reinstated — a memo
re-runs when its *dependencies* change, which means the test has to perform the write a keystroke
performs. It does now, and putting the subscription back fails it.

### F81 — Two threads on one connection: no defect, and one measurement

The last structural dimension never exercised: `SqliteStore` holds one connection behind a
`ReentrantMutex` and the shell shares it by `Arc` between the UI thread and a sync on
`spawn_blocking`. Every test until now used it from a single thread, so the contention that
exists in the real program had never happened in a test.

Nothing is wrong. A reader looping on the list query while a writer absorbs a thousand messages
neither deadlocks nor starves; two concurrent writers leave exactly four hundred messages and a
`remote_map` that agrees with them. The reentrant lock is sound here for the reason its comment
gives — `rusqlite` needs only `&Connection`, so recursion hands out a second shared reference —
and that reasoning holds across threads as well as within one.

The measurement worth keeping: the reader completed **23 reads** while nine 200-message batches
were written, about ten repaints a second. Reads and writes take turns for the length of a batch,
so the list updates in steps during a sync rather than smoothly.

`SqliteStore`'s own comment said "revisit when a profile says to". The profile now exists and is
recorded next to it — and it does not say to. A pool would buy parallel reads at the price of a
reader seeing a half-written thread summary, which is a *wrong* answer where this is a late one.
Written down rather than acted on, so the next person has the number instead of the impulse.

### F82 — The codebase disagreed with its own convention, and the convention was wrong

An audit of `CONVENTIONS.md` against the code, rule by rule, because whether a project obeys its
own stated rules is mechanically checkable and had never been checked.

Most hold. No pure crate reads the clock — the only match for `Utc::now()` in `mail-domain`,
`mail-mime` or `mail-proto` is a comment explaining why a library below `mail-runtime` must not.
No credential reaches SQLite: nothing in `mail-store` binds a password or a token as a parameter.
`deny_unknown_fields` appears nowhere but in a test asserting it appears nowhere.

One does not. §3 says "every field added after the first release carries `#[serde(default)]`",
and `ProtoOp::Submit`'s `mail_from` and `rcpt_to` do not — added in F37, on a type persisted as
`outbox.op`.

That was deliberate and it is still right: `#[serde(default)]` would give an old row an empty
recipient list, and a submission with no recipients is a message that goes nowhere while the
outbox reports success. A silent loss is worse than the loud decode failure it would replace.

So the defect is in the rule, not the code. A rule with an unstated exception is worse than no
rule: the next person either follows it and creates the silent failure, or breaks it and has no
idea a precedent exists. §3 now says default *when the default is right*, names this case, and
adds the test — check which value it is about to invent, because `Vec::new()` for a recipient
list and `String::new()` for an address are not neutral.

### F83 — The rest of the audit, and cleaning up after myself

Continuing F82's pass through `CONVENTIONS.md`:

- **§7, no `bool` in domain state.** One match, `Container.dead` in `threading.rs` — a private
  field in the JWZ algorithm's own scratch structure, which is not domain state. The rule cites
  `plan.md` principle 6 and means the vocabulary types. Holds.
- **§7, no `get_` prefix.** No matches anywhere.
- **§9, no test touches the network; live tests are `#[ignore]` and live in `mail-runtime`.**
  All three — `live_probe`, `live_smtp`, `live_imap` — are exactly that. I wrote them before
  re-reading this rule, which is a better result than the alternative but not a reason to skip
  the check.
- **§8, a file over ~400 lines wants splitting.** Seven are over, and the one I am responsible
  for is `ui.rs`: it grew from roughly 290 lines to 1026 across this session, 732 of them code.

So `ui.rs` became `ui/mod.rs` (501 lines of code), `ui/composer.rs` (197) and `ui/style.rs` (56).
The composer is one concept — what is being written and where it goes — and fifty lines of CSS in
the middle of a component tree helped nobody find either.

The remaining six oversized files are left alone. `smtp.rs` at 1477 lines is one state machine
and splitting it would put a protocol's phases in different files to satisfy a line count, which
is the rule read as a target rather than a signal. Recorded so the next person sees a decision
rather than an oversight.

Verified the way a UI refactor has to be: the suite stays green, and the window still opens.

### F84 — The migration that repairs data had never run on data needing repair

`plan.md` asks for "a test that opens a checked-in fixture DB from each prior version and
migrates it", under a heading calling it required from day one. It did not exist. Every test
creates a fresh database, so each migration ran against an *empty* schema — and a migration whose
job is to repair data has nothing to repair.

That is worst for 0002. Its purpose is to collapse the duplicate `remote_map` rows every
version-1 database accumulated (F52) and then add a unique index. Remove the `DELETE` and the
`CREATE UNIQUE INDEX` fails, the transaction rolls back, and `migrate` returns an error — the
application does not start, for exactly the users who have run it longest. That is the failure
`tests/upgrade.rs` now reproduces and the suite had no way to see.

The migration is correct; the test was missing. Each version is built by applying a prefix of
`MIGRATIONS` rather than checking in a binary fixture, so what is tested cannot drift from the
migration it represents.

One thing this took two attempts to get right. The first mutation — grouping on the raw columns
instead of `COALESCE` — did not break anything, because SQLite's `GROUP BY` treats NULLs as
*equal* while a `UNIQUE` index treats them as *distinct*. That asymmetry is precisely what caused
F52, and it means a dedup written the obvious way happens to work. Removing the `DELETE`
altogether is the mutation that fails, and it is the one that matters.

### F85 — The plan described an engine that was never built

Auditing `plan.md` against the code, since "finish the plan" is only meaningful if the plan
describes what exists:

- The `AccountEngine` sketch carried a `caps` field. The real one has none — capabilities belong
  to the backend, which is what discovers them, and a copy here would be a second answer to one
  question. It also predates `schedule` and `last`, the three intervals.
- "one WAL connection pool" — it is one connection, deliberately, and `SqliteStore`'s own comment
  argues why.
- `Secrets` was sketched with a `SecretError` that does not exist and without `forget`.
- The `#[serde(default)]` rule was stated absolutely, which F82 has since shown is wrong.

The table list, the crate graph and the phase structure all match. Corrected in place rather than
appended to, because a design document that contradicts the code teaches the reader something
false with the same confidence as the parts that are true.

### F86 — The plan's own required mitigations, checked

Continuing F85's audit, because the plan states requirements and whether they hold is
checkable — and F84 showed one of them had simply never been done.

The two "both required" mitigations against `Filter::fit` and the SQL compiler diverging:

1. **`MemoryStore::threads` implemented by calling `fit`, never a second matcher.** Holds —
   `matching` builds a `MatchCtx` and calls `filter.fit(&ctx)`. A second hand-written matcher
   would make the parity test compare two wrongs, which is the trap the mitigation names.
2. **A proptest asserting `fit(f, ctx) ⟺ id ∈ sqlite_query(f)` over random filters.** Holds, and
   the generator really does produce `Filter::Text` — which matters this round, because F75
   rewrote exactly that clause's SQL from a correlated subquery to an uncorrelated one. The
   change was covered by a property the plan required before either of us thought about it.

Also checked and holding: attachments are `BlobId` and never a path; `Change` is domain-level and
never a SQL row; `Tls` has no opportunistic variant.

A requirement nobody re-reads is a requirement that quietly stops being true. Two of these were
still true; the third, in F84, had never been true at all.

### F87 — A design document should say what was built, including where it lost

`plan.md` specified `cid:` resolution "through a custom protocol handler keyed on `BlobId` only",
and the shell does not do that — it inlines parts as `data:` URIs, because a custom scheme
requested from a `sandbox=""` document is refused as cross-origin and the only way to permit it
is `allow-same-origin`, which defeats the iframe. That was decided in F42 and recorded in
FINDINGS, which is where nobody reading the design will look.

The plan now says it, with the reasoning, in the place the reader will be standing when they ask
why. Same for the list pane, which grows a page rather than following `Page::next`.

And each phase now carries what is actually true of it rather than only its criterion: phase 4's
CLI does list, open and reply and has never authenticated to the campus server; phase 5's two clauses are both
met against servers nobody here wrote, with Gmail and Exchange outstanding for a client id that
is registered rather than written; phase 6's shell does everything checkable from inside the
repository, and its criterion is a judgement about using the thing with real mail over days.

Writing "done when" and leaving it is how a plan becomes a wish. Writing what happened next to it
is the difference between a document that closes and one that is merely abandoned.

### F88 — I reported a pass I could not reproduce

F73 says the live IMAP test covered `ENVELOPE` and `BODYSTRUCTURE`. Re-run from a clean server
this round, `a_real_server_accepts_login_select_and_fetch` **failed**, and had been failing: the
fixture's messages carried no `Content-Type` and no `Content-Transfer-Encoding`, so Twisted
rendered `BODYSTRUCTURE (NIL NIL NIL …)` and then `("text" "plain" … NIL 251 1 …)` with a NIL
encoding, where RFC 3501 requires `body-fld-enc` to be a string. `imap-proto` was right to refuse
both.

I had restarted that server several times while editing the fixture, and reported a result from a
run whose server state I had not pinned down. The lesson is not about IMAP: a test that talks to
something started by hand is only evidence if you know what was answering, and "I saw it pass"
is not that. The clean experiment — kill everything on the port, start one process from the
committed script, run — took two minutes and settled it.

Both tests pass now, against a server started that way, and the fixture is committed in the state
that makes them pass. F73 is corrected in place rather than left with a footnote, because someone
reading it for what is covered should not have to find this entry to learn it was wrong.

### F89 — The one function the binary calls could not be run

`sync::run` ties configuration, credentials, the backend, the engine and the store together, and
reached for `KeyringSecrets` directly — so the assembly was the one thing no test could execute,
while every layer beneath it had tests. `run_with` takes the secret store; `run` supplies the
keyring.

What that buys is the assembly itself: an account with no credential is skipped with a reason
that names the command which fixes it, an unreachable server is reported against that account
rather than thrown, and — against the Twisted server — a stored plan becomes a real connection
and the user is told `2 headers, 2 bodies, 0 queued operations settled, 0 sent`.

Worth noting what could *not* be tested and why. `presets::manual` only ever produces
`Tls::Implicit`, so no configuration this program will write can reach a plaintext loopback
server; the test constructs that plan by hand. That is the security posture working as intended —
there is no setting that sends a password in clear — and it means the binary's own configuration
path can only ever be exercised against a server with a certificate a public root will sign.

### F90 — Turning F88's lesson into a mechanism

F88 was a result I reported without controlling what was answering the socket. The instance is
fixed; the process that produced it — remembering to do the clean experiment — is one that fails
whenever attention is elsewhere.

So the first thing this round was to re-verify every live result from a known state: stop
anything on the ports, start both servers from the committed scripts, run all four suites. All
six tests pass. F88 was the only claim that did not hold, which is worth knowing precisely rather
than hoping.

The second was to stop relying on myself for it. `scripts/live-tests.sh` creates the virtualenv
if it is missing, kills whatever holds the ports, starts both servers from the committed scripts,
waits until each answers, runs the ignored suites and exits non-zero if any fail. The network
probe is opt-in behind `--network`, because a suite that reaches a third party by default is one
someone runs without meaning to.

Checked the way anything else here is: reintroducing F88's exact fixture defect — the missing
`Content-Transfer-Encoding` — makes `live_imap` and `sync_path` fail and the script exit 1, and
restoring it returns both to green.

The general shape, since this is the third time it has come up: a test is only evidence about the
thing you believe it ran against. Transcript tests have that for free. Anything that needs a
daemon does not, and the fix is to make the setup part of the test rather than part of the
operator.

### F91 — Every message fetched over IMAP carried a stray `)`

Found by running the whole journey through the real binary — `sync`, `list`, `show`, `reply`,
`send`, `sync` — as a user would, rather than testing each step on its own. `show` printed the
message, and then a `)` on a line of its own.

`Job::Fetch` handed the *entire untagged response* up as the body:

```
* 2 FETCH (UID 102 BODY[] {223}\r\n<223 bytes>)
```

So every message stored from an IMAP server had `* n FETCH (...)` prepended and the response's
closing paren appended. A lenient MIME parser swallows the prefix without complaint, which is why
nothing downstream objected.

Two more defects rode along in the same line. The text came from `String::from_utf8_lossy`, so
every 8-bit byte in a message became U+FFFD — a Latin-1 mail is silently mangled and the stored
blob is wrong for ever after. And it was `trim_end`ed, which is right for protocol vocabulary and
wrong for mail: a message may legitimately end in blank lines.

`Untagged` now carries the raw bytes beside the text, and `Untagged::literal` takes exactly the
byte count the server promised. `text` stays, because `SEARCH` results and `FETCH` attribute
names are ASCII vocabulary.

**Why the end-to-end test missed it.** `a_literal_body_survives_a_line_that_looks_like_a_tagged_
response` asserted `contains("A1 OK not really")` — and that was true throughout. A substring
assertion cannot see something *added*. It now compares the stored message to the fixture byte
for byte, and reinstating the old extraction fails it. That is the same lesson as F61 arriving
from the other direction: there, `contains` matched too much; here, it failed to notice too much.

### F92 — `reply` needed a message id that nothing printed

The same journey, one step later. `mailo reply <message-id>` is the documented way to answer
mail, `list` prints thread ids, and `show` printed neither — so the sequence a user follows ended
at a command they could not construct without opening the database by hand.

`show` now prints each message's id under its header. Two lines of code, invisible to every test
that exercised `show` and `reply` separately, and unmissable the moment they are run in order.

### F93 — POP3 does not have F91's bug, checked rather than assumed

F91 found the IMAP path appending the response's closing paren to every message and running each
body through `from_utf8_lossy`. POP3 is the protocol the first real account here will use, so the
same questions had to be asked of it — against `twisted.mail.pop3`, not against a fake of mine.

It is clean, and the fixture is built to break a careless client in the three ways POP3 can: a
body line beginning with `.`, which the server doubles and the client must undouble *exactly*
once; 8-bit bytes, which must not meet a UTF-8 conversion; and CRLF endings. The retrieved
message is byte-for-byte what the server holds. `join_lines` and `unstuff` work on `Vec<u8>`
throughout, and the only `from_utf8_lossy` in `pop3.rs` builds a snippet for an error message.

`tests/live_pop3.rs` asserts that byte for byte rather than with `contains`, which is the
assertion that let F91 through, and `TOP 0` is exercised too — it is what keeps a first sync from
marking an entire maildrop read in the user's webmail.

One detour worth recording. The first run showed every line ending as `\r\r\n` in the store, which
looked exactly like a client bug. It was on the wire: `twisted.mail.pop3` splits a message on
newlines and terminates each line it sends with CRLF, so a fixture written with `\r\n` is emitted
as `\r\r\n`, and our client stored faithfully what arrived. Dumping the socket before blaming
anything on this side took two minutes and is the F88 discipline paying for itself a second time.

### F94 — A send that did not go said nothing

Found the same way as F91: running the commands in order and reading what they printed. `mailo
send` queued a reply, the SMTP server was not listening, and `mailo sync` answered

```
ada@example.test: 2 headers, 0 bodies, 0 queued operations settled, 0 sent
```

Nothing wrong, on a line that looks like a report of success. The draft was marked `failed` in
the store, but only `mailo drafts` shows that, and nobody runs it after a sync that did not
complain.

The cause is a reasonable rule applied one step too far. `drain_outbox` adds to
`needs_attention` only for `NeedsReauth` and `Fatal`, because a refused connection backs off and
retries — and interrupting someone about a laptop lid would train them to ignore the warnings
that matter. But silence is not the only alternative to an alarm. Someone who has just typed
`send` and then `sync` reads "0 sent" as "there was nothing to send".

`SyncReport::still_queued` counts what remains after the pass, and the CLI says so plainly: `1
still queued; run sync again to retry, or mailo drafts to see why`. Not an error, because it is
not one; not nothing, because it is not that either. The line disappears when the message goes.

Asserted in both directions — a refused submission reports one waiting, a delivered one reports
none — and checked by pinning the count to zero and watching the test fail.

### F95 — Every sync re-downloaded every header in the mailbox

Found by doing the most ordinary thing a mail client does: syncing twice. The second pass, with
nothing new on the server, reported `2 headers` — and the fixture holds exactly two messages.

```rust
let mut wanted = self.backend.surveyed();
if wanted.is_empty() { wanted = self.unfetched(budget)?; }
```

The survey is *everything on the server*. That is the right answer to "what exists" and the wrong
one to "what should I fetch", and nothing subtracted what was already stored. On POP3 this has
always been so: one `TOP` per message per poll, which on the 2372-message maildrop this project
was measured against is 2372 commands every five minutes, against a campus server, for mail
already on disk. On IMAP it began when F44 taught `surveyed()` to return anything — before that
it fell through to `unfetched` and was accidentally correct.

The header pass now subtracts `Store::remote_refs`. A message whose header is stored but whose
body is not is deliberately *not* wanted there; `fetch_bodies` asks `unfetched` for those, which
is the question that one answers.

`syncing_three_times_stores_each_message_once` had covered this ground and stayed green
throughout, because it asked whether anything was *duplicated*. Nothing was — the work was simply
done again and thrown away. The new assertion is on `headers_fetched` being zero when nothing has
arrived, which is the thing that was untrue.

Also verified, since the point was the daily loop: a message that appears between two syncs is
fetched (one header, one body, not three), lands at the top of the list, and is counted unread.
The IMAP fixture can now gain messages at runtime, which is what made that testable at all.

### F96 — An OAuth account would have stopped working an hour after it was added

`oauth::needs_refresh` and `oauth::refresh` had no callers. None. `sync::one` read the credential
out of the keyring and handed it to the backend exactly as stored, so an access token — which
lasts about an hour — was used until it expired and then used forever afterwards, failing every
pass with an authentication error. The refresh token that would have fixed it was sitting in the
same keyring entry, untouched, from the moment `account add` wrote it.

The second half is worse: renewing needs the client id, and nothing kept one. `account add` read
`MAILO_OAUTH_CLIENT_ID` from the environment, used it for the browser round trip, and dropped it.
So even a caller for `refresh` would have had nothing to call it with. `AuthPlan::OAuth` says in
a comment that the client id "lives in runtime config, keyed by `OAuthIssuer`" — there was no
such config. The comment described an intention.

It is now `mail_runtime::signin`: a `Registration` per issuer in
`$XDG_CONFIG_HOME/mailo/oauth.json`, written by `account add` and read once per sync run. Not a
keyring entry, because an installed application cannot keep a secret — that is the entire reason
for PKCE — and a client id is not one.

The decision and the effect are separate on purpose. `oauth::assess` is a pure function of the
credential and the clock returning `Freshness::Ready | Expired { refresh_token }`, so the
refreshing branch cannot be reached without the token it needs in hand; `signin::renew` performs
the exchange and writes the result back under both `IncomingPassword` and `OAuthRefresh`, since
`account add` writes both and a stale copy under either is the same account failing an hour later
by a different route. Writing back rather than only returning also means SMTP submission later in
the same pass picks up the renewed token, because the engine reads it from the same `Secrets`.

Found by asking what `needs_refresh` was for and grepping for its callers, which is the same
sweep that found F44 and F52. Proved through the real binary rather than only in tests: an
account pointed at a loopback IMAP port, an expired credential in the keyring, a registry whose
endpoints override names a local token endpoint. The binary sent
`grant_type=refresh_token&refresh_token=…&client_id=local-test-client`, wrote the renewed token
back to the keyring with a future expiry and the original refresh token preserved, and on the
next run did not contact the issuer at all. Then the keyring entries were cleared.

### F97 — `cargo fmt` collapsed two messages into sentences with holes in them

A `\`-continuation inside a string literal strips the newline *and* the next line's indentation.
`cargo fmt` had joined two such literals into one line and kept the indentation, so what shipped
was:

```
the server advertises LOGINDISABLED: it does not accept passwords on                          this connection, so one was not sent
```

Twenty-six spaces, in a message a person reads when their account will not log in. The second was
the new "no OAuth client id is configured" line, which acquired fourteen the moment it was
formatted. Both are now `concat!` of whole fragments, which `fmt` cannot rejoin, and both tests
assert the rendered message contains no run of two spaces.

Swept the whole workspace for the pattern; those were the only two. The two other hits are
deliberate — test fixtures whose subject is exactly that whitespace is preserved.

### F98 — Every date in the application was shown in UTC, including in mail sent to other people

Found by running the program on a machine set to `Asia/Taipei`, `+0800`. A
message the fixture stamped `09:02 +0800` appeared in `mailo list` as `01:02`. At the moment this
was found the clock read 07:25 on the 22nd, UTC read 23:25 on the 21st, and every row, every
header in the reader and every draft in the shell was dated the 21st.

Six sites, each formatting a `DateTime<Utc>` directly:

| where | pattern |
| --- | --- |
| `cli.rs` list rows | `%m-%d %H:%M` |
| `cli.rs` `show` | `%Y-%m-%d %H:%M` |
| `ui/mod.rs` thread rows | `%b %d` |
| `ui/mod.rs` draft rows | `%b %d` |
| `ui/mod.rs` reader header | `%Y-%m-%d %H:%M` |
| `compose.rs` quote attribution | `%a, %d %b %Y at %H:%M` |

The last one is the serious one. It is not displayed — it is *written into the body of a reply*,
so `On Tue, 22 Sep 2026 at 01:02, Grace Hopper wrote:` went out over SMTP quoting a message Grace
wrote at 09:02, into her mailbox, permanently. Confirmed on the wire against the local
`aiosmtpd` fixture before the fix, and after it the same reply reads `On Tue, 22 Sep 2026 at
09:02`.

It is now `view::Stamp` — an enum of the four places an instant appears — and `view::stamp`,
which takes the zone as a parameter. A parameter and not `Local` read from inside, because a
function that asks the machine what zone it is in can only be tested against whatever that
machine is set to, and on a runner set to UTC that is the bug passing. `compose::draft_reply`
keeps its signature and delegates to `draft_reply_in`, which names the zone, in the same shape as
`sync::run`/`run_with` and `oauth::refresh`/`refresh_at`.

The tests use `FixedOffset` for `+08:00` and `-05:00` and assert both directions — a zone behind
UTC must roll the date *back*, which a fix that only ever added hours would get wrong — plus a
UTC case that must be unchanged. The original in the compose fixture is 22:13 on Tuesday the 14th
in UTC and 06:13 on Wednesday the 15th in Taipei, so the attribution assertion differs in hour,
day *and* weekday and cannot pass by accident. Reverting the one line makes it fail.

Why nothing caught it: every test formatted the instant the same way the code did, or asserted
that a date was merely present. Six sites, forty-three view tests, and none of them ever asked
what time it was where the user is.

### F99 — The CLI offered a send that could not work

`mailo reply` on a message you sent yourself prints an empty `to`, then `send it with: mailo send
<id>`. That command fails with `cannot build a message with no recipients`, and the CLI has no
command that adds a recipient — so the advice was not merely useless, it was unfollowable.

`Draft::reply_to` dropping your own address is right; a reply to yourself has nobody to go to.
What was wrong was saying so nowhere. The command now says the only address on the original was
your own and points at the composer.

Found in the same session as F98, doing the thing that turned it up: replying to a message.

### F100 — The shell's layout had never been seen, and three things were wrong with it

The window is a WebView surface owned by the compositor; under rootless XWayland an X11 grab of
it returns `BadMatch`, and this machine has no compositor screenshot tool or nested X server. So
the components had been *executed* in tests since F61 and nobody had ever looked at the result.

There is a way, and it needed no software on the machine. `dioxus-ssr` renders the same component
tree to HTML; inlining `STYLE` makes a self-contained page; a headless browser already installed
screenshots it:

```
cargo test -p mail-app --bins -- --ignored render_the_shell_to_a_file
google-chrome --headless --screenshot=shell.png --window-size=1200,800 target/shell.html
```

It is the markup and the CSS, not the running application — nothing clicks and a WebView is not a
browser — but it is the difference between looking and guessing. Three `#[ignore]`d tests write
`shell.html`, `shell-real.html` and `reader.html`; the second uses a fixture shaped like real
mail, because the one it had held a single message from "Ada" with the subject "hi", a size at
which nothing can be wrong.

With six realistic rows on screen:

1. **Every subject was truncated after about twenty characters** while the reader pane held six
   hundred pixels of nothing. `.app` was `180px 380px 1fr` — the list never grew — and `.row` gave
   a flat `140px` to the sender, so in a 380px pane the name got 37% of the width whether it was
   `Dr. Wolfgang Amadeus Pemberton-Featherstonehaugh` or `Mum`. Now `minmax(340px, 32%)` and
   `7fr 13fr`, so both scale and the subject always gets roughly twice the sender.

2. **Every row said `Sep 22`**, including the message that had arrived an hour earlier — the least
   useful answer available, and the same one it gave for mail from three weeks ago. `view::listed`
   now writes the time for today, a weekday for the last week, a day and month within the year and
   a full date beyond it. Today is a *calendar* day and not the last twenty-four hours, which is
   two separate test cases: 00:10 this morning is today although it is fourteen hours ago, and
   23:50 last night is not although it is fourteen hours ago as well.

3. **"Load remote images" was offered above every conversation in the mailbox** — plain-text ones
   included — because nothing asked whether anything had been blocked. An offer that is always
   there is furniture, and this one asks the user to make network requests on a sender's behalf.
   `sanitize` is the only thing that knows what it dropped, so `SafeHtml` now carries the count
   and `Reading::Html` a flag. Only `http`/`https` count: a `javascript:` src is dropped too, and
   offering to load *that* puts a button in front of a user whose only answer makes things worse.

The lasting part is not the screenshots. `dioxus_ssr::render` on the existing `VirtualDom`
harness means the shell's markup can be asserted, which it never could be — the first three such
assertions are in `ui::render_tests`, and the one about the images button fails if the gate is
forced open.

### F101 — "Discard" did not discard, and nothing in the application could delete a draft

Found by rendering the composer, which nothing had ever looked at either: it only exists while a
draft is open, so the page dumps from F100 had to go through the harness that already existed for
the hook-order test.

The button closed the pane without saving. For a draft that had never been saved that is the same
thing as discarding it — and `Composing`'s own doc comment says how often that case arises:

> The draft this edits. It **already exists in the store** before the composer opens.

A reply is written to the store the moment `draft_reply` creates it, and a draft opened from the
drafts list came off disk. So "Discard" closed the pane and left the draft in Drafts, permanently.
Worse, there was no other way to remove one: not in the shell, not in the CLI. `Change::
DraftDelete` had existed in the domain since phase 1 with no caller anywhere.

`compose::discard` now deletes it, and both surfaces use it — `mailo discard <draft-id>` and the
button. It refuses `Queued` and `Sending`, because deleting either leaves the outbox draining
something that is no longer there and `Sending` may already be on the wire; it allows `Sent`,
because that is a record rather than work in progress and the drafts list is the only place it
appears, so refusing would make the list unclearable.

The button now destroys something, and it sits beside Close, so it asks once first. The decision
is `view::discard_click` returning `Confirm | Delete(id)` rather than a branch inside the click
handler — the same reason `op_for` and `hover_actions` are values: a decision that exists only
inside a closure attached to a DOM node cannot be tested without a DOM. Reopening the composer
resets the confirmation, since one that survives the pane closing is a trap set for the next
draft.

Two things looked at and deliberately left alone:

- **Dark mode**, now rendered by every page dump with `color-scheme: dark` forced on the root.
  The stylesheet leans on `Canvas`/`CanvasText` rather than hard-coded colours, so it resolves
  correctly and needed nothing. The sanitized message still renders on white inside its frame,
  which is the sender's document and what other clients do.
- **The composer's body box renders empty in these dumps.** That is the renderer, not the app:
  dioxus emits `<textarea value="…">` and HTML wants the value as the element's text. A WebView
  sets the DOM property, where it works. Recorded so the next person to look does not chase it.

### F102 — The first thing a new user sees said "Nothing here."

The one state no fixture covered, because every fixture seeds an account before rendering. With
an empty database the shell draws six folders, a Sync button, and `Nothing here.` — identical to
a mailbox that happens to be empty. Nothing says an account has not been added, and the shell
cannot add one: that is a terminal command. So it was not an empty state, it was a dead end.

The CLI had this right from the start — `mailo sync` on an empty database says "no accounts. Add
one with: mailo account add <address>" — which is what made the shell's version easy to miss:
the sentence existed, in the other surface.

`view::nothing_to_show(accounts, search) -> Nothing` now decides, with `NoAccount` checked before
the search: with no account there is nothing to search, and "nothing matches" would send a new
user hunting for a typo instead of doing the setup step they have not done. `Nothing::command()`
is separate from `message()` so the shell can set the command in a monospace box rather than in
the italic the rest of the pane uses — a command shown in italic prose is a command someone
retypes wrongly.

A search that matches nothing now says so and quotes the words, which are the part most likely to
be mistyped. An empty folder still says `Nothing here.`, because that one was right.

Rendering the first run is how it was found, and the first-run page is now one of the dumps.

### F103 — The shell had no keyboard, and the shell cannot run a spawned future

Not one key handler anywhere in `mail-app/src/ui`. Moving between conversations, opening one,
archiving, starring, replying and closing a half-written reply were each a mouse click and
nothing else. A mail client is a thing people sit in for hours, and this is the part of "daily
driver" that does not depend on anyone's taste.

The decision layer is `view::shortcut`, `view::op_for_shortcut` and `view::step`, in the same
shape as `op_for` and `hover_actions`, and it is tested:

- **`typing` is the whole of the safety.** A letter is a shortcut while reading and a letter
  while writing, and the client that confuses the two archives a conversation because someone
  typed "e" into a reply. Every letter key is asserted dead while typing; `Escape` is asserted
  alive, because closing what you are typing in is not something you can be asked to reach for
  the mouse to do.
- **Toggles resolve through `hover_actions`**, so `s` on a starred thread unstars it and `e` on
  something that was never in the inbox does nothing — one table of what is possible, not two.
- **Movement does not wrap.** A list that jumps from the bottom back to the top loses the user's
  place in a way that is hard to notice and easy to act on: the next keystroke archives the wrong
  thing. A selection that has left the list — archived out from under itself — lands at the end
  the movement comes from rather than nowhere.

Wiring it up turned over something larger: **a future spawned from a component body is never
polled in this application.** `use_hook`'s closure runs and a `spawn` inside it never starts;
`use_future` never runs its body at all. Both were established with prints that do not depend on
any input reaching the window. Everything else followed from it — `document::eval` returns a
handle that is driven by a task, so the script never ran; `MountedData::set_focus` is a future,
so the root was never focused; and an unfocused root means keydown targets `body`, which is the
root's *parent*, and events bubble up rather than down. `tabindex` alone does not help and
`autofocus` does not either: that attribute is for form controls and WebKit ignores it on a div.

So focus is held by a script injected with `Config::with_custom_head`, which needs nothing from
Dioxus, and the handler is an ordinary `onkeydown`. A test asserts the two halves still name the
same element, since they live in different files and renaming one silently turns the keyboard off.

**What is not verified:** that a key press in the running window reaches this code. The
instrument was not trustworthy — `xdotool getactivewindow` reported our window while
`xdotool getwindowfocus` reported another, so under this compositor synthetic X key events may
never have been delivered to the WebView at all, and every "nothing happened" in that experiment
is equally explained by the keys not arriving. Recorded rather than dressed up: the decisions are
tested, the wiring is argued, and the last inch is one more thing the daily-driver criterion is
for. The spawned-future finding is *not* in that category — it stands on prints alone.

It also means the Sync button, which spawns from a click handler, is worth a look by someone who
can watch the window. Spawning from an event handler is a different path from spawning during a
render and may well be fine; this could not settle it.

### F104 — The last inch of F103, closed: events can be dispatched in a test

F103 left two things open, both because the only instrument was pressing keys at a window under a
compositor that would not say where the focus was. Neither needed a window.

`VirtualDom::handle_event` delivers an event to the running component tree. It needs two things a
renderer normally provides: a `PlatformEventData` wrapper, and a global `HtmlEventConverter` to
turn that back into the typed data the handler expects — without the converter every dispatch
panics on a failed downcast, which is what it did first. Twenty stub methods and one real one is
the whole of it. `bubbling: true` means the event climbs to the handler on the root exactly as it
does in a browser, so the test does not need to know which element it started from — only that it
started inside the shell, which ids 1 and 2 are not.

**The Sync button is fine.** A task spawned from an event handler does run; it is only a task
spawned from a *component body* that never starts. The distinction is now a test of its own,
`a_task_spawned_from_an_event_handler_does_run`, against a component that exists to ask nothing
else. That closes the question F103 had to leave open, and the answer is that nothing is wrong.

**The keyboard reaches the store.** `a_keystroke_reaches_the_store` presses `j` then `e` on the
real `App` over a real database and asserts the inbox is one conversation shorter. Removing the
`onkeydown` attribute makes it fail. `a_letter_typed_into_a_reply_is_not_a_shortcut` presses `j`,
`r`, `e` and asserts nothing was archived; dropping `composing.is_some()` from the `typing` guard
makes it fail with "an \"e\" typed into a reply archived the conversation behind it", which is
exactly the sentence the guard exists to prevent becoming true.

That second test was vacuous when first written — it asserted `drafts > 0` after `r`, and
`realistic()` seeds a draft of its own, so it was true whatever `r` did. It now counts drafts
across the keystroke. The same mistake as the four in CONVENTIONS §"Substrings are not tokens",
in a new place: an assertion about a *quantity* that was already satisfied before the action.

What remains unverified about the keyboard is now only what happens between a physical key and
the WebView — the layer F103's experiment could not address either. Everything from the event
reaching the document down to the row leaving the inbox is tested.

### F105 — Two more assertions that were already true

Applying the rule F104 produced, as a sweep over every `assert!` using `>`, `<`, `is_empty`,
`is_some` or negation. Most are sound — `explain(...).is_some()` tests a function whose default
answer is `None`, and a `before > 0` guard placed to keep a *later* assertion honest is exactly
right. Two were not:

- `a_whole_sync_over_a_real_socket_lands_mail_in_the_store` asserted `headers_fetched > 0` and
  `bodies_fetched > 0` against a fixture holding exactly two messages. That is true of a pass
  that fetched the same header eight times as well as of one that did its job — the precise shape
  F95 hid behind for the entire life of the project. Both are exact counts now.
- `phase3_milestone` asserted `unread_inbox > 0 && unread_inbox < 20`. Almost any wrong answer
  satisfies a range that wide. It now checks the same filter through `count` and through
  `threads` and requires them to agree, which is a claim a wrong answer cannot accidentally
  meet — and it is a better test than a magic number, because it compares two implementations of
  one question rather than one implementation against a constant copied out of a fixture.

### F106 — The shell has never rendered anything

The one thing behind every "I could not verify that" in F100 through F104: **`mailo` with no
arguments opens a window that stays empty.** Not "looks wrong" — the mount point has no children
at all.

Found by giving the page a way to talk back. The window cannot be screenshotted here and its
stderr says nothing, but a WebView can make an HTTP request to `127.0.0.1`, so a few lines in
`with_custom_head` reported what the document actually contained to a listener in a scratch
directory. What came back:

```
/head-script-ran
/focus-is-BODY
/app-found-no
/main-len0
/roots-DIV|__dx-toast|dx-toast,DIV|main|,SCRIPT||,SCRIPT||
/toast-Your app is being rebuilt.\nA non-hot-reloadable change occurred and we must rebuild.
/late-main-len0-app-no
```

`main-len0` at 1.5 seconds and again at 6. No `.app` element, so nothing this shell renders was
ever in the document. Narrowed from there:

- **Not the debug profile.** A release build says the same, minus the devtools toast.
- **Not our `App`.** A component whose whole body is `rsx! { div { "hello" } }` mounts nothing
  either.
- **Not the focus script.** Stripped to a bare probe, `main` is still empty.

So it is the renderer's plumbing. Dioxus 0.7 desktop applications are built and launched through
`dx`, which is not installed here and is not this repository's to install. Nothing in the project
said so: `ORCHESTRATION.md` documents every CLI command and never mentions how to start the shell.

This reframes four earlier findings. F103's "spawned futures are never polled" is what an
unmounted tree looks like from inside — no component body ever ran, so of course nothing it
spawned did. The `xdotool` experiments could not have worked against a document with no handlers
in it. F100's screenshots were of `dioxus-ssr` output, which is exactly what the shell *would*
draw and remains the right way to look at it, but it was never what the window was showing.

What is fixed here is the part that is ours: a blank window that explains nothing is the worst
version of this. If the mount point is still childless after four seconds, the page now says the
interface did not start, names `dx serve --package mail-app` as the way to launch it, and lists
the commands that work in a terminal today — which is most of the application. Written through
`textContent`, so it cannot become markup. Verified the same way it was found: `/fallback-shown-940`,
in a mount point that had been empty.

What is not fixed is the launch itself. Installing a toolchain onto someone's machine is their
call, and "how do you start it" is the first thing a daily driver has to answer.

### F107 — F106 was wrong: the shell renders, and my harness was launching it wrongly

**Retraction.** F106 says "the shell has never rendered anything". It renders. The same binary,
same profile, ten seconds after launch, reported from inside the page:

| launched as | mount point | `.app` | focus | sidebar |
| --- | --- | --- | --- | --- |
| `./target/debug/mailo` | **5898 chars** | found | **on `.app`** | 7 places |
| `GDK_BACKEND=x11 ./target/debug/mailo` | **0 chars** | missing | BODY | — |

`GDK_BACKEND=x11` is mine. I set it in the F100 round so `xdotool` could see the window, kept it
in every launch afterwards, and never once ran the program the way a person would. On a Wayland
session it forces the WebView onto XWayland, and there the edits Dioxus produces never reach the
page: `window.onload` fires, `window.interpreter` exists, `initialize` is sent, there is no
JavaScript error, and nothing ever arrives. Everything F106 narrowed — release says the same, a
trivial component says the same, the focus script is not the cause — was true and pointed at the
wrong thing, because every one of those runs carried the same variable.

What this corrects, beyond F106 itself:

- **F103's "a future spawned from a component body is never polled"** is what an unmounted tree
  looks like from inside. The prints that established it ran under the same broken launch.
- **The focus mechanism works.** `focus-app` on a live run. That was F103's open question, and
  F104 could only test it below the DOM.
- **The keyboard's last inch is closed in the direction that matters**: the root holds focus, so
  a keydown lands on the element carrying the handler. F104 already proved everything from there
  to the store.

Found by asking the page rather than the operating system. It could talk back all along — a
WebView can reach `127.0.0.1`, so a few lines in `with_custom_head` and a listener in a scratch
directory answered in one run what three rounds of `xdotool` could not. The first thing it said
was `main-len0`; the thing that made it useful was running it a second time *without* my variable.

The lasting rule is in CONVENTIONS: a failure that only reproduces through your own harness is a
fact about the harness until you have run the thing the way its user would. F88 was the same
mistake in the other direction — a pass I could not reproduce. This was a catastrophic failure
I reported at length, in a commit message, in the plan and in the project's own documentation,
and it was my `env`.

### F108 — The keyboard, verified in the running application

The last inch. F104 proved everything from a dispatched event down to the database; F107 proved
the root holds focus in a live window. What was never joined up was a key press in the real
WebView producing a change in the real store.

It can be, and it needs no input tooling at all — the page can press its own keys. A few lines in
`with_custom_head`, an account pointed at the local IMAP fixture, and a listener in a scratch
directory:

```
/before|rows-2|focus-app
/after-j|rows-2|reader-a tricky body
/after-e|rows-1
inbox before: 2 → after: 1
```

The list draws two real rows from the database; the root has focus; `j` opens a conversation and
the reader shows its subject; `e` archives it, the list drops to one row, and the store agrees.
That is the whole chain — WebView keydown, focus, handler, `view::shortcut`, `op_for_shortcut`,
`apply_op`, SQLite, re-render — end to end in the program as it ships.

Two things made this reachable that were not before. The shell renders when it is not launched
with `GDK_BACKEND=x11` (F107). And `dispatchEvent(new KeyboardEvent(...))` from inside the page
sidesteps the whole question of whether a synthetic X event reaches a Wayland WebView, which is
what three rounds of `xdotool` were really stuck on — the wrong layer for the question.

A side lesson, cheap and annoying: two runs reported nothing because a listener from the previous
round still held port 18081 and was writing to a log file that had since been deleted. The
evidence looked like silence and was someone else's success. `ss -ltnp` names the holder; it is
worth asking before believing an empty file.

### F109 — The two journeys a mail client is for, run in the live application

No defect this round. Three things that had been argued from tests became things that were
watched happening, in the program as it ships, against a real database and a real IMAP server.

**New mail arrives when you press Sync.** Two rows in the list, one message waiting on the
server, the button clicked from inside the page:

```
/before|rows-2|button-Sync|disabled-false
/during|note-ada@example.test: 1 headers, 1 bodies, 0 queued operations settled, 0 sent
/after|rows-3|note-…1 headers, 1 bodies…
stored 2 → 3, and the new subject is at the top of the list
```

The button spawns a task from a click handler, that task runs, the pass fetches, the store gains
the message, `revision` bumps, and the list redraws. Every link in that chain had a test; none of
them had been seen joined up.

**Replying works from the keyboard.** `j` then `r`:

```
/start|rows-3
/composer|open|subject-Re: arrived while the window was open|to-Carol Shaw <carol@example.test>
/after-escape|composer-closed
drafts: "Re: arrived while the window was open" | "typed into the live composer"
```

The composer opens addressed to the right person with the right subject, typing in the body
reaches the shell's state, and `Escape` closes it *and saves* — which is the semantics F103
argued for and could not demonstrate. The typed text is in the drafts table afterwards.

Two process notes, both old lessons re-learned:

- The first four attempts at this reported nothing, for four different reasons, all mine: a
  listener from the previous round holding the port; `rm` on a log file the listener already had
  open, so it wrote to a deleted inode; a `const say` declared twice, which killed the whole
  script silently; and a fixture whose UIDs churn when a file is added while it is running. Only
  the last is about the program. **The fix was to stop typing sequences at a prompt and write the
  run as a script**, which is exactly what `scripts/live-tests.sh` exists for and what F88 already
  taught. Ad-hoc is where the errors live.
- A parse error in an injected script is invisible: nothing runs, including whatever would have
  reported the error. A separate first `<script>` that only installs an error listener turns
  silence into `SyntaxError: Cannot declare a const variable twice`.

### F110 — Forwarding was modelled and reachable from nowhere

`Draft::forward_of` has existed in `mail-domain` since phase 1, with its own tests for the `Fwd:`
prefix and for starting with no recipients. Nothing in `mail-app` ever called it. There was no
`forward` command, no button, no key. A mail client that cannot forward a message is not one, and
this one had the hard half written and the reachable half missing.

The missing half is the same one `Draft::reply_to` leaves to its caller: what the carried message
looks like. `compose::forwarded` writes the block every client writes —

```
---------- Forwarded message ----------
From: Bob <bob@example.test>
Date: Wed, 15 Nov 2023 at 07:13
Subject: a tricky body
To: me@example.test
```

— and then the original text *unmarked*. Not `>`-quoted: a forward passes the message on rather
than answering it, and a recipient who sees `> ` reads it as a reply. `To` and `Cc` are omitted
when empty rather than written as blank headers. The date is in the sender's zone, like every
other date since F98.

Recipients are a parameter, not a guess. Nothing in the original says who a forward should go to,
which is why `Draft::forward_of` leaves them empty — so `mailo forward <id>` without `--to` is
refused with the command that would work, rather than producing the F99 dead end: a draft the CLI
cannot finish and no command can repair. That also gives the CLI its first way to name recipients,
which F99 recorded as missing.

In the shell it is a row button and the `f` key, through a new `Composes` enum rather than a
third `ReplyScope` — a forward is not a reply with a different audience. Offered on every
conversation, because carrying a message does not depend on which folder it is in.

Proved on the wire, not only in tests. Through the real binary against the local `aiosmtpd`:

```
Subject: Fwd: a tricky body
To: "Bea" <bea@example.test>

have a look at this

---------- Forwarded message ----------
From: Bob <bob@example.test>
Date: Wed, 15 Nov 2023 at 07:13
...
```

One small thing the tests taught: the F97 "no run of two spaces" rule is about prose. The CLI's
draft summary is an aligned table and its runs of spaces are the alignment, so that assertion
belongs on sentences and not on everything a command prints.

### F111 — Attachments were stored at ingest and reachable from nowhere

`mail-runtime` writes every attachment to its own blob as a message arrives, and
`Message::attachments` has named them since phase 1. Nothing read that. No command listed them,
no pane showed them, and there was no way at all to get a file out of a message. A client people
are sent PDFs through is not one that can only display text.

`mailo attachments <message-id>` lists them; `mailo save <message-id> <n> [dir]` writes one out;
the reader shows what a message carries, named and sized, with the command that fetches it.

The part worth the care is the file name. It is a MIME parameter chosen by whoever sent the
message — not a fact about the file and not a promise about where it belongs — so
`../../../.ssh/authorized_keys` is a well-formed attachment name, and writing a file where it
asks is how a mail client hands someone else's machine over. `attach::safe_name` keeps one path
component: both separators stripped (a message written on Windows names its parts with
backslashes, and the same code on Windows would write two directories up), control characters
dropped (a newline makes a terminal print something other than what was written; a NUL truncates
the name at the syscall), `.` and `..` and empty replaced, and a long name shortened *keeping its
extension*, because the extension is what decides which program opens it. Saving never
overwrites: `invoice.pdf` twice is `invoice.pdf` and `invoice (2).pdf`, because the same supplier
sends one every month.

The listing shows the name that will be written, not the claim. A listing that said
`../../escape.pdf` would be describing something that does not happen.

Proved from the wire in: real RFC 5322 multipart bytes with a hostile `filename`, ingested the
way a sync ingests them, listed, saved, and the base64 decoded — so the MIME parser, the blob the
runtime writes, the listing and the name are the real ones rather than four fixtures agreeing
with each other.

### F112 — Every live IMAP test had been single-part, and the fixture could not do better

Trying to receive an attachment over the IMAP fixture killed the connection. The fixture declared
every message `isMultipart() -> False` and raised from `getSubPart`, and Twisted raises *inside*
BODYSTRUCTURE generation, so the client sees "connection closed unexpectedly" — which looks
exactly like a client bug and is not one. Every message the live IMAP tests had ever carried was
`text/plain`; attachments and HTML alternatives, which are most real mail, had never been on that
wire.

The fixture now parses each message with `email.parser` and answers `isMultipart`/`getSubPart`
from it. That exposed the next layer: **Twisted emits a space between the nested body lists**,
and `body-type-mpart = 1*body SP media-subtype` allows none, so the response it produces is not
conformant and our parser rejects it.

Checked before concluding anything, which is the F107 lesson: the spelling real servers send
parses fine.

| BODYSTRUCTURE | parses |
| --- | --- |
| single part | yes |
| nested parts with no space (RFC 3501, Dovecot, Gmail) | **yes**, attachment and all |
| nested parts separated by a space (Twisted) | no |

So the client is right and the fixture is not, and multipart over this particular fixture stays
out of reach — recorded rather than worked around, because the alternative is teaching the
fixture to emit what Twisted does not and calling that a test of the client.

What *is* ours is what the failure looked like. The error printed `nom`'s `Error { input: [42,
32, 49, …] }` — five hundred decimal integers where the answer is one line of IMAP — and that is
the only diagnostic the field ever sees. It now prints the response as text, escaped to one line
and truncated with a byte count.

### F113 — The envelope walk asked for eleven times the bytes it read

F95's sibling, one layer down. The IMAP envelope walk asked every server for

```
(UID FLAGS INTERNALDATE RFC822.SIZE ENVELOPE BODYSTRUCTURE)
```

and `parse_fetches`, the only thing that reads the result, takes `UID`, `RFC822.SIZE` and
`FLAGS`. `INTERNALDATE`, `ENVELOPE` and `BODYSTRUCTURE` — and on Gmail `X-GM-MSGID`,
`X-GM-THRID` and `X-GM-LABELS` — were requested on every walk and discarded. Headers arrive
separately from `BODY.PEEK[HEADER]`; nothing needed any of it.

Measured against the fixture with 22 messages, a third of them multipart with an attachment:

| asked for | bytes |
| --- | --- |
| `(UID FLAGS INTERNALDATE RFC822.SIZE ENVELOPE BODYSTRUCTURE)` | 10162 |
| `(UID FLAGS RFC822.SIZE)` — what is read | 922 |

Eleven times the response, for nothing. On the 2372-message maildrop this client was built for
that is roughly a megabyte per walk instead of a hundred kilobytes, every five minutes, on a
campus link.

The second cost is not bandwidth. **Asking for data is asking a server to produce it**, and
producing a BODYSTRUCTURE means parsing the whole message — so the request is also an exposure to
whatever bugs that path has. F112 is exactly that: a server that crashes generating a
BODYSTRUCTURE we would have thrown away. With the item list trimmed, the same 22-message
multipart mailbox that could not be synced at all now syncs completely — 22 headers, 22 bodies,
attachments stored, `mailo save` writing `report-18.pdf` out of a message that arrived over IMAP.
Not because the client got better at multipart, but because it stopped asking for something it
never read.

`X-GM-LABELS` comes back when labels are implemented, together with the code that reads it, which
is the only condition under which asking for something is worth the bytes.

A note on the measurement: the before-and-after could not both be taken through this client,
because the old item list cannot complete a sync of that mailbox. The 11× figure is measured
server-side with `imaplib`, which is the right place to weigh what a server sends anyway.

And the new error message earned itself immediately. The failed run printed

```
could not parse a response: * 3 FETCH (UID 200 FLAGS (\Seen) INTERNALDATE "14-Nov-2023 …
BODYSTRUCTURE (("text" "plain" ("charset" "utf-8") NIL NIL "7bit" 82 … (9543 bytes)
```

which says what happened. A week ago it would have been nine thousand decimal integers.

### F114 — Looking for an amplification in the part walk, and not finding one

A mail client takes input from strangers, so a small message that makes it do a large amount of
work is a denial of service anyone can post. `parse::bodies` copies every part's bytes with
`part.contents().to_vec()` and has no cap on how many parts it will take, and `assemble` then
writes each one to a blob — so the shape was worth measuring rather than assuming.

Measured, on a multipart message whose every part is a distinct attachment:

| parts | message | ingest | blob rows |
| --- | --- | --- | --- |
| 1 000 | 111 kB | 45 ms | 1 001 |
| 10 000 | 1.1 MB | 400 ms | 10 001 |

Linear, at about 40 µs a part, which is the property that matters: the work is bounded by the
size of the message, so the worst a sender can do is send a big message. A deliberately hostile
25 MB message — Gmail's limit — would carry perhaps 230 000 parts and take some seconds, once.

And the obvious flood costs nothing at all. Ten thousand parts with *identical* contents produce
**two** blob rows: the raw message and one shared part. The blob store is content-addressed, so
repetition — which is what a cheap attack is made of — collapses. Each of the ten thousand is
still a separate attachment with its own name; only the bytes are shared.

No defect, so nothing changed. The measurements are assertions now, because "we checked once" is
worth nothing later: a change that made attachment handling quadratic, or that gave every part
its own copy of the bytes, would fail `ten_thousand_parts_cost_ten_thousand_blobs_and_no_more` or
`ten_thousand_identical_parts_cost_one_blob`.

Two other things were looked at in the same pass and found sound, recorded so the next sweep can
skip them: the outbox backoff is `1s, 2s, 4s …` capped at an hour with `attempts.min(12)`, so it
cannot overflow or park a message for a century; and `Retry::Fatal` applies the undo patch rather
than leaving the user looking at a local state that will never become true.

### F115 — Snooze was modelled, queryable, and impossible to set

`Op::SetSnooze` has been in the domain since phase 1. `Filter::Snoozed` and `Filter::SnoozeDue`
have been answerable by SQLite for as long, with `SnoozeDue` deliberately resolved against `now`
at query time rather than frozen in at apply time. Nothing could set one: `op_for` returns `None`
for `OpKind::Snooze` because the op needs a payload, and no surface supplied it.

The payload is a time, and the interesting half is which times a person actually names.
`view::snooze_until` takes `later`, `tonight`, `tomorrow`, `weekend`, `monday`…`sunday`, `+90m`,
`+2h`, `+3d`, `2026-09-25` and `2026-09-25 14:30` — in the reader's zone and against a `now` the
caller supplies, for the reason every date in this project is: a function that reads the clock
decides its own test's answer, and one that assumes UTC sends "tomorrow morning" to the middle of
tonight for anyone east of Greenwich. "Tuesday" said on a Tuesday afternoon means the next one,
not an hour that has passed; "tonight" after seven means tomorrow evening. A property test asserts
every phrase resolves to something later than now — a snooze into the past is due the instant it
is made, so the conversation never leaves and the feature silently does nothing.

Nothing runs when a snooze expires and nothing needs to. The conversation returns to the inbox on
the stroke whether the client was awake or not, which is the right design for something that may
be asleep for a week — and it is the existing `SnoozeDue` design, not a new one.

`mailo snooze <thread> <when>`, `mailo wake <thread>`, `mailo list snoozed`, and a Snoozed place
in the sidebar next to Drafts, where a conversation that was put off is looked for.

### F116 — Two spellings of "the inbox", and only one of them learned

The feature above was finished, tested at every layer, and did not work. `mailo list` still
showed the snoozed conversation.

The shell asked `view::source_for`; the CLI's `list` built `Filter::InMailbox(role)` of its own.
Two definitions of the same place, and the day the inbox learned to hide a snoozed conversation
only one of them learned it.

Five tests passed over it, because **the test spelled the filter out itself**:

```rust
fn inbox() -> Filter {
    Filter::And(vec![InMailbox(Inbox), Not(pending_snooze())])   // written here
}
```

which tests the filter the test wrote, not the one the program uses. It is the vacuity rule of
CONVENTIONS §"An assertion that was already true proves nothing" in a new disguise — not an
assertion that is already true, but a *fixture* that restates the thing under test and therefore
agrees with it whatever the program does. The tell is the same: it could not distinguish a
working program from a broken one.

`view::place_filter(role)` is the single definition now, and both surfaces call it. The test asks
for it rather than restating it, and a new one asserts the sidebar's Inbox and the command's
inbox are the same filter.

Found by running `mailo snooze` and then `mailo list` — the two commands anyone would type in
that order, which is the whole of the technique.

### F117 — Pin, and the sort that could not carry it

The last of the three `op_for` returns `None` cases that had no surface. `Op::SetPin` and
`Filter::Pinned` had been in the domain and answerable since phase 1; nothing could set one.

`view::pin_op` resolves the payload — the direction from the conversation's current state, the
rank from the clock, so the most recently pinned sorts first among pins. `mailo pin <thread>`
toggles, `mailo list pinned` lists, a Pinned place sits in the sidebar, and `p` does it from the
keyboard. Unlike snoozing, a pinned conversation **stays in the inbox**: a pin is a note to
yourself about something you are still dealing with, not a change of where it lives. The two are
independent, and a conversation that is pinned *and* snoozed is still away — asserted, because
"pinning brought it back" is the obvious way to get that wrong.

**What this deliberately does not do is put pinned conversations at the top of the inbox**, which
is what the word means in most clients. `Sort` carries one property, and `Property::Pin` maps to
`ts.pin` — the *JSON text* of the value. That orders `{"kind":"rank",…}` before
`{"kind":"unpinned"}` by accident of spelling, ranks lexicographically so 10 sorts before 9, and
breaks ties on thread id rather than date. Sorting the inbox by it would put pins first and
scramble everything else.

Doing it properly means a multi-key `Sort`, which reaches the SQL builder, the keyset cursor
encoding and the `fit` ⟺ SQL parity proptest — the machinery migration 0003 was carefully built
around. That is a larger change than a pin is worth today, so the honest version shipped instead:
a place that holds them, which is expressible exactly and does what it says.

`Property::Pin` is unused outside a serde round-trip test. It is left alone and recorded here, so
that whoever wants pins-on-top finds the sharp edge before standing on it rather than after.

That leaves labels as the only modelled-and-unreachable operation: `Op::Label(LabelId,
Membership)` can be applied and `X-GM-LABELS` can be written to Gmail, but nothing ever reads
labels back — the envelope walk discarded them (F113) and `Ingest.labels` has always been empty.
Reading them is the work, and verifying it needs an account on a server that has them.

### F118 — A whole query algebra, reachable through one word

The largest instance of the shape this project keeps turning up. `mail-domain` has had
`From`, `To`, `Subject`, `Date`, `HasAttachment`, `Read`, `Starred`, `Pinned`, `Snoozed`,
`InMailbox` and `And`/`Or`/`Not` over all of them since phase 1 — every one of them proptested
against the SQL that answers it, in `mail-store/tests/parity.rs`. Every search the application
could make was `Filter::Text(Contains(the whole line))`.

On a maildrop of 2372 messages, "from Bob about the invoice, some time last year" is a question
the store could already answer and nobody could ask.

`query::parse` is the missing sentence. The vocabulary is the one every mail client uses, because
the point is to be guessable rather than clever:

```
from:ada  to:bob  subject:lunch     is:unread is:read is:starred is:pinned is:snoozed
in:inbox  in:archive  in:sent       has:attachment
before:2026-01-01  after:2025-12-25 -from:newsletter      "an exact phrase"
```

Terms join with `And` — each word you add finds less, which is what everyone expects. `Or` is
deliberately absent: it reads ambiguously beside `-` and nobody types it.

Three decisions worth the words:

- **An unknown term is searched for, not refused.** A box that rejects what is typed while it is
  being typed is unusable, so `frm:ada` becomes text and finds nothing. What it must never do is
  become `Filter::All` and show the whole mailbox as though it had matched — asserted.
- **Dates are read in the reader's zone.** Midnight on the 22nd in Taipei is 16:00 on the 21st in
  UTC; read as UTC, `after:2026-09-22` would include eight hours of somebody else's day.
  `before` is exclusive and `after` inclusive, matching `DateRange`'s own `>= from` and `< to`:
  a day named is a whole day, and "before the 25th" must not include it.
- **One parser, both surfaces.** The shell's box and `mailo search` call it, so `from:ada` means
  one thing — F116's lesson, applied before rather than after.

Tested in two halves, because a filter that parses and does not select is worse than no filter:
what the parser builds, and what the store returns for it against real ingested mail. And through
the binary: `from:billing`, `-from:billing`, `subject:invoice is:unread`, `after:2026-01-01`,
`before:2026-01-01` each return what they should, and `frm:billing` finds nothing.

`label:` is absent on purpose. Nothing populates labels — the envelope walk discarded Gmail's
(F113) and `Ingest.labels` has always been empty — so the term would parse, select nothing, and
look like a bug in search rather than the gap it is.

### F119 — Labels, the last thing that was modelled and never arrived

I said twice that this needed an account on a server that has labels. Both times were wrong, and
for two different reasons.

The first: `crates/mail-proto/tests/traces/imap/gmail_fetch.trace` is **derived from a real Gmail
capture**, and contains `X-GM-LABELS (\Inbox "travel")`. The bytes were in the repository the
whole time.

The second: I claimed the chain was unverifiable end to end. The IMAP test servers in
`mail-runtime/tests` are hand-written and in-process, and the backend is sans-I/O — "the server"
can be a slice of bytes. Nothing about verifying this needs a socket, let alone Google's.

What was missing, once the excuses were gone, was the assignment. `Ingest.labels` has upserted
label *definitions* since phase 1 and nothing ever said which message carried which: per-message
labels ride on `Fetched.message.labels`, which `absorb` fills from parsed RFC 5322 bytes, and a
parsed message has no Gmail labels in it. So `Ingest` gains `label_names: Vec<(RemoteRef,
Vec<String>)>` — names, because the protocol has names and the store owns ids, and parallel to
`flags` because a label change is far cheaper than refetching a message.

Three decisions worth the words:

- **The list is complete, not additive.** It is what the message is labelled *now*. A client that
  only ever adds accumulates labels the user deleted years ago and no later sync takes them off,
  so the store applies the difference in both directions — and only the difference, because
  writing every membership every pass is the waste F95 and F113 were about. Asserted: a second
  identical sweep writes no changes at all.
- **Gmail's own names are not labels.** `\Inbox`, `\Sent`, `\Draft`, `\Spam`, `\Trash`,
  `\Important`, `\Starred`, `\Muted` are `mailbox`, `read` and `star` here already; carried
  through they would appear on every row and the user could not remove them. Anything beginning
  with a backslash is dropped rather than a fixed list matched — Gmail has added to that set
  before, and a client that enumerates it inherits the next name as a label.
- **Names are modified UTF-7.** A Chinese label arrives as `&Ux1Tgg-` and must not be shown that
  way; `mutf7::decode` was already there for mailbox names (F44) and is the same answer here.

`X-GM-LABELS` is back in the fetch item list, which is exactly the condition F113 set: ask for
what something reads, and only that. `ENVELOPE`, `BODYSTRUCTURE`, `INTERNALDATE` and the other
`X-GM-*` attributes stay gone.

`label:travel` now works in search, through the same parser both surfaces use, with the name
resolved by the caller so the parser stays pure.

Checked at every layer that exists: the parse, against the capture's own shapes and against
quoting, escaping and non-ASCII; the store, that a name becomes a row, that the same name twice
is one label, that a dropped label comes off, that an unchanged list writes nothing, and that a
label row for a message this client has not fetched is skipped rather than fatal; and the seam
between them in `mail-runtime`, which is the only crate allowed to hold both — a survey's bytes
in, a thread found by `Filter::HasLabel` out.

What is *not* proved is a real Gmail account, and nothing here can be. What it would exercise
beyond these tests is whether Gmail's wire matches its documentation, which is exactly what the
capture in `traces/` was recorded to answer.

### F120 — Measuring the query shapes F118 made reachable

A query language that produces shapes the SQL builder answers by reading the whole mailbox is a
slower search than the one it replaced. F118 turned `from:ada is:unread after:2026-01-01` into a
three-clause `Filter::And` that nothing in this application had ever asked the store for, so the
shapes are measured rather than assumed. Ten thousand messages:

| search | |
| --- | --- |
| `subject:widgets` | 0.76 ms |
| `is:unread` | 0.82 ms |
| `after:…` | 0.79 ms |
| `-from:s1` | 0.79 ms |
| `from:s1` | 4.9 ms |
| `is:starred` / `has:attachment` / `is:pinned` / `is:snoozed` | 5.9–7.7 ms |
| `from:s1 is:unread` | 9.8 ms |
| `from:s1 widgets after:… is:unread` | 28 ms |

No pathology. The slowest thing the language can build is four clauses at 28 ms, which is well
inside the range where a search box still feels immediate.

Two of those numbers are worth understanding rather than optimising. `from:s1` costs six times
`subject:widgets` because the fixture's subject matches every message and its sender matches one
in ninety-seven — filling a page of fifty takes more rows, which is the query doing its job. And
the four that find *nothing* cost 6–8 ms because proving a mailbox contains no starred message
means looking at all of it; there is no index on `star`, `pin` or `snooze`. At this size that is
the right trade: an index would have to be maintained on every write to save seven milliseconds
on a search nobody runs twice. Recorded so the decision is visible if the mailbox ever gets large
enough to change it.

A second assertion pins the property that makes a query language safe to offer at all: **adding a
term must not cost more than the terms it narrows**. One clause against three, and the three are
no slower — a builder that evaluated each clause independently and intersected afterwards would
fail that, and it is the shape a query language invites.

### F121 — Checking the new ingest path against the rule the old ones follow

F119 added a second place where server truth is written into the store, and the design has a rule
about that: **server truth is the base, and anything the user did that the server has not
confirmed goes back on top of it.** A new writer that ignores it silently undoes the user's last
action every time a poll lands.

The label path follows it, and only because of where it sits: step 3b, before the re-layer at
step 5, marking each affected thread `touched` so the re-layer visits it. That is correct by
construction rather than by intent, which is exactly the kind of correctness that survives until
someone moves the block.

So it is asserted now, in the suite that already holds this rule for flags:

- A label the user added a second ago survives a survey that predates it.
- A label the user *removed* does not come back — the direction that is easy to get wrong,
  because the server's list is complete and looks authoritative, so a naive apply puts back
  precisely what the user took off.
- Once the send settles, the server's list wins again. Otherwise a confirmed change would be
  re-layered for ever and another client could never remove the label.

Removing the two lines that mark the thread touched fails the first two and leaves the third
passing, which is the right signature: the re-layer is what the first two are about.

No defect. The value is that the rule is now checked for labels rather than reasoned about, and
the next person to add an ingest path has two more examples of what it has to satisfy.

### F122 — Two writers, and a five-second decision nobody had made

`mailo sync` in a terminal while the window is open is an ordinary thing to do, and so is a
scheduled sync overlapping a manual one. WAL is what makes a *reader* concurrent with a writer,
and does nothing for two writers: SQLite serialises them, and the second either waits or is told
`database is locked`.

Nothing in this codebase set `busy_timeout`, so the question was what the default is. Probed by
holding a write transaction open on one connection and writing from another:

```
the second write was refused after 5.00397673s: database is locked
```

Five seconds, then a plain error. That is `rusqlite`'s default rather than SQLite's — SQLite's own
default is **zero**, an immediate failure — so the behaviour was right by inheritance.

Right by inheritance is the shape of F121 one layer down: a default that nothing names is a
behaviour nobody notices changing. It is set explicitly now, with the reason beside it, and
asserted on both the file and in-memory paths because they are opened by different code. The
`journal_mode` and `synchronous` pragmas are asserted too — a database that fell back to `delete`
journalling would block every read behind every write, which is the shape of "the window freezes
while it syncs" and would not otherwise fail any test.

Five seconds is the right number against transactions that are one fetch batch long, which the
scale tests measure in milliseconds.

Two other things were checked in the same pass and found sound. `write_ingest` is one transaction
from the UIDVALIDITY reset to the cursor, so a failure halfway rolls back rather than leaving a
cursor advanced past messages that were never stored — which would be lost mail. And a rolled-back
ingest leaves its blobs behind, which is not a leak: blobs are content-addressed, so the retry
writes the same hashes and reuses them.

### F123 — Every test in the project used one account

Someone with a work address and a personal one has two accounts, and that is ordinary. Every
suite in the repository — nine of them touching the store — creates exactly one account. Two
accounts was not an edge case that had been decided against; it was a case nobody had looked at.

The things that can only go wrong with two are the ones worth checking, so they are checked now:

- **The unified inbox is `InMailbox(Inbox)` with no `Account` clause** and holds both, which is
  the plan's claim about places being saved filters, asserted rather than assumed. A list that
  silently showed one account is the kind of thing a user notices when mail goes missing.
- **A message that arrives on both accounts is two threads.** A mailing list both addresses are
  on, or one that forwards to the other. Merged, it would be one thread whose messages live on
  two accounts — and archiving it would have to act on two servers with two credentials, which no
  operation here can do. Threading looks up `In-Reply-To` scoped by account, so a reply arriving
  on one attaches to that one's copy; asserted by message count on each side.
- **Archiving one account's copy leaves the other's alone.**
- **One label name on two accounts is two labels**, which `UNIQUE (account, name)` gives and
  nothing had relied on.
- **A count adds up the accounts it is asked for**, one or both.

All seven passed first time, which is the answer: the store's multi-account behaviour is sound.
What was *not* sound was above it.

**`label:travel` searched one account.** `labels_named` took the first label with that name and
`Filter::HasLabel` takes one id, so with "travel" on both the Gmail and the campus account the term
matched whichever account was created first — a wrong answer that looks exactly like an empty
one, which is the worst kind. The resolver returns every match now and the parser builds
`Or([HasLabel(a), HasLabel(b)])`, because someone who types a word means the word. A name nothing
bears still becomes text rather than `Filter::Nothing`, so it reads as a typo rather than as a
label with no mail.

### F124 — Every message this client had sent went out unsigned

Following the seam F123 opened — *what does every test quietly assume?* — to the next assumption:
one identity per account, with nothing set on it.

`Identity.signature` has been a column since phase 1 and is loaded into the struct on every
`identity_of`. Nothing has ever read it. No sending path appended one, no composer showed one,
and no command could set one, so the field was decoration on a struct.

`compose::signed` puts it beneath what was written, and `mailo signature <address>` sets it from
stdin (`--clear` takes it off). Three decisions:

- **At compose time, not at send.** Like the quoted material beside it, so the user can see it,
  edit it, or delete it for one message. A signature that cannot be removed from a particular
  reply is worse than none.
- **Above the quote, below the body.** Which is where every client puts it and where a reader
  expects it.
- **The delimiter is `"-- "` — two hyphens, a space, and nothing else.** RFC 3676 §4.3 names that
  exact string and every client that trims a signature when quoting looks for it; `--` without
  the trailing space is a different line and gets quoted back at people for the rest of the
  thread. An account with no signature must not gain a bare `-- ` either, since other clients
  read it as "everything below is a signature" and hide it.

On the wire, through the real binary against the local `aiosmtpd`:

```
one o'clock suits

--=20
Ada Lovelace
Analytical Engines Ltd

On Wed, 15 Nov 2023 at 07:13, Bob wrote:
```

`--=20` is the delimiter in quoted-printable, and it is the *right* answer rather than a
surprising one: a trailing space at the end of a line is exactly what transport strips, so it has
to be encoded or the delimiter arrives as `--` and stops being one. `mail-builder` did that
without being asked, which is worth knowing the next time a whitespace-significant line is added
to an outgoing message.

Whitespace is not a signature: a file of blank lines, or `< /dev/null`, is stored as NULL so that
"has one" is a single question rather than two that can disagree.

### F125 — Full-text search cannot find a Chinese word

The next assumption after F123's "one account" and F124's "nothing set on the identity": every
message in every test is in English.

Measured against one real-shaped message — a campus IT notice, subject in an RFC 2047
encoded word, body in UTF-8:

```
subject as stored: "【重要】校園郵件信箱系統維護"

  search "校園郵件信箱系統維護"        -> 1 hit     the whole run
  search "校園"                       -> 0 hits    campus
  search "郵件"                       -> 0 hits    mail
  search "維護"                       -> 0 hits    maintenance
  search "暫停服務"                   -> 0 hits    service suspended
```

`unicode61` classifies ideographs as token characters and Chinese is written without spaces, so
an unbroken run of them is **one token**. The whole subject is a single word, and the only query
that finds it is all fourteen characters of it. For a mailbox that is substantially Chinese, full-
text search does not work.

What *does* work, and is worth knowing: the field clauses are `LIKE '%needle%'` on a column rather
than FTS, so `subject:校園`, `from:` and `to:` find Chinese correctly. The query language added in
F118 gave Chinese a working search path by accident.

The obvious fixes were checked rather than assumed, and the two obvious ones do not work:

- **`trigram`, SQLite's substring tokenizer.** It answers queries of three characters or more.
  Chinese words are overwhelmingly *two* characters — 校園, 維護, 服務 — so it fails on the
  common case. Measured directly: a trigram index finds `校園郵` and does not find `校園`.
- **Transforming the text before it is indexed**, by segmenting CJK runs into per-character or
  bigram tokens. `messages_fts` is an external-content table populated by SQL triggers straight
  from the `messages` columns. Rust never touches the indexed text, so there is nowhere to put
  the transformation without giving up external content and writing the index rows from Rust.

A real fix is therefore: drop `content = 'messages'`, write `messages_fts` rows from Rust with
CJK runs segmented, segment the needle the same way, and mirror the rule in `filter.rs`'s
`fts_tokens` so the `fit` ⟺ SQL parity proptest still holds. That is a change across three crates
and a migration that reindexes, and it is the right change — but it is a redesign of the search
index, not a patch, and starting one at the end of a long round is how the index ends up
half-rebuilt.

**Fixed in F126**, immediately below. What follows was written before that and is kept because
the measurements and the two dead ends are why the fix has the shape it does.

So it is written down instead, with the measurements, the two dead ends and the shape of the
answer. Four assertions pin the present behaviour — including that the encoded-word subject is
decoded on the way in, that the field clauses *do* find parts of a Chinese phrase, and that
English is unaffected. The one that says a partial Chinese search finds nothing is written to
fail the day the index learns to segment, which is the day this file should change.

### F126 — Segmenting the index, so Chinese is searchable

F125's fix, which the previous round described and declined to start. Declining was the wrong
call: it is a defined change with a proptest for a safety net, and "large" is not "someone else's".

**The rule.** A run of ideographs or kana becomes its overlapping bigrams: `校園郵件` is `校園`,
`園郵`, `郵件`. Bigrams rather than single characters because separate characters ANDed would
match any message containing 校 and 園 anywhere — the looseness that is acceptable between two
English words and useless between two halves of one Chinese word. Overlapping, so a needle
starting mid-word still matches. A run of one character is that character, so an isolated
ideograph stays findable. Hangul is deliberately excluded: Korean is written with spaces and
already tokenizes correctly.

**Where it lives.** `fts_tokens` in `mail-domain` and its mirror in `mail-store`, which the
`fit` ⟺ SQL parity proptest already forces to agree — and `sql::indexable`, which is *built from*
`fts_tokens` rather than written beside it, so the index and the query are the same function by
construction.

**What had to change underneath.** A tokenizer cannot be asked to segment, so the indexed text
must be segmented before it reaches one — and the index could not read the message columns any
more. Migration 0004 gives `messages` an `fts_text` column that Rust fills with exactly the
tokens the query side will ask for, and points `messages_fts` at that one column. External
content is kept, so the index still stores no duplicate text.

Three things this turned up, none of which review would have:

- **The body pass has its own update path.** `fill_body` writes `body_text` directly rather than
  going through the upsert, so a message fetched headers-first was indexed with a subject and a
  sender and never with its text. `a_whole_sync_over_a_real_socket_lands_mail_in_the_store`
  caught it — an English search, broken by a change about Chinese.
- **Telling an external-content index to remove an entry that was never inserted corrupts it.**
  Every row predating the migration has `fts_text IS NULL` and no entry, and the backfill is
  exactly an update of those rows, so the first thing that happened was `database disk image is
  malformed`. There are two update triggers now, split on `old.fts_text IS NULL`.
- **SQL cannot compute the new column**, which is the whole point of the migration, so
  `from_connection` backfills rows that have none. Idempotent and skipped by one count on a
  database that has already done it. `mail_that_predates_the_segmented_index_is_findable_afterwards`
  builds a version-3 database by hand, with English and Chinese in it, and asserts both are
  findable after the upgrade.

Through the real binary, against a campus notice with an RFC 2047 subject and a base64 body:

```
search 校園      →  【重要】校園郵件信箱系統維護
search 郵件      →  【重要】校園郵件信箱系統維護
search 維護      →  【重要】校園郵件信箱系統維護
search 暫停服務  →  【重要】校園郵件信箱系統維護     (in the body)
search lunch     →  lunch on friday                  (English is unaffected)
search 臺北      →  nothing matches "臺北"
```

`臺北` is the assertion that matters as much as the others: bigrams must not become a substring
match on everything. `園校`, `件維` and `維郵件` are checked too — reversed, and one character
from each end of the run.

Search over ten thousand messages is 47 ms, inside the same budget as before.

### F127 — Only the inbox was ever fetched, and nothing said so

The next assumption after "one account" and "one identity": one mailbox. `sync::one` built
`MailboxRef { path: "INBOX" }` and that was every folder this client had ever looked at.

For an IMAP account that means: mail sent from a phone or from webmail never appears in **Sent**,
which shows only what this client sent; mail archived elsewhere vanishes from the inbox and turns
up nowhere. Neither `plan.md` nor `ORCHESTRATION.md` mentioned it. POP3 has one mailbox by
construction, so this only ever bit the IMAP accounts — which are the Gmail one and the two
Microsoft tenants.

Two things had to change. `absorb` hard-coded `mailbox: MailboxRole::Inbox` for every message
whatever folder it came from — invisible while one folder was synced, and the *first* thing to go
wrong when a second is, because mail the user sent would be listed among the mail they received.
It now takes a `Destination`: the mailbox and what that mailbox is for. And a pass loops over the
mailboxes rather than assuming one, inbox first and in full, with a failure on a later folder
reported by name rather than losing the mail the earlier ones fetched.

**`Archive` is deliberately not fetched, and finding out why is the useful part of this round.**
A test asserting that a message in INBOX and in All Mail — the ordinary Gmail case — ends up in
both places *failed*: it is stored once, correctly, and keeps the role of whichever folder saw it
first. `Message.mailbox` is one role, so it cannot be in two. A pass over All Mail would
therefore mark inbox mail as archived and the user's inbox would empty itself. Fetching Sent is
safe because a message in Sent is not also in the inbox.

Making Archive safe means a message carrying a set of mailboxes rather than one, which is a
domain change and not a sync one. The limitation is now a test — `a_message_in_two_folders_is_
stored_once_and_keeps_the_first_role` — so it is written down somewhere that fails if the model
changes underneath it.

`Spam` and `Drafts` are absent for their own reasons: downloading the spam folder to populate a
place nobody opens costs a first sync twice over, and the server's drafts are other clients'
half-written mail, which would collide with this one's outbox.

And because "why is my Sent folder empty" is a question a user asks, `mailo account list` now
answers it:

```
someone@gmail.com            no credential stored
                             syncs INBOX, [Gmail]/Sent Mail
```

### F128 — Mail only arrived when you pressed a button

`AccountEngine::watch` has existed since phase 3 — IDLE where the server offers it, a poll
interval where it does not — and nothing ever called it. There was no timer anywhere in the
shell. A window left open all day fetched nothing; mail arrived when the user pressed Sync and at
no other time. For something meant to sit on a second monitor, that is the difference between a
mail client and a mail viewer.

The loop is an ordinary `use_future`, which is worth saying because F103 concluded that a future
started at mount never runs. That conclusion was taken under the broken launch F107 found, and
F107 suspected as much; a test now settles it — `a_future_started_when_a_component_mounts_does_run`
— before anything was built on the answer.

**The rule the whole thing turns on is what happens when the credential is wrong.** Five minutes
is 288 attempts a day. With a password the server has already refused, that is 288 *failed
logins* a day against the user's own mail server, which is how an account gets locked — the exact
hazard that has kept every experiment in this project pointed at a fixture rather than at a real server.
Backing off is not enough: half-hourly is still 48 a day. `view::next_sync` returns
`NextSync::Wait` for a rejected sign-in, the loop stops, and the Sync button still works, so
someone who has fixed the credential is one click from finding out.

That classification has to survive the trip. By the time a failure reaches the user it is prose,
and prose is not something a loop can safely decide on — so `SyncReport` gained `needs_reauth`,
set where the error is still a typed `Retryable`, and `sync::run` returns a `Ran` that carries it
beside the text.

Everything else doubles from the account's own interval and stops at half an hour. Four rules,
each a test:

- A rejected sign-in stops the loop at any failure count, and says how to fix it.
- A good pass goes again at the interval, and forgets the failures before it.
- A transient failure doubles: 5, 10, 20, then the ceiling.
- **A machine asleep for a week does not come back to a short wait.** `1 << n` wraps, and a
  wrapped shift produces a *smaller* number — which would turn a long outage into the fastest
  polling the client ever does. Checked at 31, 32, 33, 64, 1000 and `u32::MAX`.
- A long configured interval is never *shortened* by the ceiling: an hourly POP3 account must not
  poll more often when it is failing than when it is working.

The interval is the shortest any account asks for, read from `WatchMode::Poll` rather than
written down twice. An `Idle` account is polled like any other until something actually holds an
IDLE connection, because treating "IDLE is available" as "no polling needed" would mean an IMAP
account never syncing at all.

Watched in the live window, with the interval turned down and nobody touching anything:

```
stored before the window opens: 0
  /rows-2|note-ada@example.test: 0 headers, 0 bodies, 0
  /rows-2|note-ada@example.test: 2 headers, 2 bodies, 0
stored after 12s with nobody pressing anything: 2
```

### F129 — A wrong password would have been retried 288 times a day

F128 built the poll loop's safety rule on `Retry::NeedsReauth` and never checked that anything
produces it. It does — on two servers. `classify` in `imap.rs` decided what a `NO` meant by
searching its text for `[AUTHENTICATIONFAILED]` or `invalid credentials`, which is Dovecot's
wording and Gmail's, the two that were to hand. Exchange, Courier, UW-imapd and Zimbra all answer
a wrong password with a bare `NO LOGIN failed.`

That fell through to `Refusal::Permanent`, so `retry()` said `Fatal` rather than `NeedsReauth`,
`SyncReport.needs_reauth` stayed false, and the loop scored the pass as a clean success and came
back in five minutes. For ever, with the same wrong password. The rule F128 is *about* was off on
most of the servers it was written for, and every test passed because every test used a wording
from the table.

A `NO` is a reply to a command, and the command was one field away at the call site. Sign-in
refusals are now decided by which command was refused — as POP3 has always done, and as SMTP
does by reply class, with 4xx correctly left transient so a temporary auth failure is not blamed
on the password. Only IMAP read the prose. The text is still consulted first for rate limiting,
which is the one thing only the text says.

Six real servers' wordings are enumerated in a test, all of which must reach `Retry::NeedsReauth`;
a refused `SELECT` must not. End to end, a pass against a server sending the bare wording sets
`Ran.rejected`. The control is what makes that worth anything: an *unreachable* server must not
set it, or one flaky minute of network would stop the loop until the user next noticed.

A bare `FAIL` in a protocol trace now accepts any error, because a test about which error a
response produces must not name the answer in its own transcript.

### F130 — "Wait an hour" was computed, then dropped on the floor

`ProtoError::Throttled` has always become `Retry::After`, carrying the server's own wait or an
hour where it names none. The comment on it says plainly that backing off is the entire remedy
and that hammering lengthens the lockout. Nothing downstream read the number. The pass folded the
error into its prose, `rejected` stayed false, and the loop treated it as a success and came back
in five minutes — twelve times an hour into a server that had just said stop.

`SyncReport` now carries `hold`, the longest wait anything asked for, and `SyncReport::saw`
gathers both facts a caller must act on so the three sites that see a `Retry` cannot drift apart
— which is exactly how F129 happened. `view::Passed::Throttled { wait }` puts it in front of the
decision, and the wait is honoured in full: `BACKOFF_CEILING` deliberately does not apply, since
capping an hour at thirty minutes means knocking twice inside the window the server asked for.

Rejection still wins over a hold when a pass reports both. One asks the user to act; the other
only asks for time.

The control is again the half that could have gone wrong quietly. `Retry::After` is also produced
by ordinary sixty-second backoffs, and a throttle resets the consecutive-failure count — so if an
unreachable server set `hold`, it would be polled flat for ever instead of climbing to the
ceiling. It does not, and a test says so.

### F131 — `label:` worked in the terminal and found nothing in the window

`label:` is the one search term that needs the store, so it arrives as a resolver and the parser
stays pure. `mailo search` passed one. The window called `query::parse`, a convenience wrapper
that passes a resolver knowing no names at all — so every `label:` typed into the search box
resolved to nothing, fell through the unknown-term rule that exists so a half-typed word still
searches, and became a full-text search for the literal string `label:travel`. No results, no
error, and the same query working one surface over. A whole wave of work on Gmail labels,
reachable only from the command line.

`Shell` carries the label index as data, so `query` stays a pure function of the shell, and `App`
refills it on every revision — a label that arrives in a sync is searchable without a restart.
`query::known_labels` and `query::named` are the single definition both surfaces use.
`query::parse` is deleted rather than fixed: a caller with no index has to write `&|_|
Vec::new()` and see itself do it.

**The tests drive the real `App`.** A mutation recorder finds the search box's element id,
`HasFormData` joins the test event converter, and the input event goes to the component tree with
a real store behind it. A test that set `Shell::search` directly would build the index itself and
prove nothing about whether the window ever does — which is how this shipped. Disabling the
effect fails the test; the control, a name nothing bears, still finds nothing.

Two of this project's own rules caught me while writing them. `page.contains("hi")` matched
`white-space` in the stylesheet, which is *Substrings are not tokens* in a test of its own making.
And seeding the label by writing `labels` and `message_labels` by hand looked equivalent to a sync
and was not: `Filter::HasLabel` reads `thread_summary.labels`, a materialized union that only the
ingest path rewrites, so the rows were there and no search could see them.

**Then it was run the way its user runs it, which is the part I first got wrong.** The window
was launched, the screen was locked, and the round ended saying the shell had not been seen. That
was a claim about my eyes, not about the program: WebKit runs and executes script whether or not
a compositor is showing the pixels to anyone. `scripts/live-window.sh` now seeds a real store,
starts the real binary, and has the page type into its own search box and post what the list
contained:

```
{"stage": "mounted",                  "subjects": ["flight to taipei", "the invoice"]}
{"stage": "after label:travel",       "subjects": ["flight to taipei"], "typed": "label:travel"}
{"stage": "after label:nosuchlabel",  "subjects": []}
{"stage": "after clearing",           "subjects": ["flight to taipei", "the invoice"]}
```

Real WebKit, real store, a real `input` event on the real search box. The probe reaches the head
through `$MAILO_PROBE`, which `cfg(debug_assertions)` keeps out of any release build — a mail
window that runs script from an environment variable would turn control of the environment into
reading every message in the store and sending it somewhere, and that is a real step up from what
setting a variable otherwise buys.

The technique had been rebuilt from scratch three times and thrown away each round. It is a
script now.

### F132 — `sync` told a Gmail account to go and find a password

Phase 6's criterion is living with it, which is not something that can be done from inside the
repository. What *is* inside it is the on-ramp: whether someone who sits down to start using this
can get from an address to mail arriving. So the on-ramp was walked, with two real addresses —
one Gmail, one campus POP3 — against a clean data directory.

`mailo account add` gets it right. It knows the Gmail account is OAuth and says so. Then:

```
$ mailo sync
you@gmail.com: no credential stored. Run: MAILO_PASSWORD=… mailo account add <address>
you@university.edu:     no credential stored. Run: MAILO_PASSWORD=… mailo account add <address>
```

One hardcoded sentence for every account, ignoring the `AuthPlan` sitting in the row it just
read. For the campus account it is exactly right. For Gmail it is advice that **cannot** work — Google stopped
accepting passwords for IMAP in May 2022 — and someone who follows it makes a failed sign-in
against Google with a credential that was never going to be accepted. That is the hazard this
whole project has been careful about, and the client was printing instructions for it. The
command that got it right and the command that got it wrong disagreed one line apart.

`view::no_credential(address, auth)` is the one definition now, and `sync` uses it. It also names
the account instead of the literal `<address>`: advice that has to be edited before it can be run
is advice someone gets wrong at the point they are least equipped to notice. `--microsoft` is
carried into the suggested command, because the address alone does not reproduce a managed-tenant
account.

`mailo account list` was the third surface, saying "no credential stored" for the OAuth account —
milder, but it still reads as *find a password*. It says "not signed in" now, from
`sync::auth_by_account`, so all three agree.

And the instruction that was left for the user to research is now written down. "Register an
installed application with the issuer" meant: a Google Cloud OAuth client ID of type "Desktop
app", with this address added as a test user while the consent screen is in Testing — the step
most people miss — or, for Microsoft, an Entra app registration with a public-client redirect. It
names the product and the credential type rather than a path through a menu, because console
navigation is rewritten far more often than either.

The `\`-continuation hazard from F97 turned up twice more while writing these strings: `cargo
fmt` folded the continuations and the indentation went into the message, so the user would have
read "needs a client&nbsp;&nbsp;&nbsp;&nbsp;id". Both messages are `concat!` of whole lines now,
which formatting cannot reach. Note that `concat!` hides inline format captures from the
compiler, so the arguments have to be named.

### F133 — The window's first run, checked, and a fixture that lied

F132 fixed three surfaces. The fourth is the one a new user actually looks at: the window, opened
after `mailo account add` and before a client id exists — which is where someone will sit for as
long as it takes them to register one. `scripts/live-window.sh` exists to answer that, so it was
asked.

The first answer was alarming. The list said "Nothing here." and the sidebar said
**"you@gmail.com: no capabilities recorded. Something has gone wrong with setup."**
Nothing had gone wrong: the account had been added exactly as instructed, and the user was off
doing the thing they were told to do.

It is not a defect, and the reason matters more than the finding would have. That state was
invented by the fixture. `seed_unsigned` wrote an `accounts` row by hand and left `account_caps`
empty; `mailo account add` always writes capabilities from the preset, as the store it produced
confirms. The guard is correct and fires only on a genuinely corrupt row. Checking what the real
command produces, before reporting, is the whole of the difference between this paragraph and a
wrong finding — the third time this project has caught a fixture restating the thing under test.

Run against a store `account add` actually made, the window is right:

```
list          Nothing here.
sidebar       you@gmail.com: not signed in yet. This account uses OAuth (Google),
              which needs a client id registered with the issuer — a password will not work. Run:
                  MAILO_OAUTH_CLIENT_ID=… mailo account add you@gmail.com
              you@university.edu: no credential stored. Run:
                  MAILO_PASSWORD=… mailo account add you@university.edu
```

F132's fix reaches the window for free, because the window and the CLI both read `sync::run`.
That is what one definition buys, and it is the answer to the question F131 and F132 both asked.

The lying fixture is deleted rather than kept, and `seed_live.rs` and `live-window.sh` both now
say that a store in some other state is made by running the command, not by writing the rows.

### F134 — Every "re-run this command" ended in a database error

Found by a user, on the first real attempt, which is the argument for phase 6's criterion in one
line.

`mailo account add` is the only way to supply a credential, and every message this program prints
about a missing one says to re-run it: F132's `not signed in yet … Run: MAILO_OAUTH_CLIENT_ID=…
mailo account add <address>`, the password equivalent, and the line `account add` itself prints
after creating an OAuth account. The address column is `UNIQUE` and the insert was a plain
`INSERT`, so the second run failed with

```
cannot save the account: UNIQUE constraint failed: accounts.address
```

A raw SQLite error, as the entire response to following the program's own instruction — and a
dead end, because no other command finishes a half-configured account either. Anyone adding an
OAuth account hits it: the first run creates the row and then asks for a client id, so the
*first* thing they are told to do is the thing that cannot work. This is F99 again, which was a
`reply` offering a `send` no command could make work, and it is the same lesson: advice that does
not work when followed is worse than none.

Adding an address that is already there now updates it and says `updated` rather than `added`.
Two things had to be got right beyond the upsert:

- **The account id is the keyring key.** `AccountId::generate()` ran before the insert, so an
  upsert keyed on address alone would have left a new id in the row and orphaned a credential
  already stored — a working account would quietly stop working. The existing id is read back
  and reused.
- **`mailo signature` writes to the identity row.** Rebuilding the identity on each run would
  have silently deleted a signature, and a display name with it — the sort of loss nobody notices
  until it has gone out on a week of mail. `default_identity` reads the stored one back, and the
  insert is `ON CONFLICT DO NOTHING`.

The plan and the expected capabilities are refreshed, since a preset may have learned a better
host; `created_at` is not.

### F136 — Nothing ever called the flag sweep

Found by using it. The first real Gmail account synced, and all 338 messages were unread —
including 138 the user had *sent*, which Gmail always marks `\Seen`. That is not a proportion to
argue about; it is a proof.

`AccountEngine::sweep` has existed since phase 3. It fetches flags, Gmail's labels and the list
of messages that have gone, on three separate clocks, and it is thoroughly tested. Every one of
its callers was a test. `sync::pass` ran `sync`, `fetch_bodies` and `drain_outbox`, and never it.

So: every message in the application was unread for ever. Mail read on a phone stayed bold.
Unread counts were the size of the mailbox. Stars never arrived. Messages deleted elsewhere were
never removed. And Gmail's labels never appeared at all, because they ride the same survey —
which is why F131's `label:` fix had nothing to find on a real account.

This is the third time: F128 was `watch`, which nothing called; F131 was the label resolver,
which the window never passed; this is `sweep`. A capability with tests and no caller looks
exactly like a working feature from inside the repository.

The live IMAP fixture already served both its messages as `\Seen` and the test already asserted
the pass "reported success and stored nothing" was false. It never asked what state they were in.

### F137 — The header fetch asked for flags and threw them away

F136's fix made the sent folder correct and left something behind: of 200 messages added by
backfill, 23 had a read state. The sweep was doing all the work, and on a CONDSTORE server it
asks `CHANGEDSINCE`, which by design never revisits mail that has not changed. Anything found by
walking backwards through the mailbox was therefore unread for ever, sweep or no sweep.

The header fetch is `UID FETCH … (UID FLAGS BODY.PEEK[HEADER])`. It asks for `FLAGS`. The
`Job::Fetch` arm collected the literals and returned `ProtoOutcome::Fetched { items }`, and the
flags on those same FETCH lines went nowhere. `parse_fetches` — the function that reads exactly
this, for the survey — was three hundred lines away.

`ProtoOutcome::Fetched` now carries them, and the engine applies them through `Ingest.flags`,
which is the path the sweep already used. POP3 sends an empty list, because POP3 has no
server-side flags at all.

**The test for this already existed and asserted nothing.** `fetching_headers_uses_body_peek`
replays a trace containing `FLAGS (\Seen)` and checked
`matches!(outcome, ProtoOutcome::Fetched { .. })` — true whatever became of them. It was written
to pin `BODY.PEEK` over `BODY`, which it does; the flags were in the fixture by accident and
nobody ever looked. That is `CONVENTIONS.md`, "An assertion that was already true", in a test
that had been passing for months.

On the real account, after both fixes: 55 of the next 200 backfilled messages arrived with a read
state, the sent folder is 135 of 138 read, and 463 messages carry Gmail labels where 123 did
before, and none did before F136.

### F138 — The send path supported attachments until somebody needed one

The fourth modelled-and-unreachable, after F128 (`watch`), F131 (the label resolver) and F136
(`sweep`), and the one that reads most like a working feature from inside the repository.

`Draft.attachments: Vec<PendingAttachment>` has been in `mail-domain` since phase 1 and is
covered by the serde round-trip tests. `mail_mime::build` has assembled `multipart/mixed` from
it since phase 2, refusing a referenced blob that is absent with `MimeError::MissingPart` rather
than dropping it silently, and `mail-mime/tests/build.rs` proves that with a fixture. The
`drafts` table has had an `attachments` column since migration 0001, and `sqlite/draft.rs` reads
and writes it. `compose::send` resolves every `PendingAttachment` to its bytes out of the blob
store before calling `posting`, with a per-attachment error message naming the file.

Five layers, each tested. `grep -rn PendingAttachment --include=*.rs crates/` outside
`mail-domain` returns the serde tests, the MIME builder, its fixture, and the store — and nothing
in `mail-app`. No surface ever constructed one, so no draft ever had a non-empty
`attachments`, so every message this client has ever sent was single-part regardless of what the
user meant to send with it.

What makes this the worst of the four is that it is invisible in exactly the direction that
matters. `sweep` not running showed up as mail that stayed unread; a missing `watch` showed up as
mail that did not arrive. An attachment that cannot be added shows up as nothing at all: the
composer had no control, so there was no failure to notice, and the machinery beneath it would
have worked the first time it was asked.

`compose::attach_bytes` is the missing constructor. Bytes rather than a path, because the window
has a chooser that hands over contents and the command line has a path, and only one of those is
I/O that function should be doing. The name goes through `attach::safe_name` — the same function
that decides where an *incoming* attachment may be written — so `../` and a newline are refused
in both directions by one rule, which matters here because the name is about to become part of a
`Content-Disposition` header.

The budget is checked while the file is being attached rather than at Send, and it counts what
is already there. A refusal at Send arrives after the message is written and addressed, when
there is nothing useful left to do about it; a budget that measures only the newest file is how
a message grows past what the server will take, one acceptable file at a time.

`BlobStore::size` came out of the same work. `blobs.size` has been a column since migration 0001
and nothing selected it, so the first thing that wanted a file's length read the whole file to
measure it — for a list of what a draft carries, that is every attachment loaded into memory to
print its size.

### F139 — Everything the window did stopped at this machine

Found while wiring phase 7d, by asking what would carry a label to Gmail.

`Op::apply` returns three things: the local patch, its undo, and `remote: Option<RemoteIntent>` —
the server's half. `ui::apply_op` wrote the first and dropped the third on the floor. It is the
function behind every row button and every keyboard shortcut in the window, so archiving, trashing,
starring, and marking read or unread all changed this database and told the server nothing.

It was worse than that, because the same function also built the `AccountCaps` it passed to
`apply` out of hardcoded safe defaults — `ArchiveMeans::LocalOnly`, `ServerLabels::LocalOnly` —
with a comment saying real capabilities arrive once an account is synced. They do. Nothing read
them. `remote_intent` is the *only* consumer of `caps`, and under `LocalOnly` it returns `None`
for Archive, Trash, Spam and Restore. So the remote work that was being discarded had mostly
never been computed: two independent faults, each of which alone would have produced the same
silence.

Real accounts show what this cost. Gmail is recorded as `archive: drop_inbox`,
`labels: supported`; the campus account, being POP3, is `local_only` for both. So archiving a Gmail conversation
in mailo left it in the inbox on the phone and brought it back here on the next full sync, and
mail read here stayed bold everywhere else. `SetFlags` is emitted whatever the capabilities say,
so read and star were lost purely to the dropped field.

**Why it was invisible.** The account tested against most is the one where the hardcoded value is
the truth: on POP3 there is nowhere to file anything and `LocalOnly` is correct. F108 watched `j`
open a conversation and `e` archive it, and the assertion — the inbox is one conversation shorter
— was true, because the local half always worked. The test could not have failed. Nothing in the
repository distinguishes "archived" from "archived here only" unless it looks in the outbox, and
nothing did.

`apply_op` now reads the account's capabilities through `sync::caps_of`, keeping the safe
defaults only for an account nothing has connected to yet, and enqueues `applied.remote` with
`applied.inverse` as its undo — the same call `compose::send` has always made. The fixture also
gained an `account_caps` row, because `account add` always writes one and a fixture without it is
a state the application cannot reach, which is F133's lesson applied a second time.

### F140 — The window may stop running after its first render

Found by phase 8c, which moved the thread list and the badge counts onto blocking threads behind
`use_resource`: 1382 tests passed and `scripts/live-window.sh` showed an empty mailbox. 8c is
reverted. What the investigation found is larger than 8c and is not yet resolved.

**What is established, against the real binary, under Wayland and under X.**

A click reaches its handler and the signal is written:

```
PROBE: sync clicked
PROBE: sync state set to Running
```

and the button's label — `if may_start() { "Sync" } else { "Syncing…" }` — stays `Sync` for every
one of fifty samples taken 100 ms apart. The DOM is read directly, so this is not about painting:
the text never changes. `scripts/live-window/sync.js` is that probe.

The committed `probe.js` says the same thing about the search box. Typing `label:travel`, then
`label:nosuchlabel`, then clearing it, the list reports the identical two subjects at all four
stages. Searching does not filter, and a query that matches nothing does not empty it.

Futures stop too. `use_future` and `use_resource` bodies start — once — and a `tokio::time::sleep`
loop added for the experiment printed one tick of twelve in fourteen seconds. There is a
multi-threaded tokio runtime and `use_effect` runs; what does not happen is the *next* poll.

**What is not established: whose bug it is.** `crates/mail-app/examples/rerender.rs` is the same
experiment with no `mail-app` code in it — one signal, one timer, one button — and it ticks
exactly once, under both `dioxus::launch` and `LaunchBuilder::desktop()`. So this is not something
about how this project uses signals or futures.

That leaves two explanations and no way to choose between them from a terminal. Either
dioxus-desktop's run loop stops driving the `VirtualDom` after the first wake, or a compositor is
throttling a surface that is created and never presented — the window is launched from a shell
into a session nothing brings it to the front of. The second would mean the user's own visible
window is fine and only the harness is blind; the first would mean the window is a static
snapshot of the moment it opened.

**If it is the first**, two things already recorded as done are not done. F128's poll loop is a
`use_future` whose first statement is a two-second sleep, so no pass ever runs and mail arrives
only when Sync is pressed — the sentence F128 exists to delete. And the composer's three-second
autosave never fires, covering the one window nothing else covers.

**Why the tests could not see it.** F128 added
`a_future_started_when_a_component_mounts_does_run`, which passes, under `#[tokio::test]`, where
tokio drives its own timers and wakes its own tasks. It is a property of the harness. Phase 6's
window journeys are asserted through `VirtualDom::handle_event` against a real store, which
exercises the component tree without the desktop runtime underneath it — the half that works.

**And the harness did not fail.** `scripts/live-window.sh` exits non-zero only when the page
reports *nothing*. It reported four stages showing an identical list, which is the evidence of the
bug, and the script said nothing was wrong. Printing what a page reported is not the same as
checking it — `CONVENTIONS.md`, "an assertion that was already true", in a shell script.

**Settled.** A hidden element, clicked by the page every 250 ms, reaches its Rust handler
**thirty-eight times** in twelve seconds — and in the same run the tokio timer ticks **zero**
times and the component renders **twice**. The event loop is alive and dispatching. It simply
never polls the `VirtualDom` again.

That pair of numbers rules out the explanation that looked most likely at first. A compositor
throttling a surface that nobody brings to the front does not deliver thirty-eight clicks. The
window is not asleep; the dom is.

`dioxus-desktop/src/waker.rs` names the mechanism exactly:

```rust
impl ArcWake for DomHandle {
    fn wake_by_ref(arc_self: &Arc<Self>) {
        _ = arc_self.proxy.send_event(UserWindowEvent::Poll(arc_self.id));
    }
}
```

Waking the dom sends a `Poll` through tao's `EventLoopProxy`, and `launch.rs` turns that event —
and only that event — into `app.poll_vdom(id)`. Handling a DOM event does not poll it. So when
those proxy events are not delivered, every future stalls and every signal write goes unrendered.
The send result is discarded, so nothing reports the failure.

Instrumenting `App` shows the same shape in the product: the body runs twice, a click on Sync
reaches its handler and writes its signal, and no render follows the write.

**Not fixable from inside this repository.** The proxy is a `pub(crate)` field of
`SharedContext`, so application code cannot ask for a poll; and there is no arrangement of our own
signals or futures that makes a runtime re-render. Tried and ruled out: `dioxus::launch` against
`LaunchBuilder::desktop()`, `with_focused` and `with_always_on_top`, three unrelated wake sources
(a tokio timer, a channel written from an OS thread, `document::eval`), and a page-driven DOM
heartbeat. dioxus 0.7.10 is the latest stable; 0.8 is an alpha, which is not something to put
under a mail client to chase a symptom.

**What it costs.** The window is a static snapshot of the moment it opened. F128's poll loop never
runs a pass, so mail arrives only when Sync is pressed — and pressing Sync does not visibly do
anything either, though the pass itself runs. The composer's three-second autosave never fires.
Everything phase 7 added — the New button, the label and snooze menus, the attach control — is
correct code that updates the database without updating the screen. The command line is
unaffected, and so is every test.

**Why nothing caught it.** F128 added `a_future_started_when_a_component_mounts_does_run`, which
passes under `#[tokio::test]`, where tokio drives its own timers and wakes its own tasks — a
property of the harness, not of the window. Phase 6's journeys go through
`VirtualDom::handle_event` against a real store, which exercises the component tree without the
desktop runtime underneath it: the half that works. And `scripts/live-window.sh` exited 0 as long
as the page reported *something*, so it printed four stages showing an identical list and called
that a pass. It now fails when the list does not change across a search.

**Status: open, upstream, and not reproducing.** The next step is a minimal report against dioxus
or tao — `crates/mail-app/examples/rerender.rs` is already that reproduction — and, until it is
answered, the command line is the surface that works. But see **F141**: on the same four versions
this window now re-renders, so the report cannot be written from a reproduction that no longer
reproduces, and the first thing to find is which part of the environment this was ever about.

### F141 — The window re-renders here, on the versions F140 was filed against

F140 is titled "may stop running", and the hedge turns out to be load-bearing. Driving the real
binary today, the window re-renders correctly, and nothing about the dependency set has moved:
`dioxus-desktop 0.7.10`, `dioxus-interpreter-js 0.7.10`, `tao 0.34.8`, `wry 0.53.5` — the same
four versions F140 recorded.

`scripts/live-window.sh` with its own probe now exits 0, with four distinct lists across four
stages: both subjects, then `label:travel` leaving one, then `label:nosuchlabel` leaving none,
then clearing restoring both. That is the exact check the script was taught to fail on, passing.

**What was being tested, and why it needed a second probe.** Reading `dioxus-desktop` 0.7.10
offers a candidate F140 did not consider. `WebviewInstance::poll_vdom` returns at its first line
while `poll_edits_flushed` is pending, and that flag clears only when the page acknowledges the
last batch of edits — which `dioxus-interpreter-js`'s `rafEdits` does from inside a
`requestAnimationFrame` callback, except on the `headless` branch taken when the window is
invisible. On that reading a surface the compositor never presents gets no frame callbacks, so
the acknowledgement never arrives and the dom is never polled again. It fits every number F140
recorded, including the thirty-eight clicks: a DOM event reaches Rust over a synchronous
`XMLHttpRequest` on wry's custom protocol and never touches the event loop at all.

F140 used those thirty-eight clicks to rule out a compositor throttling a window nobody brings to
the front. That argument refutes *event* throttling. It does not touch frame-callback starvation,
which is a different mechanism on a different channel — and frame callbacks are what `rafEdits`
waits on.

So `scripts/live-window/frames.js` counts frames and types, in one run, because the two
explanations differ in what they predict happens *together*: frames stalling alongside a frozen
list points at the rAF path, frames ticking through a frozen list points at the waker, and both
healthy points at neither. It reported both healthy — `raf_ticks` 3, 59, 116, 173 across 2.8 s,
about 63 a second, while the list changed at every stage, with `visibility` visible and
`hasFocus` true throughout.

**What this establishes, and what it does not.** It establishes that F140 is not a property of
these versions, because these versions work here. It does not adjudicate between the two
mechanisms at all: the run never entered the failing state, so it tested nothing about it.
Measuring frames on a machine where the window already works cannot distinguish hypotheses about
why it sometimes does not — every one of them survives.

The variable is the environment, and the honest reading is that the status of F140 was never
"broken" but "broken under conditions nobody had pinned down". Worth noting that
`scripts/live-window.sh` advertises working with the screen locked, and launches the binary
backgrounded from a shell with no window manager attending it — which is a plausible way to get a
surface that is never presented, and would be the first thing to vary deliberately.

**Status: F140 does not reproduce; both open.** What would settle it is a run of `frames.js` with
the surface deliberately unpresented — screen locked, or the window occluded — looking for
`raf_ticks` to stop climbing and the list to freeze in the same run. Until then the shell is a
surface that works on this machine and is untrusted on one that showed F140, and no amount of
correct code behind it changes that.

### F142 — A campus can run several mail systems, and an address cannot say which one it is on

Phase 9.9, 2026-09-23. The campus preset picked one of two POP3 hosts from the shape of the local
part: one for a student id, the other for anything else. Checked against the institution's own
documentation and the servers themselves (greeting, `CAPA` and `EHLO` only; no credentials sent),
there were three systems, not two:

| | host A | host B | host C |
|---|---|---|---|
| Who | students enrolled recently; alumni with id usernames | alumni and affiliated staff with name usernames | staff, faculty, units, and earlier students |
| POP3 | 995 TLS, Dovecot, `SASL PLAIN` | 995 TLS, Dovecot, identical `CAPA` | 995 TLS, Exchange, `SASL PLAIN` |
| Submission | 465 implicit TLS | 465 implicit TLS | 587 `STARTTLS`, `AUTH GSSAPI NTLM LOGIN` — **no `PLAIN`** |
| Login name | local part | local part | the full address |

The two-host rule was right for the two systems it knew about. The third is the one most people
with an address there are on, and the rule sent every one of them somewhere they could not log
in. Nor can the address decide it: the same id is on one system while enrolled and another after
graduating, and a professor's name-shaped local part looks exactly like an alumnus's.

The Exchange submission server is also the first real server seen here that offers `SMTPUTF8`,
`DSN`, `CHUNKING` and `BINARYMIME` — phase 9.4, 9.7 and 9.8 have somewhere to be tried.

**Status: closed by removing the preset.** An institution's mail layout is not general protocol,
and this is an open-source client: no preset names one. A server outside the provider table is
configured by naming its hosts — `mailo account add ADDRESS --imap HOST --smtp HOST`, or `--pop3
HOST` for one that offers only POP3 (`presets::manual_pop3`: implicit TLS, mail left on the
server, `CAPA` read before anything is fetched) — plus `--login` where the login is not the
address.

### F143 — IMAP fetches were paired with their UIDs by position, and a real mailbox was scrambled

Found 2026-09-23 by the live send tests: a Gmail bounce was listed for three syncs as "body not
fetched yet". The message had no `remote_map` row at all, so no fetch could ever name it. Nor did
1,762 others on the same account, while 1,302 messages held several INBOX UIDs each; the newest
message in the mailbox was mapped to UID 30677 while rows up to 32535 pointed at mail from
weeks earlier.

`ImapBackend`'s `Job::Fetch`, which serves both the header pass and the body pass, built its
result as `remotes.into_iter().zip(bodies)`: the n-th UID asked for, paired with the n-th
literal that came back. RFC 3501 promises no order, and servers answer in mailbox order and
say nothing about a UID that has been expunged. While the engine asked oldest first the two
orders happened to agree. Phase 9.1 made it ask newest first, and from then every batch was
paired backwards. Each message's *content* stayed its own, because the store keys a message by
the Message-ID in the bytes that arrived; what moved was the row saying which UID holds it,
and with it every flag and label the server reported for that UID.

No test could see it. The fake server in `imap_end_to_end.rs` answered in request order, which
is the one order that makes positional pairing correct.

**Fixed.** Each response's own `UID` decides whose bytes they are, read from the response with
its literal cut out, so a header or body that says `UID 3` is not mistaken for one. A reply for
a UID that was not asked for, or none for one that was, drops out rather than shifting the rest.
`bodies_are_matched_by_uid_not_by_order` fails on the old code with exactly the real symptom;
the fake server now answers in mailbox order. Migration 0008 drops every IMAP mapping, which
cannot be told right from wrong, and the header pass rebuilds them: it fetches what the server
lists and the store does not map, matches each to the message already held by its key, and
applies the flags the server reports for it. POP3 mappings never passed through this code and
are kept.

Nothing reached the server: the outbox and pending changes were empty when this was found. A
queued archive or flag change would have been sent to whichever message the scrambled row
named.

**Also seen, not investigated:** some older messages in that mailbox have subjects in raw
8-bit Big5, not RFC 2047 encoded words, and are listed as replacement characters.

**And a second bug behind it.** With the pairing fixed, passes fetched 200 headers and no
bodies. `Untagged::literal` found the literal marker by searching for the *last* `{` in the
response, which is inside the message whenever the message has one — every HTML mail with a
stylesheet. Those came back with no body, silently, and since `unfetched` hands out the newest
first, the same hundred were asked for on every pass and nothing older was ever reached. The
positional pairing had hidden it: bodies that did parse were stored under the wrong UIDs, so
something always arrived. The marker is now the first `{n}` followed by CRLF;
`a_body_with_braces_in_it_is_still_a_body` fails on the old search. Against the real account,
the next pass fetched 200 bodies — including the SMTPUTF8 bounce, which quotes
`測試.mailo@example.com` exactly as it was sent.

### F145 — A Microsoft 365 tenant, live: IMAP yes, SMTP AUTH no, Graph yes

2026-09-23, against a real work tenant — the spike `plan.md` asked for, answering its three
tenant-policy questions for one organisation:

1. **User consent:** not restricted. An app registered in the user's own directory (Entra ID
   Free; no fee) was consented to at sign-in with no administrator involved.
2. **IMAP:** enabled. The first pass fetched 200 headers and 100 bodies; the server offers IDLE
   and an Archive folder, and neither `CONDSTORE` nor `QRESYNC`.
3. **SMTP AUTH:** disabled tenant-wide. `535 5.7.139 … SmtpClientAuthentication is disabled for
   the Tenant`, which is the common default.

So the account could read and not send, and the plan's fallback was built: `Outgoing::Graph`,
`mailo account add … --microsoft --send graph`, Graph's `sendMail` with the frozen message as
base64 MIME. What the live run found on the way:

- **A code consented for two resources must be redeemed naming one.** Asking for Exchange's IMAP
  and Graph's `Mail.Send` in one sign-in is allowed; redeeming that code with no `scope` is not
  (`AADSTS28003`). The first resource named is the one redeemed for, and Graph's token comes from
  a refresh that names Graph's scope. Each access token is for one resource, so the account holds
  two: the IMAP one as its incoming credential and the Graph one as its outgoing.
- **A blind copy survives Graph.** Graph reads recipients from the headers, not an envelope, so
  a `Bcc:` header is added to the copy handed to it. Checked live: the blind recipient received
  the message, and neither copy carried a `Bcc:` header or any trace of that address. Exchange
  also replaced the `From` display name with the directory's.
- **Re-running `account add` needed the client id again** although the first sign-in had
  recorded it; it now falls back to the recorded one.
- **Discarding a failed send did not stop it.** The refused SMTP attempt backed off for a day,
  still queued, while its draft read `Failed`; discarding deleted the draft and left the outbox
  entry to send it the next day. Deleting a draft now withdraws its queued submission.

Still open: `mailo watch` holds one engine for hours and renews neither token inside it, so an
OAuth account's watch loop stops working about an hour in until restarted. And Graph takes at
most 4 MB per request, so a message over about 3 MB before encoding is refused with that said.

### F146 — ammonia strips `class`, so no quote-class rule can work

Ammonia's `generic_attributes` are `lang` and `title`. `class`, `id` and `style` are gone before the block parser sees a byte. `gmail_quote`, `moz-cite-prefix`, `yahoo_quoted` and `OutlookMessageHeader` do not survive sanitization. A rule against them never fires, and a test of it passes if the test forgot to sanitize first.

Detection is positional and textual: an attribution is recognised only when it is immediately followed by a quote. Adding `allowed_classes` was considered and rejected. Those names are chosen by the sender, so anyone could mark their first div `gmail_quote` and have the whole message drawn as quoted text.

### F147 — Three ways a watch mistook a wait for a refusal, or a refusal for a wait

Found while building 10.1 and 10.2 and fixed there.

- A token endpoint that could not be reached was reported as `RuntimeError::Secrets`, which is
  read as `NeedsReauth`. A network blip at the hourly refresh stopped `mailo watch` and asked the
  user to sign in again. It is `Connect` now; only the issuer's own error answer is a refusal.
- `refresh_caps` returning `NeedsReauth` reached `pass()` as prose, so the watch treated it as an
  ordinary failure and retried it every minute against a credential already refused.
- `pass()` dropped the outbox drain's `needs_reauth` and `hold`. A credential rejected while
  sending did not stop the poll loop, and a server's rate limit on submission was not honoured.

The common shape: each layer classified its failure correctly and the next layer flattened it to
a string. `Retry` survives only where it is passed along as a value.

### F148 — Multi-byte charsets were never decoded, and a re-ingested sender kept its old name

Found while building 10.5 and fixed there.

- `mail-parser` was built without its `full_encoding` feature. Without it every multi-byte
  charset — Big5, GBK, Shift_JIS, EUC-KR — is decoded as if it were UTF-8, so a
  `=?gb2312?B?…?=` subject or a Big5 body came out as replacement characters even when the
  sender labelled it correctly. Nothing failed: the parser returned text, just the wrong text.
- `upsert_message` in the SQLite store did not update `from_name`/`from_email` on conflict, so
  a message ingested again kept the sender it was first stored with. `MemoryStore` replaced the
  whole message, so the two stores disagreed, and the parity proptest never generated a
  re-ingest with a changed sender.

Mail already stored with replacement characters is re-read from its raw bytes once, through the
`messages_to_reparse` queue (migration 0012).

### F149 — Mail that landed just before IDLE waited for the next unrelated wake-up

Found through a test that failed only under load. A watch runs a sync pass, then opens a
connection, `SELECT`s the Inbox and `IDLE`s. `IDLE` announces what arrives while it runs; mail
that arrived after the pass looked and before `IDLE` began is visible only as a higher
`UIDNEXT` in the `SELECT` response, which nothing read. That message sat unfetched and
unannounced until some later push woke the watch, which on a quiet mailbox could be the server's
IDLE timeout.

`ProtoOp::Watch` now carries the `UIDNEXT` the client has synced to, and the walk's
`IdleAfter` command ends without idling when `SELECT` reports a higher one. The test's fake server
had the same blind spot — its IDLE compared against the count at `IDLE`, not at `SELECT` — which
is why the race showed only when a loaded machine stretched the gap.

### F150 — A send could go twice, and nothing ever said a message was being sent

Found while building 10.8 and fixed there.

- `mailo send` on a draft that was already queued or failing added a second `Submit` to the
  outbox beside the first, so the recipient got the message twice. Queuing now takes back any
  earlier submission of the same draft first.
- `SendState::Sending` existed and nothing set it. `unsend` checked for it to refuse taking back
  a message the server was already accepting, so that check could never fire. `drain_outbox` now
  marks the draft `Sending` before handing it over.
- The `Date` a message carried was the moment it was frozen, so a send held until morning — or
  by a closed laptop — was dated when it was written. It is now re-stamped as it leaves.
