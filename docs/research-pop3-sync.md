# POP3 first-sync research — 2372 messages, 255 MB, one campus Dovecot

Researcher notes for F17. Written 2026-09-22.

Everything below distinguishes **[RFC]** (guaranteed by RFC 1939 / 2449 / 5034),
**[MEASURED]** (from `spike/out/pop3.trace`, our own run against a campus POP3 server on
2026-09-22), and **[PRACTICE]** (what servers and mature clients actually do, cited).

---

## 0. Correction to the brief, before anything else

The brief says CAPA advertises "SASL PLAIN and USER. No LOGIN, no CRAM-MD5, no STLS."
That is true but incomplete. The actual CAPA in our own trace is:

```
+OK Dovecot ready.
C: CAPA
S: +OK
   CAPA
   TOP
   UIDL
   RESP-CODES
   PIPELINING
   AUTH-RESP-CODE
   USER
   SASL PLAIN
   .
```

**[MEASURED]** Three facts the brief omits and that change the answer completely:

1. The server is **Dovecot**.
2. **`TOP` is advertised.** The most important question in the brief is already answered
   affirmatively by a transcript we already own.
3. **`PIPELINING` is advertised.** (RFC 2449.)

So the two levers that matter most are both available on the one server we care about,
and we have a transcript proving it. F17 should be amended.

---

## 1. The measured shape of this mailbox — the real finding

F17 frames the problem as "2372 RETR round trips and a quarter of a gigabyte". The
`LIST` output in our own trace says the bytes are not distributed the way that framing
implies. Derived from the 2372 `LIST` sizes in `spike/out/pop3.trace`:

| statistic | value |
|---|---|
| count | 2372 |
| total | 267,508,676 B (255.1 MiB) |
| mean | 112,778 B |
| p10 / p50 / p75 | 2,516 / 11,130 / 19,160 B |
| p90 / p95 / p99 | 63,582 / 387,864 / 2,826,321 B |
| max | 9,943,900 B |

Cumulative, by size threshold:

| threshold | messages | share of count | bytes | share of bytes |
|---|---|---|---|---|
| ≤ 64 KiB | 2137 | **90.1 %** | 26.6 MB | **10.0 %** |
| ≤ 256 KiB | 2237 | 94.3 % | 38.8 MB | 14.5 % |
| ≤ 1 MiB | 2315 | 97.6 % | 83.2 MB | 31.1 % |
| ≤ 5 MiB | 2362 | 99.6 % | 195.9 MB | 73.2 % |
| the 100 largest | 100 | 4.2 % | **215.4 MB** | **80.5 %** |

**The mailbox is not 255 MB of mail. It is ~25 MB of mail and ~230 MB of attachments
in a hundred messages.** A size-aware fetch order gets 90 % of the messages
body-complete for 10 % of the bytes. This is a bigger lever than TOP, and it is free —
`LIST` gives exact octet counts for every message in one multi-line response before any
`RETR`.

Header/body split, from the one full `RETR` in the trace (message 1, 24,408 octets
total): **3,323 B of headers, 20,709 B of body** — headers are 13.6 % of that message.
Extrapolating ~2–4 KB of headers per message, a full `TOP n 0` pass over all 2372 is
roughly **7–9 MB**, i.e. ~3 % of the maildrop. *(Caveat: one sample. The estimate is
order-of-magnitude, not a measurement.)*

Network: TCP connect to the server's port 995 measured at **15–38 ms** (3 samples; ICMP
is blocked, TCP is not). So ~15 ms RTT. 2372 strictly-serial round trips ≈ **36 s of
pure latency** before a single byte of payload is counted. This is why PIPELINING is
not a micro-optimisation for the header pass — without it, the header pass is
latency-bound, not bandwidth-bound.

---

## 2. TOP — headers-only fetch

### What the RFC says

**[RFC 1939 §7]** `TOP msg n` is an **optional** command, valid only in TRANSACTION
state, taking a message number and a *required* non-negative line count.

> After the initial +OK, the POP3 server sends the headers of the message, the blank
> line separating the headers from the body, and then the number of lines of the
> indicated message's body […]
>
> Note that if the number of lines requested by the POP3 client is greater than than
> the number of lines in the body, then the POP3 server sends the entire message.

`TOP msg 0` is therefore the exact "headers only" call: headers plus the blank line,
nothing else.

**[RFC 1939 §9]** TOP is listed under "Optional POP3 Commands" alongside APOP and UIDL.
**[RFC 2449]** adds the `TOP` capability tag to CAPA, so support is discoverable rather
than guessed. Before RFC 2449 the only way to test was to issue `TOP 1 0` and see if it
errored — and **[RFC 1939 §3]** warns there is no way to distinguish "unimplemented"
from "unwilling":

> There is no general method for a client to distinguish between a server which does
> not implement an optional command and a server which is unwilling or unable to
> process the command.

### Does TOP mark the message read?

**[PRACTICE]** No, on Dovecot, and generally not elsewhere. Dovecot's
`src/pop3/pop3-commands.c` gates the seen-flag update on `body_lines == UOFF_T_MAX`,
which is the sentinel `RETR` passes; `TOP` passes an actual line count and so skips it.
fetchmail relies on exactly this: its manual says fetchmail uses TOP rather than RETR
specifically *to avoid marking messages seen*, and lists the conditions under which it
falls back to RETR (`fetchall` set; or `keep` set with `uidl` unset).

