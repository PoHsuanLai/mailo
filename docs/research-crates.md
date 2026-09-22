# Crate evaluation for mailo

Everything below was verified against the crates.io JSON API and the published `.crate`
tarballs on **2026-09-22**. Nothing here is from memory. Where a claim is about code
behaviour, the source was downloaded and read.

Workspace licence is `MIT OR Apache-2.0`. A dependency that is not `MIT`, `Apache-2.0`, or a
permissive dual/multi-licence is disqualified.

---

## Verified table

| Crate | Latest stable | Licence | Total dl | Recent dl | Last release | Repository | Verdict |
|---|---|---|---|---|---|---|---|
| `imap-proto` | **0.16.7** | MIT OR Apache-2.0 | 3,494,418 | 1,676,409 | 2026-04-21 | github.com/djc/tokio-imap | **ADOPT** |
| `imap-codec` | 1.0.0 (dead); 2.0.0-alpha.9 live | MIT OR Apache-2.0 | 108,654 | 29,821 | 1.0.0: 2023-08-22 / alpha.9: 2026-07-19 | github.com/duesee/imap-codec | **AVOID** |
| `imap-types` | 1.0.0 (dead); 2.0.0-alpha.7 live | MIT OR Apache-2.0 | 108,098 | 30,934 | alpha.7: 2026-07-19 | github.com/duesee/imap-codec | AVOID (with `imap-codec`) |
| `imap-next` | 0.3.4 | MIT OR Apache-2.0 | 53,968 | 15,668 | 2026-02-18 | github.com/duesee/imap-next | AVOID (rides `imap-codec` alpha) |
| `io-imap` | 0.6.0 | MIT OR Apache-2.0 | 6,173 | 6,009 | 2026-08-22 | github.com/pimalaya/io-imap | AVOID (rides `imap-codec` alpha) |
| `io-smtp` | 0.3.0 | MIT OR Apache-2.0 | 5,929 | 5,602 | 2026-08-15 | github.com/pimalaya/io-smtp | Keep as candidate |
| `io-sasl` | 0.1.0 | MIT OR Apache-2.0 | 4,180 | 4,180 | 2026-08-15 | github.com/pimalaya/io-sasl | WRITE OURSELVES |
| `io-oauth` | 0.3.0 | MIT OR Apache-2.0 | 3,441 | 2,020 | 2026-08-15 | github.com/pimalaya/io-oauth | AVOID (drags `io-http`) |
| `io-http` | 0.5.0 | MIT OR Apache-2.0 | 10,516 | 7,282 | 2026-08-15 | github.com/pimalaya/io-http | AVOID |
| `oauth2` | **5.0.0** | MIT OR Apache-2.0 | 51,856,623 | 13,452,311 | 2025-01-21 | github.com/ramosbugs/oauth2-rs | **ADOPT** (`default-features = false`) |
| `utf7-imap` | 0.3.2 | MIT | 90,844 | 27,050 | **2022-09-26** | github.com/iam-medvedev/rust-utf7-imap | **WRITE OURSELVES** |
| `base64-ng-imap` | 2.0.3 | (unchecked) | 68 | 68 | 2026-09-04 | — | Disqualified on maturity |
| `rsasl` | 2.3.1 | Apache-2.0 OR MIT | 1,309,467 | 600,489 | 2026-04-23 | codeberg.org/dequbed/rsasl | WRITE OURSELVES |
| `sasl` | 0.5.2 | **MPL-2.0** | 121,523 | 17,869 | 2024-07-22 | gitlab.com/xmpp-rs/xmpp-rs | **DISQUALIFIED — licence** |
| `blake3` | **1.8.7** | CC0-1.0 OR Apache-2.0 OR Apache-2.0-WITH-LLVM-exception | 185,452,710 | 44,968,737 | 2026-08-20 | github.com/BLAKE3-team/BLAKE3 | **KEEP** |
| `mail-parser` | **0.11.9** | Apache-2.0 OR MIT | 3,716,084 | 1,480,479 | 2026-09-09 | github.com/stalwartlabs/mail-parser | **KEEP + `full_encoding`** |
| `mail-builder` | **1.0.0** | Apache-2.0 OR MIT | 1,328,106 | 433,020 | 2026-09-12 | github.com/stalwartlabs/mail-builder | KEEP |
| `ammonia` | **4.2.0** | MIT OR Apache-2.0 | 16,239,542 | 4,072,875 | 2026-09-17 | github.com/rust-ammonia/ammonia | KEEP |
| `rusqlite` | **0.40.2** | MIT | 109,704,289 | 35,230,464 | 2026-08-08 | github.com/rusqlite/rusqlite | KEEP |
| `keyring` | **4.2.0** | MIT OR Apache-2.0 | 26,257,167 | 11,312,270 | 2026-08-29 | github.com/open-source-cooperative/keyring-rs | KEEP |
| `notify-rust` | **4.18.0** | MIT OR Apache-2.0 | 14,650,493 | 4,684,819 | 2026-06-16 | github.com/hoodie/notify-rust | KEEP |
| `chardetng` | **1.0.0** | Apache-2.0 OR MIT | 12,957,218 | 4,581,390 | 2026-03-30 | github.com/hsivonen/chardetng | **ADOPT** |
| `charset` | 0.1.5 | Apache-2.0 OR MIT | 24,388,608 | 4,945,367 | 2024-07-21 | github.com/hsivonen/charset | SKIP (redundant) |
| `encoding_rs` | 0.8.41 | (Apache-2.0 OR MIT) AND BSD-3-Clause | 534,415,112 | 108,530,915 | 2026-09-09 | github.com/hsivonen/encoding_rs | Transitive, fine |
| `html2text` | 0.17.1 | MIT | 6,106,913 | 1,556,055 | 2026-04-19 | github.com/jugglerchris/rust-html2text | SKIP (redundant) |
| `nanohtml2text` | 0.3.0 | MIT | 483,564 | 171,178 | 2026-08-27 | github.com/alexwennerberg/nanohtml2text | SKIP (redundant) |
| `email_address` | 0.2.9 | MIT | 67,443,188 | 24,460,281 | 2024-07-31 | github.com/johnstonskj/rust-email_address | SKIP |
| `lettre` | 0.11.23 | MIT | 18,140,123 | 6,198,768 | 2026-08-03 | github.com/lettre/lettre | Fallback only (owns I/O) |
| `mail-send` | 0.6.2 | Apache-2.0 OR MIT | 529,068 | 83,730 | 2026-08-18 | github.com/stalwartlabs/mail-send | Fallback only (owns I/O) |
| `async-imap` | 0.11.3 | MIT OR Apache-2.0 | 1,633,523 | 804,002 | 2026-07-17 | github.com/async-email/async-imap | Last resort (owns I/O) |
| `imap` | 2.4.1 | Apache-2.0/MIT | 1,863,690 | 839,624 | **2021-01-13** | github.com/jonhoo/rust-imap | Unmaintained — skip |
| `mailparse` | 0.17.0 | 0BSD | 13,846,700 | 3,648,999 | 2026-09-06 | github.com/staktrace/mailparse | Not needed |

