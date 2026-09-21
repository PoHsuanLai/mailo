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

**The NTU host heuristic is still a guess.** One or two ASCII letters then 7–9 digits, or 7–9
digits alone, → `msa`; anything else → `ccms`. `spike/out/` is empty, so nothing has checked it.
When the guess is wrong the symptom is a POP3 login failure on first connect, not data loss.

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

- **The NTU host heuristic is unverified.** `spike/out/` is empty.
- **F2's parity limit**, for whoever takes the `parity` brief: `mail-domain`'s fold table covers
  Latin-1, Latin Extended-A/-B and Latin Extended Additional. SQLite's is wider. Generate the
  proptest corpus from Latin + ASCII, or add a Unicode dependency.

## Phase 0 — the spike, run 2026-09-22

Run against a real Gmail account and `msa.ntu.edu.tw`. Transcripts are in `spike/out/`, which is
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

### F16 — the NTU preset guessed the wrong SASL mechanism

`CAPA` on `msa.ntu.edu.tw`: `SASL PLAIN`, `USER` — and **no `LOGIN`**, no `CRAM-MD5`, no `STLS`.
The preset offered `[Login, Plain]`, so the first live connect would have failed on a mechanism
the server does not implement. Corrected to `[Plain]`. No STLS is expected and fine: we connect
with implicit TLS on 995.

The host heuristic held for this account — the default `msa` worked — but only one shape has
been tested, so `ccms` remains a guess.

### F17 — the NTU mailbox is large (amended: it is mostly a hundred attachments)

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

`CAPA` on `msa.ntu.edu.tw` actually advertises:

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
`RETR` does**, and NTU runs `pop3_no_flag_updates` at its default. A first sync that simply
`RETR`s 2372 messages would silently mark the user's whole mailbox as read in NTU webmail —
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

No display name on the generated identity: deriving one from the local part produces "B09901185"
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