**This matters for us as a product fact, not a protocol fact.** The server is
Dovecot with `pop3_no_flag_updates` at its default (`no`), so **every `RETR` we issue
sets `\Seen` on the server-side mailbox**, visible in the campus webmail and any IMAP client.
A naive "RETR everything on first sync" marks the user's entire 2372-message mailbox
as read in their webmail. A TOP-first design does not, until the user actually opens a
message. That is a user-visible reason to prefer TOP that has nothing to do with
bandwidth.

### How widely supported, and how badly broken

**[PRACTICE]** TOP is the most widely implemented of the optional commands — Dovecot,
Courier, Cyrus, Exchange, Gmail and hMailServer all implement it — but it has a long
tail of misbehaviour, documented in the fetchmail FAQ:

- **Maillennium POP3/PROXY** truncates TOP responses at 64–82 kB, violating RFC 1939.
  fetchmail ≥ 6.3.2 hard-codes a greeting-string sniff for `Maillennium POP3/PROXY
  server` and falls back to RETR with a warning. (FAQ I8.)
- **USA.NET 2.2** returns only ~10 lines regardless of the argument. (FAQ I3.)
- **OpenMail** returns one line regardless of the argument.
- **MailMax** "sometimes fails to retrieve the entire message even when enough lines
  have been specified". (FAQ S6.)
- **FTGate** answers `+OK` *twice* to a TOP request. (FAQ S7.)
- **GeoCities** omits the first `Received:` line from TOP output. (FAQ I4.)

Note the shape of these bugs: **almost all of them are about the *body lines*, not the
headers.** Truncation at 64 kB, "only 10 lines", "one line" — all are failures to
deliver the requested body portion. None of them is a report of wrong or missing
headers. `TOP msg 0` sits in the part of the command that implementations get right.
The risky usage is `TOP msg 500` as a cheap partial RETR, which is what fetchmail does
and what these bugs bit.

**[PRACTICE]** JavaMail/Jakarta Mail uses TOP for header access by default and caches
the result, with `mail.pop3.disabletop` as the escape hatch that forces full RETR.
Thunderbird gates its "Fetch headers only" feature on the `TOP` CAPA tag; Mozilla bug
1783290 is a regression where a server that answered CAPA in **lowercase** (`top`,
`uidl`, `user`) was read as not supporting TOP, and Thunderbird silently downloaded
full messages instead. **Lesson: uppercase the CAPA tags before comparing.** RFC 1939
§3 says keywords are case-insensitive; the bug is in clients that forget.