Note on "latest stable": for `imap-codec`/`imap-types`, `max_stable_version` is **1.0.0**
(published 2023-08-22) but every crate that depends on them today requires
`^2.0.0-alpha.*`. The 1.0 line is not maintained.

---

## 1. IMAP modified UTF-7 (RFC 3501 §5.1.3) — **WRITE OURSELVES**

This is real and confirmed by our own spike. The two folders in FINDINGS F15 decode to:

```
&V4NXPpD1TvY-  ->  垃圾郵件   (Spam)
&kc2JgZD1TvY-  ->  重要郵件   (Important)
```

### `utf7-imap` 0.3.2 — reject

Licence MIT (permissive, so not disqualified on licence). 90,844 lifetime downloads, but
only **4 reverse dependencies**, one of which is pimalaya's own superseded `email-lib`. Last
release **2022-09-26 — four years ago**. Author `iam-medvedev` + one contributor.

I downloaded and read the whole crate: it is **189 lines in one `lib.rs`**. Specific defects:

1. **Remote-triggerable panic.** `decode_utf7_part` ends with
   `base64::decode(text_b64).unwrap()`. The decoder finds runs with the regex `&([^-]*)-`,
   which accepts *any* bytes between `&` and `-`. A mailbox name containing `&A-` or `&!!-`
   — a two-to-four byte string a buggy or hostile server can send in a `LIST` response —
   makes base64 return `InvalidLength` / `InvalidByte`, and the `unwrap` aborts the process.
   Verified the failing inputs: `&A-`, `&AAAAA-` (length ≡ 1 mod 4), `&!!-` (invalid byte).
   This is untrusted network data reaching an `unwrap`.
