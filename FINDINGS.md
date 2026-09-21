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