Thunderbird's headers-only mode has also produced a run of data-loss bugs
(bugzilla 261668, 314310, 1930847: "messages get incorrectly deleted", "mails
disappear", "prevents message download"), all of which are about the *interaction of
headers-only with deletion and size filters*. That is a strong argument for our
`LeaveOnServer::Keep` default for POP3: headers-only plus never-delete has no way to lose
mail, whereas headers-only plus delete is the configuration that has repeatedly eaten
people's inboxes.

### Verdict

**TOP is viable, and it is the right primitive for the header pass.** Specifically:

- Use `TOP msg 0` — headers only, zero body lines. This is the well-implemented corner.
- Gate on the `TOP` capability from CAPA, **case-insensitively**.
- Treat a `-ERR` from TOP as "this server's TOP is unusable" and fall back to full
  `RETR` for the whole account, not per message; a server whose TOP errors on one
  message will error on more, and per-message fallback turns a header pass into a
  full-mailbox download by accident.
- Sanity-check the response: if a `TOP msg 0` reply is not terminated within some
  bound, or arrives with no `:` in it, fail the account over to RETR rather than
  storing a truncated header block.
- Do **not** use `TOP msg <n>` with n > 0 to get a snippet. On this mailbox the median
  message is 11 KB and mostly MIME multipart; the first 20 lines of body are usually
  `Content-Type` boilerplate and base64, not readable text, so you pay for lines that
  produce no snippet — and it is exactly the usage the broken servers break.

**Cost:** one round trip per message (amortised by pipelining) and ~7–9 MB total, vs
255 MB. The inbox becomes a complete, correctly dated, correctly threaded list of all
2372 messages — sender, subject, date, `Message-ID`, `References` — with no bodies. The
only thing the list is missing is the snippet.

**Snippet, honestly:** there is no cheap POP3 way to get one. Take the snippet from the
body when the body arrives. Until then the row shows subject and sender, which is what
a POP3 account gets. Do not invent one.

---

## 3. Resumability

### What POP3 gives you and what it does not

**[RFC 1939 §5–6]** The transaction model is the whole answer here:

- After successful auth the session enters TRANSACTION state, "with no messages marked
  as deleted".
- `DELE` only *marks*: "The POP3 server does not actually delete the message until the
  POP3 session enters the UPDATE state."
- UPDATE is entered **only** by a client-issued `QUIT` from TRANSACTION state.
- > If a session terminates for some reason other than a client-issued QUIT command,
  > the POP3 session does NOT enter the UPDATE state and MUST not remove any messages
  > from the maildrop.
- Autologout does not count as QUIT either: "When the timer expires, the session does
  NOT enter the UPDATE state — the server should close the TCP connection without
  removing any messages or sending any response to the client."
- `RSET` unmarks everything within the session.
- And the failure mode to know about: on QUIT, "If there is an error […] the maildrop
  may result in having some or none of the messages marked as deleted be removed. In no
  case may the server remove any messages not marked as deleted."

**Consequence: a dropped connection is always safe.** Nothing is deleted. The maildrop
is exactly as it was. This is the single most useful property POP3 has, and it means
resumability is entirely a *client-side bookkeeping* problem.

**[RFC 1939]** What POP3 does **not** give you: there is no byte-range fetch, no
`RETR msg <start> <len>`, no resume of a partial `RETR`. **The atom of work is one whole
message.** A connection that drops 9 MB into a 9.9 MB `RETR` has to start that message
over. There is no extension for this — RFC 2449 defines the capability mechanism and a
handful of capabilities (`TOP`, `UIDL`, `SASL`, `PIPELINING`, `EXPIRE`, `LOGIN-DELAY`,
`RESP-CODES`, `IMPLEMENTATION`); none of them adds partial retrieval.

### What mature clients persist

**fetchmail** (GPL-2.0-with-OpenSSL-exception — *facts only were read, no code*):
persists a flat file of seen UIDLs, `.fetchids` by default, path settable with
`set idfile`. Notable facts:

- The file is the entire client-side state. Debian bug #798803 records a `.fetchids`
  with ~50,000 entries where the load was O(N²) because appends walked a linked list;
  the fix was a Patricia trie. Relevant to us only as a warning that **this state is
  indexed data, not a log** — we already have SQLite, so this is a non-issue by
  construction.
- `--expunge <n>` "arrange[s] for deletions to be made final after a given number of
  messages", explicitly described in the manual as breaking a long session into
  subsessions to **defend against line drops on POP3**. That is the canonical POP3
  resumability idiom: *checkpoint by QUIT-and-reconnect*.
- `--fetchlimit` caps messages per poll; `--fetchsizelimit` caps how many `LIST` sizes
  are fetched per transaction and exists, per the manual, "in reducing the delay in
  downloading the first mail when there are too many mails in the mailbox" — i.e.
  someone else already hit exactly our problem.
- `--limit <octets>`: "messages larger than this size will not be fetched and will be
  left on the server", with `--warnings` controlling how often the user is told.
  **This is size-aware deferral, in production since the 1990s.**
- `--fastuidl` does a binary rather than linear search for the first unseen UID, to
  avoid downloading all UIDLs on every poll.

**getmail / getmail6** (GPL-2.0): persists an `oldmail` file storing
`"<msgid>\0<timestamp>"` per seen message, one file per (account, folder). The docs are
blunt that deleting or editing it makes getmail re-retrieve everything. It also ships a
`BrokenUIDLPOP3Retriever` class for servers that do not implement UIDL or do not assign
unique ids — it treats every message as new, which is the honest degradation.

**Thunderbird** (MPL-2.0): persists UIDLs in `popstate.dat` per server, with a
per-message status flag (downloaded / headers-only / deleted). Bugzilla 421755 records
the failure mode when that file is lost or not written — disk full — namely re-download
of the entire mailbox. Bugzilla 1796903 records a regression where UIDL state was
ignored and messages were downloaded two or more times.

**K-9 Mail / Thunderbird for Android** (Apache-2.0): POP3 is deliberately a second-class
citizen; its docs note POP3 cannot carry read/unread state at all.

### The pattern, stated plainly

Every one of these clients persists **the same two things**: the set of UIDLs seen, and
a per-UIDL disposition. None of them persists message numbers, because message numbers
are session-scoped. None of them can resume a partial message, because the protocol
cannot.

---

## 4. UIDL stability

### What the RFC guarantees

**[RFC 1939 §7, UIDL]**, verbatim, because the exact wording matters:

> The unique-id of a message is an arbitrary server-determined string, consisting of
> one to 70 characters in the range 0x21 to 0x7E, which uniquely identifies a message
> within a maildrop and which persists across sessions. This persistence is required
> even if a session ends without entering the UPDATE state. The server **should** never
> reuse an unique-id in a given maildrop, for as long as the entity using the unique-id
> exists.

And, crucially, the very next paragraph:

> While it is generally preferable for server implementations to store arbitrarily
> assigned unique-ids in the maildrop, this specification is intended to permit
> unique-ids to be calculated as a hash of the message. **Clients should be able to
> handle a situation where two identical copies of a message in a maildrop have the
> same unique-id.**

So the RFC's own text concedes that **UIDLs are not guaranteed unique**. "Uniquely
identifies" and "clients should handle two messages with the same unique-id" are in the
same section. Any design that uses UIDL as a primary key must tolerate collisions.

Note also: **[RFC 1939]** "messages marked as deleted are not listed" by UIDL, and
`UIDL` is optional — RFC 2449 gives it a CAPA tag.

### What this server actually does

**[MEASURED]** The UIDLs are `0000000166aaf64b` … `0000094466aaf64b`. `0x944` = 2372 =
the message count. **[PRACTICE]** This is Dovecot's default
`pop3_uidl_format = %08Xu%08Xv` — the first 8 hex chars are the **IMAP UID**, the
second 8 are the **IMAP UIDVALIDITY**. `0x66aaf64b` = 1722480203 = 2024-08-01 02:43 UTC,
which is when this mailbox's index was created.

This is decodable, and we must still treat it as opaque — but it tells us the exact
shape of the failure:

**Every UIDL in this maildrop shares one 8-hex suffix, and that suffix is the
UIDVALIDITY. If the UIDVALIDITY ever changes, all 2372 UIDLs change simultaneously.**

UIDVALIDITY changes on: index corruption and rebuild, maildir recreation, server
migration, or an admin changing `pop3_uidl_format`. This is not hypothetical —
**[PRACTICE]** Plesk Obsidian 18.0.73 shipped Dovecot 2.4.1 with a changed
`pop3_uidl_format` (`UID%u-%v` → the new-syntax equivalent) and caused a documented
"POP3 re-download storm"; the community workaround was to pin
`pop3_uidl_format = %{uid | hex(8)}%{uidvalidity | hex(8)}` in
`99-local-pop3.conf`, and Plesk shipped PPP-69700 to fix it. Cyrus has the same
structural exposure: its UIDL is `uidvalidity.uid`.

### Detection and recovery

The detection rule follows from the structure, and it is cheap:

- **Total mismatch.** On a poll, compare the fetched UIDL set against the stored set.
  If `STAT` count is roughly unchanged but **the intersection with the stored set is
  empty or near-empty**, that is a UIDVALIDITY reset, not 2372 new messages. Treat "new
  message count ≈ total message count, and ≥ N previously-known UIDLs vanished" as the
  trigger.
- **Recovery must not re-download.** This is where `MessageKey` earns its keep. We
  already dedupe on `MessageKey` (`Message-ID`, or a blake3 of `(Date, From, Subject,
  first 4 KiB)`). A UIDL reset is therefore a **remap, not a refetch**: run the header
  pass (`TOP n 0` over all messages, ~8 MB), compute `MessageKey` for each, and rewrite
  `remote_map` to point the *new* UIDLs at the *existing* `MessageId`s. Only messages
  whose `MessageKey` we have never seen get a `RETR`. The cost of a full UIDVALIDITY
  reset drops from 255 MB to ~8 MB.

  This is exactly the same machinery as IMAP `UidValidity::Reset` (plan.md: "every
  `remote_map` row for that mailbox is invalid and must be dropped and refetched"). The
  POP3 case should say **dropped and re-*mapped*** — dedupe by `MessageKey` first,
  refetch only what is genuinely unknown. Worth fixing in plan.md for IMAP too.

- **Collisions.** Because the RFC permits hash-derived UIDLs, `(account, uidl)` is not
  safe as a unique key in `remote_map`. Duplicate UIDLs within one `UIDL` listing should
  be detected on parse and logged; the second occurrence gets a distinct
  `RemoteRef` (in practice: refuse to collapse them, prefer to keep both rows and let
  `MessageKey` dedupe the *messages*). Never assume UIDL ⇒ one message.
- **UIDL missing entirely.** `UIDL` is optional. Without it there is no safe way to
  leave mail on the server; getmail's `BrokenUIDLPOP3Retriever` degrades to "everything
  is new every time". For us the right answer is to refuse `LeaveOnServer::Keep` on a
  server with no UIDL capability and say so, rather than silently duplicating the
  mailbox on every poll.

---

## 5. Ordering

**[RFC 1939 §3, §5]** The guarantee is narrow:

> The first message in the maildrop is assigned a message-number of "1", the second is
> assigned "2", and so on, so that the nth message in a maildrop is assigned a
> message-number of "n".

Numbers are assigned when the maildrop is opened and are **stable for the duration of
that session** (a `DELE`d message's number is not reused, and the message simply becomes
inaccessible). Across sessions they are **not** stable: delete message 1 and everything
renumbers.

What the RFC does **not** say anywhere: that higher message number means newer. "The
nth message in a maildrop" is defined by the server's own ordering of the maildrop, not
by date. In practice every server appends, and **[MEASURED]** this one does
(UID `0x001` … `0x944` ascending with message number), but it is convention, not
contract.

### Strategies and their costs

1. **Walk `N` down to `1`.** Zero extra cost — `STAT` already gives N, and message
   numbers are free. Relies on the append convention.
2. **Sort by `Date:`/`Received:` from the header pass.** Costs nothing *extra* once you
   are doing a header pass anyway, and is derived from data rather than assumed. `Date:`
   is attacker-controlled and often wrong; the topmost `Received:` is server-stamped and
   better, but harder to parse.
3. **Sort by the UIDL's embedded ordering.** Tempting here (the first 8 hex are a
   monotonic UID) and **wrong** — F17 already says UIDLs stay opaque, and this one
   server's format is not a rule. Do not.
4. **Ignore order, fetch 1..N.** What a naive implementation does. Puts 2024 mail on
   screen first and the user's actual recent mail last. Unusable.

**Recommendation: (1) then (2), which cost nothing together.** Use descending message
number purely as the *fetch order for the header pass* — it is a heuristic about where
to find recent mail first, and if the server's ordering is unusual the only cost is
that headers arrive in an odd order. Then sort the **displayed list** by the parsed
date from those headers. The result is that ordering is never *asserted* from the
protocol; it is derived from the mail. If the append convention is violated, the
displayed order is still right and only the fetch order was suboptimal.

Concretely, **[MEASURED]**, the newest-first payoff on this mailbox: the newest 50
messages are 14.2 MB, the newest 200 are 27.5 MB, the newest 500 are 52.8 MB. So
"newest 200, body-complete" costs ~10 % of the maildrop.

---

## 6. Connection limits, timeouts, and the transaction model

### Timeouts

**[RFC 1939 §3]**

> A POP3 server MAY have an inactivity autologout timer. Such a timer MUST be of at
> least 10 minutes' duration. The receipt of any command from the client during that
> interval should suffice to reset the autologout timer.

**[PRACTICE]** Dovecot implements exactly the floor: `CLIENT_IDLE_TIMEOUT_MSECS` is
**10 minutes** post-login for POP3 (30 for IMAP), plus a **3-minute pre-login timeout**
that can shorten under connection-limit pressure. Since we send a command every few
hundred milliseconds during a fetch, the idle timer is never the thing that kills us.
The thing that kills us is the **network**, and a 9.9 MB `RETR` over a bad link.

There is no protocol-level cap on session *duration*, only on idleness — but a long
session is a long-held lock (below) and a long window in which a dropped TCP connection
loses all in-session progress. Both argue for short sessions.

### Locking and concurrency

**[RFC 1939 §4]**

> the POP3 server then acquires an exclusive-access lock on the maildrop, as necessary
> to prevent messages from being modified or removed before the session enters the
> UPDATE state.

**[RFC 2449]** defines the `IN-USE` response code for "successful authentication but the
user's maildrop is currently in use (probably by another POP3 client)", and `LOGIN-DELAY`
both as a capability (minimum seconds between logins) and as a response code.

**[PRACTICE]** Dovecot **does not** do this by default: multiple concurrent POP3
sessions to one mailbox are allowed unless `pop3_lock_session = yes`, in which case a
second connection waits for the lock (2 minutes with Maildir) and otherwise gets
`-ERR [IN-USE] Mailbox is locked by another POP3 session`. The historical problem is
the reverse of ours: a long POP session holding an mbox read-lock while the MDA times
out trying to deliver.

Our stance should be: **one POP3 session per account at a time, ours.** Never open two.
Not because we would race ourselves, but because the user's phone, webmail and our
client all share one maildrop, and a multi-connection design makes us the antisocial
one. Handle `-ERR [IN-USE]` and `-ERR [LOGIN-DELAY]` as *retry later*, not as auth
failure — `LOGIN-DELAY` in particular will otherwise look like a wrong password, and
`AUTH-RESP-CODE` is advertised on this very server, so resp-codes will show up in real
replies.

### When deletions commit — and what we should do about it

Covered in §3: **UPDATE state only, on `QUIT` only.** A dropped connection deletes
nothing. **[RFC 1939 §8]** even advises servers implementing download-and-delete to do
it as "following a POP3 login by a client which was ended by a QUIT, delete all messages
downloaded during the session with the RETR command", and says explicitly:

> It is important not to delete messages in the event of abnormal connection
> termination (ie, if no QUIT was received from the client) because the client may not
> have successfully received or stored the messages.

The same §8 note is the reason some servers cripple TOP:

> Servers implementing a download-and-delete policy may also wish to disable or limit
> the optional TOP command, since it could be used as an alternate mechanism to
> download entire messages.

Not a risk here (this account is `LeaveOnServer::Keep` and the server advertises TOP), but it is the
structural reason TOP is sometimes absent, which is worth knowing when the next preset
is written.

**The rule for us:** commit local state (ingested messages, updated `remote_map`)
**before** issuing the `QUIT` that commits any `DELE`. Order: RETR → durable local
write → DELE marks → QUIT. If we crash between the durable write and the QUIT, nothing
is deleted server-side and the next poll re-sees the message; `MessageKey` dedupe makes
that a no-op. If we crash the other way round, we lose mail. The asymmetry is the whole
argument for that order.

---

## 7. PIPELINING

**[RFC 2449]**

> The PIPELINING capability indicates the server is capable of accepting multiple
> commands at a time; the client does not have to wait for the response to a command
> before issuing a subsequent command. If a server supports PIPELINING, it MUST process
> each command in turn.

RFC 2449's rationale text names our exact use case: POP "has short commands and
sometimes lengthy responses, and there is an advantage in sending new commands while
still receiving the response to an earlier command (for example, sending RETR and/or
DELE commands while processing a UIDL reply)."

**[MEASURED]** `PIPELINING` is advertised by the server.

### Is it safe in practice?

Mostly, with two real hazards documented by client authors:

- **Servers that advertise it and do not implement it.** MailKit issue #174: "servers
  advertise that they support PIPELINING, but don't actually support it" — MailKit sent
  a batch of `RETR`s and the server replied with a single message. Microsoft's POP3
  provider documentation takes the matching stance: batch only if `PIPELINING` is
  advertised, and note that some servers support it without advertising ("experimentation
  may be required" — which we should not do).
- **Termination bugs.** Some servers emit an extra `.CRLF` after a multi-line response,
  which desynchronises a pipelined reader and hangs it waiting for bytes that never
  come. A strict `CRLF.CRLF` terminator scan with byte-unstuffing (**[RFC 1939 §3]**:
  "a multi-line response is terminated with the five octets CRLF.CRLF") plus a
  per-response read timeout is the defence.

### What to pipeline, and how deep

- **Pipeline the header pass aggressively.** `TOP n 0` responses are a few KB each; a
  window of 20–50 outstanding commands turns 2372 × 15 ms of serial latency into a few
  seconds and makes the pass bandwidth-bound. This is where pipelining pays.
- **Pipeline `RETR` shallowly, and bound the window by bytes, not by count.** The
  danger is specific: once you have written N `RETR`s, you are committed to draining N
  responses. You cannot cancel, and there is no way to say "actually, skip that 9.9 MB
  one". A 5-deep pipeline over the 100 largest messages is a 30 MB commitment on one
  socket. Bound the outstanding set at something like 1–2 MB of `LIST`-known size, or
  just do not pipeline any message over ~256 KiB — the latency saving is irrelevant
  next to its transfer time anyway.
- **Never pipeline anything across `DELE`/`QUIT`.** The commit boundary should be a
  quiescent point.
- **Fall back by *not* pipelining**, never by reconnecting-and-retrying blindly. If a
  pipelined batch desynchronises, drop to strict request/response for the rest of the
  account and record that on the account.

---

## 8. RECOMMENDED STRATEGY

### The shape

Four passes, each independently resumable, each committing durable state before moving
on. The inbox is usable after pass 2, which is seconds.

**Pass 0 — session open (≈1 s).**
Connect (implicit TLS, 995 — **[RFC 8314]** prefers implicit TLS over STLS, and this
server offers no STLS anyway). `CAPA`. Uppercase the tags before comparing (Mozilla bug
1783290). `AUTH PLAIN <base64>` with the initial-response form
(**[RFC 5034]** — saves a round trip). Re-read `CAPA` after auth if you want to be
careful; **[RFC 5034]** notes mechanisms may change after STLS, and F14 already
established the after-auth rule for IMAP. `STAT` → `(count, octets)`. `UIDL` → the full
listing (one multi-line response, ~40 KB for 2372). `LIST` → the full listing with exact
octet sizes (another ~25 KB).

Two multi-line responses buy the complete map of the maildrop. **Do this before any
`RETR`, always** — `LIST` is what makes the size-aware plan possible, and fetchmail's
`--fetchsizelimit` exists because someone once did it the other way.

**Pass 1 — reconcile (instant, local).**
Diff the UIDL set against `remote_map`. Three outcomes:
- normal: a few new UIDLs at the top, everything else known;
- **reset detected**: intersection with the stored set is empty/near-empty → do not
  treat 2372 messages as new; set a `uidl_reset` flag that makes pass 2 a *remap* pass
  (§4);
- first sync: nothing known, everything new.

**Pass 2 — the header pass (seconds; ~8 MB). This is the one that makes the inbox
usable.**
If `TOP` is advertised: `TOP n 0` for every unknown message, **pipelined 20–50 deep**,
issued in **descending message number** (newest first). Parse headers, compute
`MessageKey`, run JWZ threading, write `ThreadSummary` rows, ingest.

After this pass the user has all 2372 messages in a correctly threaded, correctly dated
list, with sender and subject — and the server has not marked a single one `\Seen`.
Every row is marked `body: NotFetched`.

Checkpoint every ~200 messages by writing the ingested batch durably. Nothing here can
be lost: no `DELE` has been issued and no `QUIT` is needed.

If `TOP` is not advertised, or errors: skip this pass entirely and let pass 3 do the
work, with the bodies-first ordering below. The inbox then fills over minutes rather
than seconds. Say so in the UI. Do not fall back per-message.

**Pass 3 — bodies, cheapest-and-newest first (~25 MB gets 90 % of messages).**
Order the `RETR` queue by a **size-banded, newest-first** key, using the `LIST` sizes we
already have:
1. band A: everything ≤ 64 KiB, newest first — 2137 messages, **26.6 MB**;
2. band B: 64 KiB – 1 MiB, newest first — 178 messages, 56.5 MB;
3. band C: > 1 MiB, newest first — 57 messages, 184 MB.

Band A is 90 % of the mailbox's *messages* for 10 % of its bytes. On any plausible link
it completes in well under a minute, and at that point every message the user is likely
to open is already local. Bands B and C are background work, interruptible, and the only
place a user ever sees a spinner.

Pipeline within band A; do not pipeline band C.

**Pass 4 — `DELE` and `QUIT`, only if `LeaveOnServer::DeleteAfterFetch`.**
Not applicable to this account (`Keep`). When it is: durable local write **first**, then `DELE`,
then `QUIT`.

### Checkpointing across sessions

Borrow fetchmail's `--expunge` idiom: **end the session and reconnect every N messages
or every M megabytes** — say every 500 messages or 50 MB. Reasons: it bounds how much
in-session progress a dropped connection can cost; it releases the maildrop lock for
other clients; it keeps us far from any server-side session-duration policy; and it
makes "resume" and "poll" the same code path. Message numbers change across sessions,
so **re-`UIDL` on every reconnect and re-derive the numbers** — never carry a message
number across a session boundary.

### What is stored

In `remote_map` / a POP3-side table, keyed by `(account, uidl)`:

| field | why |
|---|---|
| `uidl` | the only cross-session identity POP3 offers |
| `message_id` | the local `MessageId` this UIDL resolves to (many-to-one, per plan.md) |
| `size` | from `LIST`; drives the banding and lets us show "3.2 MB, not downloaded" |
| `state` | `Unknown → HeadersFetched → BodyFetched` (+ `Deferred` for oversize, `Failed{attempts}`) |
| `message_key` | the dedupe key, so a UIDVALIDITY reset is a remap not a refetch |
| `first_seen` | for diagnosing UIDL churn |

Deliberately **not** stored: the message number (session-scoped), the UIDL's internal
structure (opaque, per F17), and any assumption that `remote_map` is a bijection.

Account-level state: the last observed `STAT` pair, the capability set as observed
*after auth*, a `top_usable` flag (set false on the first TOP failure, so we do not
re-probe every poll), a `pipelining_usable` flag with the same latching behaviour, and
the UIDL-set digest used for reset detection.

Everything is a row in SQLite that we already have. There is no separate state file, and
in particular no fetchmail-style flat `.fetchids` — the Debian #798803 O(N²) story is a
good reminder of why a real index matters at this size.

### Failure modes and what each one costs

| failure | cost | why it is safe |
|---|---|---|
| connection drops mid-`RETR` | that one message, re-fetched | no UPDATE state, nothing deleted (**[RFC 1939 §6]**) |
| connection drops mid-header-pass | the un-checkpointed tail (≤200 headers ≈ 600 KB) | same |
| process killed | whatever is not yet committed to SQLite | same |
| server autologout (10 min idle) | the session | same; and we are never idle 10 min mid-sync |
| `-ERR [IN-USE]` | the poll | retry with backoff; do **not** treat as auth failure |
| `-ERR [LOGIN-DELAY]` | the poll | honour the advertised delay; **[RFC 2449]** |
| UIDVALIDITY / UIDL reset | ~8 MB header re-pass, **not** 255 MB | `MessageKey` remap (§4) |
| TOP absent or broken | inbox fills in minutes not seconds | latch `top_usable = false`, fall back to `RETR` for the whole account |
| pipelining desync | the session | latch `pipelining_usable = false`, reconnect, strict request/response |
| duplicate UIDLs in one listing | nothing | RFC permits it; dedupe messages by `MessageKey`, not by UIDL |
| a 9.9 MB message on a flaky link | retry loop, potentially forever | cap attempts, mark `Failed`, surface "not downloaded" in the UI with a retry action — copy fetchmail's `--limit` + `--warnings` behaviour rather than retrying silently |

### What this changes in `plan.md`

- `Pop3Backend` is currently described as "`UIDL` every poll, diff against `remote_map`,
  `RETR` what is new." That is right for steady state and wrong for first sync. It needs
  `LIST` (sizes), `TOP` (headers), and a banded fetch order.
- `AccountCaps` has no POP3 side. It needs at least `top: Supported | Absent` and
  `pipelining: Supported | Absent`, discovered from `CAPA` **after** auth (F14's rule
  generalises), and both must be *latchable to false* by observed misbehaviour, not only
  by advertisement.
- The message model needs a body-presence state. A `ThreadSummary` whose messages are
  headers-only is a legitimate, common, first-class state on POP3 — not an error — and
  the store and the UI both have to say so.
- `SyncCursor::Pop` is described as "no cursor; diff UIDL each poll". That is still true,
  but there is per-message *progress* state that is not a cursor and needs somewhere to
  live (the `state` column above).
- `UidValidity::Reset` should mean "drop and **re-map** via `MessageKey`", not "drop and
  refetch" — for POP3 certainly, and arguably for IMAP too.

---

## 9. Open questions I could not settle from the desk

- **Header size across the corpus.** I have one sample (3,323 B of 24,408 B). The 7–9 MB
  estimate for the header pass rests on it. A second spike run doing `TOP n 0` for 50
  messages and summing would turn the estimate into a measurement, and would also prove
  TOP works on this server for real rather than by advertisement.
- **Whether the server's Dovecot sets `pop3_lock_session`.** Two concurrent sessions would tell
  us; I did not try, because probing a lock on a live personal maildrop is not a thing to
  do casually.
- **The institution's second host.** Still untested, still a guess (F16). Its CAPA may
  differ; nothing here should assume both hosts are the same Dovecot build.
- **Effective throughput.** I measured RTT (~15 ms) but not bandwidth. The wall-clock
  claims above are latency arithmetic plus byte counts, not an end-to-end timing.

---

## Sources

RFCs:
- [RFC 1939 — Post Office Protocol Version 3](https://www.rfc-editor.org/rfc/rfc1939.txt) (§3 states/autologout/multi-line framing, §4 lock, §5 TOP/UIDL, §6 UPDATE, §8 operational notes, §9 command summary)
- [RFC 2449 — POP3 Extension Mechanism](https://www.rfc-editor.org/rfc/rfc2449.txt) (CAPA, PIPELINING, TOP, UIDL, SASL, EXPIRE, LOGIN-DELAY, RESP-CODES, IN-USE)
- [RFC 5034 — POP3 SASL Authentication Mechanism](https://www.rfc-editor.org/rfc/rfc5034.txt) (AUTH with initial response)
- [RFC 8314 — Cleartext Considered Obsolete](https://www.rfc-editor.org/rfc/rfc8314.txt) (implicit TLS preferred)

Server behaviour:
- [Dovecot — POP3 server configuration](https://doc.dovecot.org/2.3/configuration_manual/protocols/pop3_server/) (`pop3_uidl_format` default `%08Xu%08Xv`, `pop3_lock_session`, `pop3_no_flag_updates`, `pop3_client_workarounds`)
- [Dovecot — Timeouts](https://doc.dovecot.org/2.3/admin_manual/timeouts/) (POP3 10-minute idle, 3-minute pre-login)
- [Dovecot commit setting `pop3_uidl_format` default to `%08Xu%08Xv`](https://dovecot.org/list/dovecot-cvs/2007-July/009305.html)
- [Dovecot `src/pop3/pop3-commands.c`](https://github.com/dovecot/core/blob/main/src/pop3/pop3-commands.c) (seen flag gated on the RETR sentinel, so TOP does not set `\Seen`)
- [Dovecot list — POP3 flag updates](https://dovecot.org/list/dovecot/2009-June/040570.html)
- [Plesk forum — POP3 re-download storm after Dovecot UIDL format change (PPP-69700)](https://talk.plesk.com/threads/after-upgrading-to-18-0-73-pop3-re-download-storm-due-to-index-uid-changes-dovecot-2-4-1.389420/)
- [Cyrus list — server migration, UIDL and re-download](https://lists.andrew.cmu.edu/pipermail/info-cyrus/2005-May/000731.html)

Clients (facts read, no code copied; licences noted):
- [fetchmail FAQ](https://www.fetchmail.info/fetchmail-FAQ.html) — GPL-2.0-with-exception. Items I3, I4, I8, S6, S7 on broken TOP; D2/D3, R8, R9 on dropped connections and timeouts.
- [fetchmail manual](https://www.fetchmail.info/fetchmail-man.html) — `--limit`, `--warnings`, `--fetchlimit`, `--fetchsizelimit`, `--fastuidl`, `--expunge`, `--uidl`, `--idfile`, and when TOP is used vs RETR.
- [Debian #798803 — `.fetchids` O(N²) at ~50k entries](https://bugs.debian.org/798803)
- [getmail6 configuration](https://getmail6.org/configuration.html) and [FAQ](https://getmail6.org/faq.html) — GPL-2.0. `oldmail` state file, `BrokenUIDLPOP3Retriever`.
- [Mozilla bug 1783290 — lowercase CAPA breaks "fetch headers only"](https://bugzilla.mozilla.org/show_bug.cgi?id=1783290) — MPL-2.0.
- [Mozilla bug 1796903 — POP3 UIDL ignored, messages downloaded twice](https://bugzilla.mozilla.org/show_bug.cgi?id=1796903)
- [Mozilla bug 421755 — lost `popstate.dat` causes full re-download](https://bugzilla.mozilla.org/show_bug.cgi?id=421755)
- [Mozilla bug 261668 — headers-only: mails disappear from inbox](https://bugzilla.mozilla.org/show_bug.cgi?id=261668)
- [Mozilla bug 1930847 — headers-only + "get selected messages" deletes messages](https://bugzilla.mozilla.org/show_bug.cgi?id=1930847)
- [Mozilla bug 314310 — headers-only + size filter prevents download](https://bugzilla.mozilla.org/show_bug.cgi?id=314310)
- [Mozilla bug 156998 — TOP may not be supported by the server](https://bugzilla.mozilla.org/show_bug.cgi?id=156998)
- [MailKit issue #174 — servers advertise PIPELINING and do not implement it](https://github.com/jstedfast/MailKit/issues/174) — MIT.
- [MailKit issue #82 — unexpected disconnect on large attachments](https://github.com/jstedfast/MailKit/issues/82)
- [JavaMail/Jakarta Mail `com.sun.mail.pop3`](https://javaee.github.io/javamail/docs/api/com/sun/mail/pop3/package-summary.html) — TOP for headers, `mail.pop3.disabletop` escape hatch.
- [Microsoft — Managing message downloads for POP3 accounts](https://learn.microsoft.com/en-us/office/client-developer/outlook/auxiliary/managing-message-downloads-for-pop3-accounts) — UIDL history, PIPELINING batching stance.
- [K-9 Mail POP3 server settings](https://docs.k9mail.app/en/6.400/accounts/incoming_pop3/) — Apache-2.0.

Measurements from this project:
- `spike/out/pop3.trace` (gitignored) — CAPA, STAT, full UIDL, full LIST, one RETR.
- TCP connect to the server's port 995: 15–38 ms over 3 samples, 2026-09-22.