2. **No error type at all.** `decode_utf7_imap(String) -> String`. There is no way to
   distinguish "decoded" from "gave up". A malformed name is either a panic or silent
   mojibake, and we can never surface it.
3. **Wrong ASCII range.** `is_ascii_custom` is `(0x20..=0x7f)`. RFC 3501 requires printable
   US-ASCII `0x20..=0x7e` to pass through and everything else to be encoded; `0x7f` (DEL) is
   passed through raw. Minor, but it is a spec deviation in an 189-line crate.
4. **Dependency set from 2022.** `base64 0.13`, `regex 1.6`, `encoding_rs 0.8`. `regex` for a
   five-state scanner pulls in `regex-automata` + `aho-corasick` + `memchr`; `base64 0.13`
   will sit in the tree alongside whatever modern base64 we use elsewhere.
5. **`String` by value on both directions**, and `format!` in a loop — an allocation and an
   O(n²) rebuild per mailbox name, including the overwhelmingly common pure-ASCII case.

### Alternatives

There are none. `base64-ng-imap` 2.0.3 has **68 lifetime downloads** and first shipped
2026-09-04 — disqualified on exactly the maintenance-risk grounds that killed
`mail-threading`. `imap-types`/`imap-codec` explicitly do **not** do it: issue #370 ("Handle
{De,En}coding of Mailbox") is still open and parked in a future milestone. `imap-proto` has no
UTF-7 code either (grepped the source: zero hits).

### Recommendation

Write it in `mail-mime` (or a `mail-proto::utf7` module) as roughly **80 lines, zero new
dependencies**:

```rust
pub fn encode(name: &str) -> String;               // infallible
pub fn decode(raw: &str) -> Result<String, Utf7Error>;   // fallible, never panics
```

Encode is a shift-state loop over `chars()` with a hand-written modified-base64 alphabet
(`A-Za-z0-9+,`). Decode is the same loop in reverse with explicit length and alphabet
validation. Neither needs `base64` (the alphabet differs by one character: `,` for `/`),
`regex`, or `encoding_rs` (`char::decode_utf16` is in `core`).

This is the one place where the brief's "hand-rolling is a known source of subtle bugs" is
true, so pay for it properly:

- Lift `utf7-imap`'s six unit vectors — it is MIT, attribute in a comment. They cover the
  three hard cases: a fully-encoded run, runs split by ASCII (`&AWA-iuk&AWE-liad&ARcBfgEX-`),
  and consecutive accents in one run (`th&AOkA4g-tre`).
- Add the two real fixtures above from `spike/out/`.
- Keep the round-trip proptest `decode(encode(s))? == s`.
- Add the proptest `utf7-imap` does not have: **`decode(arbitrary_bytes)` never panics.**
  That is the property their design cannot express.

The decision is not "the encoding is easy". It is that a 189-line unmaintained crate with a
network-reachable `unwrap` and no `Result` is *more* correctness risk than 80 lines we own,
test, and can fix.

---

## 2. IMAP protocol types / parsing — **ADOPT `imap-proto` 0.16.7; drop `imap-codec`/`imap-next`**

This is the significant finding of this review, and it contradicts the current plan.

### The plan's two options are not independent

`plan.md` lists `io-imap` 0.6 as the primary and `imap-codec` + `imap-next` as the fallback,
and the risk notes treat the fallback as mitigation. Checked the dependency manifests:

```
io-imap   0.6.0  ->  imap-codec ^2.0.0-alpha.8   (+ io-sasl ^0.1)
imap-next 0.3.4  ->  imap-codec ^2.0.0-alpha.7
```

**Both sit on the same pre-release parser.** Swapping `io-imap` for `imap-next` changes the
event-loop wrapper and keeps 100% of the protocol-parsing risk. The plan's stated hedge does
not hedge the thing that can actually be wrong.

It also means committing to a `2.0.0-alpha.N` in `Cargo.toml`. Cargo does not unify across
pre-release identifiers the way it does across patch versions, so an alpha bump in either
crate is a manual coordination step, and `cargo update` can silently stop resolving.

### `imap-codec` cannot parse Gmail's FETCH responses

Downloaded `imap-types-2.0.0-alpha.7` and `imap-codec-2.0.0-alpha.9` and read them.

`imap_types::fetch::MessageDataItem` has these variants and **no catch-all**:
`Body`, `BodyExt`, `BodyStructure`, `Envelope`, `Flags`, `InternalDate`, `Rfc822`,
`Rfc822Header`, `Rfc822Size`, `Rfc822Text`, `Uid`, `Binary`, `BinarySize`, `ModSeq`.
`grep -ril 'x-gm\|gmail' src/` over the whole crate returns exactly one hit — a doc-comment
URL in `auth.rs` about XOAUTH2. The extension modules present are: binary, compress,
condstore_qresync, enable, idle, metadata, move, namespace, quota, sort, thread, uidplus,
unselect, utf8. **There is no Gmail extension module.**

And the parser fails closed rather than skipping:

```rust
// imap-codec-2.0.0-alpha.9/src/fetch.rs:126
pub(crate) fn msg_att(input: &[u8]) -> IMAPResult<&[u8], Vec1<MessageDataItem>> {
    delimited(
        ...
        separated_list1(sp, alt((msg_att_dynamic, msg_att_static))),
        ...
    )
}
```

An unrecognised item ends the `separated_list1`, the closing `)` then fails to match, and the
**entire untagged FETCH response is a parse error** — not a dropped attribute.

Consequences for our design, all of them load-bearing:

- `MessageKey::Gmail(u64)` (X-GM-MSGID) is unimplementable.
- `ServerThreads::ProviderId` (X-GM-THRID) is unimplementable.
- `ImapBackend`'s X-GM-LABELS mapping is unimplementable.
- The exact `FETCH 1:5 (UID FLAGS ENVELOPE BODYSTRUCTURE X-GM-MSGID X-GM-THRID X-GM-LABELS)`
  the phase-0 spike ran — and used to prove the many-to-one `remote_map` claim — would not
  parse.

There is no workaround short of forking `imap-codec`. X-GM-MSGID is a FETCH data item, not a
header, so it cannot be smuggled through `BODY.PEEK[HEADER.FIELDS (...)]`.

### `imap-proto` 0.16.7 does exactly what we need

- **Licence** MIT OR Apache-2.0. **3,494,418** lifetime downloads, **1,676,409** recent.
  Released **2026-04-21**. Maintainer is Dirkjan Ochtman (`djc` — also rustls, quinn,
  instant-acme, hickory-dns). This is the strongest maintenance profile of any IMAP crate on
  crates.io by an order of magnitude, and it is the parser under both `async-imap` and
  `rust-imap`, so it is exercised against real servers daily.
- **One dependency: `nom ^7`.** Nothing else. No tokio, no async, no sockets — verified.
- **Sans-I/O in exactly our shape.** The public entry point is
  `pub fn parse_response(msg: &[u8]) -> ParseResult<'_>` where
  `ParseResult<'a> = IResult<&'a [u8], Response<'a>>`. It uses `nom::bytes::streaming`, so a
  short buffer returns `Err::Incomplete` — which maps one-to-one onto
  `Progress::Need(vec![IoNeed::Read])`. Borrowed `&'a [u8]` output means zero-copy into our
  buffer. This is a better fit for `Machine` than either alternative, because it is a *pure
  function over bytes* rather than a state machine with its own opinion about the loop.
- **Extension coverage** (from `src/parser/`): `gmail.rs` (X-GM-MSGID, X-GM-THRID,
  X-GM-LABELS — for both FETCH attributes and LIST data), `rfc4551` (CONDSTORE),
  `rfc7162` (QRESYNC), `rfc4315` (UIDPLUS — the COPYUID/APPENDUID we need for MOVE and
  APPEND), `rfc5161` (ENABLE), `rfc5256` (SORT/THREAD), `rfc2971` (ID), `rfc5464` (METADATA),
  `rfc2087` (QUOTA), `rfc4314` (ACL), plus a dedicated `bodystructure.rs`.
  `AttributeValue` carries `GmailMsgId`, `GmailThrId`, `GmailLabels`, `ModSeq`, `Envelope`,
  `BodyStructure`, `Uid`, `Flags`, `InternalDate`, `BodySection`, `Rfc822*` — and is
  `#[non_exhaustive]`.
- **5,807 lines of source** — small enough to read when something goes wrong.

Costs, stated honestly:

- **Only 14.6% documented** (docs.rs's own figure). We will read source. Mitigated by size.
- **`builders` is thin** — `command::{check, close, examine, fetch, list, login, select,
  uid_fetch}` with a typestate builder for FETCH/SELECT. It does not cover STORE, APPEND,
  MOVE, IDLE, AUTHENTICATE, SEARCH. **We write command serialisation ourselves.** That is
  fine and arguably preferable: commands are trivial to emit (`tag SP verb SP args CRLF`),
  they are where our `IoNeed::Write` needs to be byte-exact against the phase-0 traces, and
  we need our own literal/continuation handling anyway. Parsing is where the bugs live, and
  parsing is what we are buying.
- `nom 7` rather than `nom 8`. Still maintained, widely used, no security concern.
- No MOVE (RFC 6851) parser module — not needed: MOVE's response is `COPYUID` (UIDPLUS,
  covered) plus a tagged OK.

### Where that leaves the pimalaya crates

`io-imap` is genuinely sans-I/O — docs.rs: "every command exchange is a resumable state
machine emitting read and write requests instead of performing I/O itself, so the caller owns
the socket and pumps the coroutine", yielding `ImapYield::{WantsRead, WantsWrite}`. tokio is a
**dev**-dependency only. Architecturally it is a clean match for `Machine`/`Progress`. But
6,173 lifetime downloads, one maintainer, seven 0.x releases in three months (0.1.0 in June
2026 → 0.6.0 in August 2026), and it inherits the Gmail blindness above. The design fits; the
parser underneath does not.

**Recommendation:** `imap-proto` for parsing; `ImapSession`/`ImapBackend` and command
serialisation are ours. This is *more* of our own code than the plan assumed, but it is the
easy half, and it removes both the alpha-version dependency and the Gmail blocker.

---

## 3. SASL (PLAIN, LOGIN, XOAUTH2, CRAM-MD5) — **WRITE OURSELVES**

- `sasl` 0.5.2 — **MPL-2.0. Disqualified on licence.** Moving on.
- `rsasl` 2.3.1 — Apache-2.0 OR MIT, 1,309,467 downloads, 600,489 recent, 2026-04-23. Healthy
  and permissive, so this is a judgement call, not a disqualification. It supports PLAIN,
  LOGIN, CRAM-MD5, SCRAM, GSSAPI, OAUTHBEARER, feature-gated, and is sans-I/O
  (`SASLClient` → `start_suggested()` → `Session::step64()`). The reasons not to take it:
  it is a *framework* — config objects, callbacks, mechanism negotiation, a `Validation`
  system — built for servers and middleware that must support arbitrary mechanisms. We need
  four fixed mechanisms chosen by an `AccountPlan` field we already have (`SaslMech`), and
  our `Progress`/`IoReady` loop already *is* the step machinery. Adopting it means mapping
  our own state machine onto theirs.
  Decisive: **XOAUTH2 is not in it.** OAUTHBEARER (RFC 7628) is a different wire format from
  Google's XOAUTH2, and XOAUTH2 is the one Gmail IMAP and SMTP actually require. So we write
  the mechanism we most need regardless.
- `io-sasl` 0.1.0 — 4,180 downloads, a single release from 2026-08-15. Too new to trust for
  auth. (It arrives transitively if we ever take `io-imap`.)

### What we actually write

```
PLAIN     base64("\0" + user + "\0" + pass)                       ~6 lines
LOGIN     two base64 challenge/response steps                     ~10 lines
XOAUTH2   base64("user=" + u + "\x01auth=Bearer " + t + "\x01\x01")  ~6 lines
CRAM-MD5  base64(user + " " + hex(hmac_md5(pass, challenge)))     ~12 lines
```

Plus mechanism selection off `AuthPlan.sasl` and the AUTHENTICATE continuation handling that
belongs in `ImapSession`/`SmtpSession` anyway. Call it **60–80 lines**.

New dependencies: `base64` (already needed for IMAP literals and MIME), and for CRAM-MD5
`hmac` + `md-5` (RustCrypto, both `MIT OR Apache-2.0`). Both are tiny.

One real caveat worth writing into the plan: **CRAM-MD5 sends a password-derived MAC over the
wire and is strictly worse than PLAIN inside TLS.** Our `Tls` enum has no plaintext-by-accident
path, so PLAIN-inside-TLS is always available. The spike (F16) found the campus POP3 server offers
`SASL PLAIN` and `USER` only — no LOGIN, no CRAM-MD5. Gmail wants XOAUTH2. **Neither of our
two real accounts needs CRAM-MD5.** Consider dropping `SaslMech::CramMd5` from v1 entirely
and adding it when a server demands it; that is the cheapest correct answer.

---

## 4. OAuth2 / PKCE — **ADOPT `oauth2` 5.0.0, but move it to `mail-runtime`**

### `oauth2` 5.0.0

MIT OR Apache-2.0. **51,856,623** lifetime downloads, **13,452,311** recent. Released
2025-01-21 (20 months ago — stale, but 5.0.0 is a settled major from a maintainer who also
owns `openidconnect`; there is nothing pending that we need).

The API fits our rule that the HTTP call happens in our runtime. In v5 the HTTP client is a
trait, not a hard dependency:

- `SyncHttpClient` — blanket-implemented for any `Fn(HttpRequest) -> Result<HttpResponse, E>`
- `AsyncHttpClient` — the async equivalent
- `HttpRequest` / `HttpResponse` are aliases for `http::Request<Vec<u8>>` /
  `http::Response<Vec<u8>>`

`reqwest` **is** a default feature, so we must set `default-features = false`. Then `oauth2`
builds the request and parses the response, and `mail-runtime` performs it. PKCE is
first-class: `PkceCodeChallenge::new_random_sha256()`, `.set_pkce_challenge()`,
`.set_pkce_verifier()`. `CsrfToken` covers the `state` parameter the plan already requires.

Dependency-hygiene note: `oauth2` 5.0.0 pins `thiserror ^1` (we standardise on 2),
`rand ^0.8`, `getrandom ^0.2`, `sha2 ^0.10`, `base64 >=0.21,<0.23`. Expect duplicate versions
in the tree. Bloat, not breakage.

### The plan needs a structural correction here

`plan.md` lists `OauthPkce` as a `Machine` in `mail-proto` with `Out = Credential`. That does
not work with `oauth2`, because `oauth2` emits a **structured `http::Request`**, not socket
bytes, and `IoNeed` has no variant for it. The choices are:

1. Add `IoNeed::Http(http::Request<Vec<u8>>)` and keep `OauthPkce` in `mail-proto`.
2. Use `io-oauth`, which *does* emit raw HTTP bytes over `io-http` and fits `IoNeed::Write` +
   `IoNeed::OpenTls` as-is.
3. **Delete `OauthPkce` from the `mail-proto` machine table and put OAuth in `mail-runtime`,
   where reqwest already lives.**

Take option 3. Option 2 means taking `io-oauth` 0.3.0 (3,441 downloads) *plus* `io-http` 0.5.0
(10,516 downloads) *plus* an optional `rsa ^0.10.0-rc.18` — i.e. hand-rolling HTTP/1.1 framing,
chunked encoding, redirects and certificate verification against Google's token endpoint to
preserve a purity rule. reqwest already does all of that correctly. Option 1 makes `IoNeed`
carry an HTTP type that only one machine ever uses.

The principle behind sans-I/O here is "protocol code must be testable without a socket". A
token exchange is a single stateless request/response to a well-known HTTPS endpoint, not a
stateful line protocol with interleaved untagged responses — it has nothing to replay from a
trace. It does not need to be a `Machine`. The loopback redirect listener is already in
`mail-runtime`; the token exchange belongs next to it.

---

## 5. Content hashing — **KEEP `blake3` 1.8.7**

Latest stable 1.8.7, released 2026-08-20. **185,452,710** lifetime / **44,968,737** recent
downloads — one of the most-used crates in the ecosystem, maintained by the BLAKE3 team
(Jack O'Connor, Samuel Neves, Zooko, Jean-Philippe Aumasson).

**Licence: `CC0-1.0 OR Apache-2.0 OR Apache-2.0 WITH LLVM-exception`.** Not MIT, but it is a
triple-licence that *offers* Apache-2.0, so it is compatible with our `MIT OR Apache-2.0`
workspace and is not disqualified. (The CC0 option is on some corporate deny-lists over patent
grants; irrelevant here because Apache-2.0 is offered alongside it.)

The choice is right for both uses:

- **`MessageKey::Synthetic([u8;32])`** — blake3's native output is exactly 32 bytes, so no
  truncation decision to get wrong.
- **Hash-addressed blobs** under `~/.local/share/mailo/blobs/` — blake3 is several times
  faster than SHA-256 on the multi-megabyte attachments that dominate this workload, and its
  tree structure means we can hash a blob incrementally as it streams off the wire.

One build note: `cc ^1.1.12` is a **non-optional** dependency (SIMD assembly), so a C compiler
is required. Fine on Fedora; if it ever becomes awkward, the `pure` feature drops it.

Neither use needs collision resistance against an adversary who picks both inputs, so SHA-256
buys nothing here and costs throughput.

---

## 6. Gaps in the current plan

### 6a. `mail-parser` needs `features = ["full_encoding"]` — **ADOPT the feature**

`plan.md` lists `mail-parser 0.11.9` with no features. Read
`src/decoders/charsets/mod.rs`: the built-in table covers ASCII, UTF-8/16/7, ISO-8859-*,
windows-125*, KOI8, macintosh, ibm850. Everything else is behind `#[cfg(feature =
"full_encoding")]` (which turns on `encoding_rs`, itself an optional dependency):

```
shift_jis, big5, euc-jp, euc-kr, iso-2022-jp, gbk, gb18030,
x-mac-cyrillic, x-user-defined, iso-2022-kr, hz-gb-2312
```

**`big5` is in that list.** Our second account is a campus POP3 mailbox in Taiwan, whose
2,372-message maildrop (F17) will contain Big5 mail. With default features those bodies decode
to mojibake with no error. One-word fix, but it has to be written down.

### 6b. Mislabelled charsets — **ADOPT `chardetng` 1.0.0**

`mail-parser` decodes what the `Content-Type: charset=` label *says*. Mail lies routinely —
Big5 labelled `us-ascii`, windows-1252 labelled `iso-8859-1`, no label at all. The plan has
nothing for this.

`chardetng` 1.0.0, Apache-2.0 OR MIT, **12,957,218** downloads / **4,581,390** recent, updated
2026-03-30, by Henri Sivonen (Mozilla's encoding lead — this is Firefox's detector, ported).
Deps: `encoding_rs`, `memchr`, `cfg-if`. It answers exactly the right question:
`EncodingDetector::feed(bytes); .guess(tld_hint, allow_utf8) -> &'static Encoding`.

Use it as a **fallback only**: trust the label, and run the detector when the label is absent,
unknown, or when decoding under the label produces replacement characters. Never override a
label that decodes cleanly.

`charset` 0.1.5 (same author) is **redundant** for us — it maps MIME charset labels to
`encoding_rs::Encoding` and adds UTF-7 body decoding, and `mail-parser` already ships a
255-entry label map *and* its own `decoder_utf7` (verified: `"csutf7"`, `"utf_7"` both present,
and not behind `full_encoding`). Skip it.

### 6c. Malformed RFC 5322 `Date:` headers — **no crate needed**

`mail-parser` has `src/parsers/fields/date.rs`, 568 lines, with an explicit "4.3 obsolete date
and time" section (obsolete two-digit years, alphabetic zone names, whitespace and comment
handling), a `DateTime::is_valid()` predicate and `to_timestamp()`. That is a tolerant parser
already.

Do **not** reach for `chrono::DateTime::parse_from_rfc2822` as a primary — it is strict and
rejects the obsolete forms that real mail is full of. Use `mail-parser`'s, check `is_valid()`,
and fall back to the message's IMAP `INTERNALDATE` (which we fetch anyway) when it fails.
That fallback is better than any parser, because it is a real timestamp rather than a guess.

### 6d. Address parsing — **no crate needed**

`mail-parser` has `src/parsers/fields/address.rs`, handling groups, angle-addr, comments and
RFC 2047 encoded display names, producing name + address. That is exactly our
`Address { name, email }`.

`email_address` 0.2.9 (MIT, 67M downloads, last released 2024-07-31) *validates* a single
address — it does not parse headers. The only place it would help is validating what the user
types into compose. That is a UI nicety; a `@` check plus what the SMTP server says is enough
for v1. Skip.

### 6e. HTML-to-text for snippets — **no crate needed**

`mail_parser::decoders::html::html_to_text(&str) -> String` is **public** and already in a
crate we depend on. Verified in the source.

So skip both candidates: `html2text` 0.17.1 (MIT, 6.1M downloads, but a full renderer with
tables, link footnotes and width-based wrapping — wrong tool for 140 characters) and
`nanohtml2text` 0.3.0 (MIT, zero-dep, a reasonable crate — just redundant).

Snippets only need this at all when a message has no `text/plain` alternative.

### 6f. SMTP

Unchanged advice: `io-smtp` 0.3.0 (MIT OR Apache-2.0, 5,929 downloads, 2026-08-15) is the only
sans-I/O option and carries the same single-maintainer 0.x risk as the rest of the pimalaya
set — but SMTP is a much smaller surface than IMAP, and unlike `io-imap` it does not drag in a
parser with a known blind spot. `lettre` 0.11.23 (MIT — permissive, fine) and `mail-send`
0.6.2 both own their I/O and cannot sit in `mail-proto`.

Given how small SMTP submission is (EHLO, AUTH, MAIL/RCPT/DATA, and the enhanced status codes
for `Retry`), writing `SmtpSession` ourselves is a serious option and is consistent with
writing `Pop3Session` and `ImapSession`. Decide it alongside the IMAP decision, not separately.

---

## Summary

**ADOPT**

| Crate | Version | Licence | Why it beats hand-rolling |
|---|---|---|---|
| `imap-proto` | 0.16.7 | MIT OR Apache-2.0 | 5,800 lines of battle-tested nom parsers for BODYSTRUCTURE, ENVELOPE, CONDSTORE, QRESYNC, UIDPLUS **and the Gmail extensions**; `nom`-only, streaming, borrowed output; fits `Progress::Need(Read)` exactly. Writing a correct BODYSTRUCTURE parser is weeks. |
| `oauth2` | 5.0.0 | MIT OR Apache-2.0 | 51M downloads; PKCE, CSRF state, token/refresh/error handling done right; `default-features = false` keeps the HTTP call ours. Getting OAuth error semantics subtly wrong is a silent-reauth-loop bug. |
| `chardetng` | 1.0.0 | Apache-2.0 OR MIT | Firefox's charset detector. Statistical detection is not something to reimplement. |
| `mail-parser` feature `full_encoding` | 0.11.9 | Apache-2.0 OR MIT | Not a new crate — a feature flag we are currently missing, and Big5 mail depends on it. |
| `blake3` | 1.8.7 | CC0-1.0 OR Apache-2.0 OR Apache-2.0-WITH-LLVM-exception | Keep. Apache-2.0 option satisfies our licence. |

**WRITE OURSELVES**

| Thing | Size | Why not the crate |
|---|---|---|
| IMAP modified UTF-7 | ~80 lines | `utf7-imap` 0.3.2 is unmaintained since 2022, has no `Result`, and `unwrap`s a base64 decode on server-controlled input (`&A-` panics). Nothing else exists above 68 downloads. |
| SASL PLAIN/LOGIN/XOAUTH2/CRAM-MD5 | ~60–80 lines | `sasl` is MPL-2.0 (disqualified). `rsasl` is a negotiation framework for a problem we do not have and **has no XOAUTH2**. `io-sasl` is one release old. |
| IMAP command serialisation | small | `imap-proto::builders` covers 8 commands; we need STORE/APPEND/MOVE/IDLE/AUTHENTICATE/SEARCH and byte-exact output against phase-0 traces. |
| `Pop3Session` | 200–400 lines | Already decided. Confirmed: no good sans-I/O POP3 crate exists. |
| JWZ threading | ~300 lines | Already decided and already written. |
| `SmtpSession` | moderate | Worth reconsidering against `io-smtp` once the IMAP decision lands. |

**DISQUALIFIED ON LICENCE:** `sasl` 0.5.2 (MPL-2.0).

**MISTAKES IN THE CURRENT PLAN**

1. **`imap-codec` / `imap-next` / `io-imap` cannot parse Gmail FETCH responses.** No Gmail
   extension module, no catch-all `MessageDataItem` variant, and `separated_list1` inside
   `delimited(...)` fails the whole response on an unknown attribute. This kills
   `MessageKey::Gmail`, `ServerThreads::ProviderId`, X-GM-LABELS, and the phase-0 FETCH the
   spike already ran.
2. **The stated fallback is not a fallback.** `io-imap` 0.6 and `imap-next` 0.3.4 both depend
   on `imap-codec ^2.0.0-alpha.*`. Swapping one for the other retains all the parser risk.
3. **The `imap-codec 1.0.0` row is misleading.** 1.0.0 shipped 2023-08-22 and nothing depends
   on it; all live work is `2.0.0-alpha.9`. Adopting the stack means pinning a pre-release.
4. **`OauthPkce` cannot be a `mail-proto` `Machine` if we use `oauth2`** — `oauth2` emits
   `http::Request`, not bytes. Move OAuth to `mail-runtime`.
5. **`mail-parser` is listed without `full_encoding`**, which silently breaks Big5/GBK/
   Shift_JIS/EUC-KR bodies.
6. **Modified UTF-7 is still absent from the dependency table** although FINDINGS F15 recorded
   it. Neither `imap-proto` nor `imap-codec` provides it; it must be an explicit line item.
