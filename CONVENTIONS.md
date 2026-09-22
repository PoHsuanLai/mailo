# Conventions

Binding on every contributor, human or agent. These exist so that work happening in parallel
behind frozen signatures composes without integration surprises. `plan.md` says *what* we are
building and *why*; this file says *how the code is written*.

If a convention here blocks you, say so in your report. Do not quietly deviate — a silent
deviation in a shared type is exactly the failure this file prevents.

---

## 0. Design style

The rules below are the general shape of the code. Everything after this section is a specific
application of them, so when a later rule seems arbitrary, this is why it exists.

### Prefer pure functions

A function that takes values and returns values can be tested with a table, reasoned about in
isolation, and reused from a context its author never imagined. One that reaches for a socket,
a clock or a global cannot.

Push effects to the edges. The layering already encodes this: `mail-domain`, `mail-mime` and
`mail-proto` are pure and mechanically kept that way by `scripts/check-boundary.sh`;
`mail-store` may touch the disk; only `mail-runtime` may open a socket, spawn a task or read
the clock. When a pure crate seems to need an effect, that is a sign the effect belongs to the
caller — `Op::apply` returns a `RemoteIntent` rather than performing one, and `Machine::feed`
returns an `IoNeed` rather than satisfying it.

Effects belong in the signature. `&mut self`, `Result`, or living in a crate permitted to do
I/O are how a reader sees that something happens; a function that looks pure and is not is
worse than one that is honestly imperative.

### Separate data from logic

Data types describe what is true. Functions describe what follows from it. Keep them apart.

A struct's own `impl` should hold construction, derivation and accessors — the things that are
*about* that value. Anything that coordinates several values, or makes a decision, reads better
as a free function or as an `impl` on the type that owns the decision. `ThreadSummary::derive`
belongs on `ThreadSummary` because it *is* a `ThreadSummary`; the reconciliation rule belongs
in `mail-store` because it is about the relationship between a patch and an ingest, and lives
on neither.

Avoid the type that grows methods because it was convenient rather than because they belong.
If half a struct's methods never touch half its fields, it is two types.

### Make illegal states unrepresentable

Encode invariants in types rather than checking them at runtime and hoping every caller
remembers. This is the whole reason the domain is enums.

- Newtype every identifier and every unit. `AccountId`, not `Uuid`; `BlobId`, not `String`.
  A newtype costs nothing at runtime and makes an argument-order mistake a compile error.
- No stringly-typed fields where a closed set exists. `MailboxRole`, not `&str`.
- No `bool` in state (see §7). No `bool` parameters either: a call reading `send(true)` tells
  the reader nothing, while `send(Confirm::Yes)` tells them everything.
- Prefer an empty collection to `Option<Vec<T>>`. There is one way to say "none" and it needs
  no unwrapping.
- Model absence precisely. `Attachments::Present { count: u32 }` cannot represent zero
  attachments; `Option<u32>` can, and then every caller must decide what `Some(0)` meant.

### Parse, don't validate

Convert untrusted input into a type that cannot be wrong, once, at the boundary — then the rest
of the program needs no defensive checks. `mail-mime::parse` turns hostile bytes into `Parsed`;
`Credential` cannot be logged because its `Debug` is written by hand. A validation function
that returns `bool` and leaves the caller holding the same loose type has moved the problem,
not solved it.

A fallible constructor returns `Result`. A constructor that cannot fail returns `Self`.

### Traits are seams, not decoration

A trait earns its place when two implementations are genuinely swapped at that boundary —
`Store` (SQLite and in-memory), `Backend` (IMAP, POP3 and a fake), `Secrets` (keyring and a
map). A trait with one implementation is indirection with no payoff, and a trait invented to
group functions that merely feel related makes the call site harder to follow.

Prefer an enum to a trait for closed vocabulary. Mail is a closed protocol: `Op`, `Filter` and
`RemoteRef` should be exhaustive in one place, so that adding a variant produces a list of
compile errors naming every site that must be updated. A `dyn Trait` hides that list.

Implement the standard traits when the semantics genuinely hold, and derive rather than
hand-write unless a behaviour must differ. `Display` is for humans, `Debug` is for developers,
and neither is a serialization format — that is `serde`'s job.

### Errors are values, and they carry a decision

See §5 for the mechanics. The principle: an error type exists so a caller can *act*, not so a
string can be logged. Every error in this workspace answers `Retryable::retry`, because the
outbox has to choose between backing off, prompting for reauthentication and giving up, and
nothing else can make that choice for it.

Reserve panics for broken invariants inside our own code, and document them as caller
contracts. Malformed input is never a panic: the bytes came from a stranger.

### Borrowing from other projects

This workspace is `MIT OR Apache-2.0`. Most mature mail software is not, and the licence
determines what you may do with what you read. `docs/licensing-references.md` has the verified
table; three buckets, not six:

- **permissive** (MIT, Apache-2.0, BSD, 0BSD) — code may be copied, with attribution.
- **weak copyleft** (MPL, LGPL) — link freely, but MPL's unit of copyleft is the *file*: paste one
  function and that file becomes MPL, and our crate-level licence claim becomes false.
- **strong copyleft** (GPL, AGPL, EUPL) — read for facts, never paste.

**The rule for reading is note-and-close.** Read the source, reduce what you learned to a sentence
about observable behaviour, close the file, implement from the sentence. The test: if it fits in a
sentence about what a *server* does, it is a fact and it is yours; if you would need the source
open to reproduce it, it is expression and it is theirs. Signs you actually copied: matching
identifiers, surviving comments, matching branch order, matching magic constants, matching error
strings, the same bug.

**A delegated worker in an isolated worktree cannot see gitignored files.** A git worktree
carries tracked content only, so recorded captures under `spike/out/` — the most valuable input
a protocol brief has — are simply absent there, and the worker writes synthetic traces instead
without knowing better. Copy any ignored input into the worktree before starting, or the brief
is quietly asking for guesswork. This cost the IMAP brief its real Gmail capture until the
worker's progress log said `spike/out/imap.trace absent`, which is the second time that log has
paid for itself.

**Never put GPL, AGPL or MPL source into a model's context and ask for an equivalent.** That is
note-and-close inverted, at volume, with nobody able to testify to independent derivation. Put the
RFC section and our own trace fixture in the prompt instead — both are things we own or that are
BSD-licensed. This applies to every delegated brief in `ORCHESTRATION.md`.

**Cite the behaviour, not the client.** `// taken from mbsync driver.c:412` in an MIT crate is a
pointer to GPLv2 code that reads as an admission, and it does not tell the next reader the thing
they need. Write what the server does and point at the trace that proves it. If you genuinely
copied permissively-licensed code, attribute it properly instead — that is a different situation
with different obligations.

**A quirk without a trace is a rumour.** That is the review comment.

### Composition over accumulation

Prefer iterator chains and small combinators where they read more clearly than a loop, and a
plain loop where they do not — clarity wins over point-free style, every time. Prefer
returning a new value to mutating an argument. Where a mutable accumulator really is the
clearest expression, keep it local to the function so the mutation cannot be observed from
outside.

Keep functions short enough to hold in your head. A long function is usually several functions
that have not been named yet, and naming them is most of the work of understanding them.

## 1. The interface freeze

`crates/mail-domain/src/**` and `crates/mail-proto/src/machine.rs` are the **frozen
interface**. Every other crate codes against them.

- **You may** fill in a `todo!()` body, add a private helper, add a test.
- **You may not** change a public signature, add or remove a public field or enum variant,
  or change a `#[serde]` attribute — not even one that "obviously should" change.

If you believe a frozen signature is wrong, **stop and report it**. Do not work around it.
A signature change is a coordinated edit across every agent currently running; it is cheap
to make deliberately and expensive to discover.

Empty crates carry `//! Filled in a later phase.` Leave them alone unless your brief names them.

## 2. Derives

Every public type derives, in this order:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
```

Then, where they apply:

- `Copy` — only for types that are one word or smaller (IDs, fieldless enums).
- `Hash` — every ID newtype, and anything used as a map key.
- `PartialOrd, Ord` — only where a total order genuinely means something. Not on enums
  whose variant order is arbitrary.
- `Default` — only where a default is genuinely meaningful. `ReadState::Unread` is;
  `MailboxRole::Inbox` is not.

Floats are not allowed in domain types, which is why `Eq` is always available.

**Exception — secrets.** `Credential` implements `Debug` **by hand**, redacting its
contents. Never `#[derive(Debug)]` on a type that holds a password or a token; derived
`Debug` leaks into logs, panics, and error chains.

## 3. Serde

The serde form of a domain type is a **persisted schema** — `AccountPlan`, `RemoteRef`,
`ProtoOp`, `SyncCursor`, `Patch` and `View` all round-trip through SQLite. Changing their
representation is a migration, not a refactor.

- **Enums with data: adjacently tagged.**
  ```rust
  #[serde(tag = "kind", content = "v", rename_all = "snake_case")]
  ```
  Adjacent tagging, not internal tagging: internal tagging cannot represent newtype variants
  over non-map types (`RemoteRef::Pop(String)` would fail at runtime, not compile time).
- **Fieldless enums:** `#[serde(rename_all = "snake_case")]`.
- **Structs:** field names as written, `snake_case`.
- **Never `deny_unknown_fields`** on a persisted type. It turns every forward-compatible
  field addition into a hard startup failure on downgrade.
- **Every field added after the first release** carries `#[serde(default)]`, *unless no safe
  default exists*. A field without it is a breaking schema change, and sometimes that is the
  correct one.

  The exception is narrow and has to be argued in writing. `ProtoOp::Submit` gained `mail_from`
  and `rcpt_to` (FINDINGS F37). `#[serde(default)]` would give an old row an **empty recipient
  list**, and a submission with no recipients is a message that goes nowhere while the outbox
  reports success — a silent loss, which is worse than the loud decode failure it replaces.

  So the rule is really: default when the default is *right*. Where it is not, the field is
  mandatory and the migration story is stated explicitly — for that change, an argument recorded
  next to the fixture that no row of the old shape can exist, because the only writer of a stored
  `ProtoOp` had never produced one.

  If you find yourself reaching for `#[serde(default)]` to silence a decode error, check which
  value it is about to invent. `Vec::new()` for a recipient list and `String::new()` for an
  address are not neutral.
- Every persisted type has a round-trip test in `tests/serde.rs`, and a **frozen fixture**
  in `crates/mail-domain/tests/fixtures/` that must continue to deserialize. Add to the
  fixtures; never edit one.

## 4. `#[non_exhaustive]`

On nothing, for now. This is a single-binary workspace with no external consumers, so
`non_exhaustive` buys no compatibility and costs exhaustive matching — which is the whole
reason the vocabulary is enums. Revisit only if a crate is ever published.

## 5. Errors

One error enum per crate, in `error.rs`, built with `thiserror`.

```rust
#[derive(Debug, thiserror::Error)]
pub enum StoreError { /* ... */ }
```

Every error type implements `mail_domain::Retryable`:

```rust
pub trait Retryable {
    fn retry(&self) -> Retry;
}
```

This is not decoration. The outbox backoff loop, the reauth prompt, and the undo-on-fatal
path in `mail-store` all branch on it, and there is no other source for that decision.

- No `anyhow` below `mail-runtime`. Libraries return typed errors.
- No `unwrap()` or `expect()` in non-test code, except where a comment on the same line
  proves the invariant.
- `panic!` is for programmer error only. Malformed mail is not programmer error: a message
  with a broken `Date` header parses to an error value, never a panic. Assume every byte
  from the network is hostile.

## 6. Time

`now: DateTime<Utc>` is an argument. There is no `Clock` trait and no call to
`Utc::now()` anywhere below `mail-runtime` — including in tests, which should use fixed
instants so failures are reproducible.

`std::time::Duration` for configured intervals. `chrono::TimeDelta` only for arithmetic on
`DateTime`.

## 7. Naming

- Enums read as values at the use site: `ReadState::Unread`, not `ReadState::IsUnread`.
- No `bool` fields in domain state (see `plan.md` principle 6). Predicate **returns** are
  `bool` — `Filter::fit` returns `bool`, not an enum.
- No `get_` prefix. `fn address(&self)`, not `fn get_address(&self)`.
- Wire vocabulary stays out of domain names. `ArchiveMeans::DropInbox`, not
  `ArchiveMeans::GmailStyle`.

## 8. Modules and files

One concept per file; a file over ~400 lines wants splitting. `lib.rs` declares modules and
re-exports the flat public surface — it holds no definitions.

Public items carry a doc comment saying what they mean, not what they are. `/// The messages
a thread spans, unioned over its messages.` is useful; `/// The mailboxes.` is not.

## 9. Tests

- **Unit tests** in `#[cfg(test)] mod tests` beside the code, for private behaviour.
- **Integration tests** in `crates/<crate>/tests/<topic>.rs`, for the public surface.
- Table-driven wherever there is more than one case: a `const CASES: &[(Input, Expected)]`
  and one loop. A failure must name the case.
- Property tests use `proptest`. Required in two places, both non-negotiable:
  `Op::apply` → `inverse` round-trips to the original state, and `Filter::fit` agrees with
  the SQL compiler. If either is skipped, the corresponding bug ships.
- Protocol tests are byte transcripts. See `crates/mail-proto/tests/traces/FORMAT.md`.
- No test touches the network. Live tests are `#[ignore]`, and are run by
  `./scripts/live-tests.sh`, which starts the servers they need from committed scripts rather
  than trusting whatever happens to be on the port (FINDINGS F88, F90).

## 10. Verification

Before reporting work complete, all four must pass:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
./scripts/check-boundary.sh
```

The last one is the sans-I/O boundary, mechanically enforced: `mail-domain`, `mail-mime` and
`mail-proto` must not reach `tokio`, `rusqlite`, `dioxus`, `reqwest` or `keyring`.

Note that `cargo tree -p <crate> -i <dep>` **exits 101 when the dependency is absent**, which
is the condition we want. A CI step that checks the exit status alone therefore fails exactly
when it should pass; `scripts/check-boundary.sh` checks for output instead.

**While a wave is in flight, run `cargo fmt -p <your-crate>`, not `cargo fmt --all`.** The
`--all` form rewrites every file in the workspace, including ones another agent has open.
`--all --check` is read-only and is fine at any time.

Report honestly. A failing test reported as passing costs more than the bug did.

## Substrings are not tokens

`contains`, `ends_with` and `starts_with` compare *characters*. A domain, a capability atom, a
status code and a word are **tokens**, and matching one with a substring is a bug that passes its
own test. This has happened four times here, in four disguises:

| intended | written | matches wrongly |
|---|---|---|
| the domain `office365.com` | `ends_with("office365.com")` | `evil-office365.com` |
| the word `io` in an error | `contains("io")` | every `connection` |
| the code `5.7.139` | `contains("5.7.139")` | `5.7.1399` |
| the capability `MOVE` | `contains("MOVE")` | `REMOVE` |

Three were caught by a test, one by a test written for something else. None was caught by review.

**So: when the thing being matched has a grammar, match it at its boundaries.** A domain compares
whole labels (`host == d || host.ends_with(&format!(".{d}"))`); a status code is bounded by
non-digits; a capability atom is compared whole (`imap::has_capability`); a word is split on
whitespace. The helper exists in each case — use it rather than the substring that looks the
same in the one example in front of you.

The cost is asymmetric, which is why this is a rule rather than a preference. A too-narrow match
fails visibly on input you have. A too-wide one silently does the wrong thing on input you have
not thought of, which is the input an attacker chooses.

**But a narrow match only fails visibly when you have the input.** An IMAP `NO` to a sign-in was
classified by searching its text for `[AUTHENTICATIONFAILED]` and `invalid credentials` — the
two wordings to hand, Dovecot's and Gmail's. Exchange, Courier, UW-imapd and Zimbra all answer a
wrong password with a bare `NO LOGIN failed.`, so on those servers the refusal came out
`Refusal::Permanent`, `retry()` said `Fatal` instead of `NeedsReauth`, and the poll loop treated
the pass as a success and came back in five minutes — 288 failed sign-ins a day against the
user's own mail server. Every test passed, because every test used a wording that was in the
table.

So before reaching for the text at all, ask **what the protocol already told you.** Here the
answer was sitting one field away: a `NO` is a reply to a *command*, and a `NO` to `LOGIN` or
`AUTHENTICATE` is a rejected credential whatever words follow it. POP3 in this repo had always
done it that way and SMTP classifies on the reply code; only IMAP read the prose. Prose is the
part of a protocol the RFC does not pin down, so it is the last thing to branch on and never the
first — and if you must, enumerate real servers' wordings in a test rather than the one in front
of you.

## Run it the way its user would, before concluding anything about it

A failure that reproduces only through your own harness is a fact about the harness.

The harness here was one environment variable. `GDK_BACKEND=x11`, set so a screenshot tool could
find the window, kept for three rounds, and never once removed — and under it the shell's WebView
receives no updates at all and shows an empty window. That produced a finding (F106) which said
the interface had never rendered, a commit message that said so at length, a section in
`ORCHESTRATION.md` telling the next person to install a toolchain they do not need, and a
correction (F107) that had to undo all three.

**So: before reporting that something does not work, run it with nothing of yours around it.**
No wrapper, no forced backend, no injected variable, no redirected output. If it works there, the
finding is about the wrapper. If it fails there too, you have something.

The tell is a conclusion drawn entirely from runs that share a flag you added. Ask what the
command would look like if a user typed it, and type that.

**And "I could not look at it" is usually a claim about your eyes, not about the program.** The
window was launched once with the screen locked, and the round ended saying the shell had not
been seen. Seeing it was never what verification needed: WebKit runs and executes script whether
or not a compositor is showing the pixels to anybody. `scripts/live-window.sh` seeds a real
store, starts the real binary, and has the page type into its own search box and post what the
list then contained to a loopback listener — no input tooling, no forced backend, no screen. It
is one command, and it exists because the technique had been rebuilt from scratch three times.

Reach for it whenever a conclusion is about what the user sees. A component-tree test proves the
components agree with each other; it does not prove WebKit renders them or that a keystroke
arrives.

## An assertion that was already true proves nothing

The same failure as above wearing different clothes. A test asserted `drafts > 0` after a
keystroke that was supposed to create a draft — and the fixture it ran against seeds a draft of
its own, so the assertion held whatever the keystroke did. It passed for a week and would have
gone on passing with the feature deleted.

**So: assert the difference the action makes, not a state the action happens to be compatible
with.** Read the quantity before, act, and compare — `drafts_before + 1`, not `> 0`; `before - 1`
conversations in the inbox, not `is_empty()`. Where the "before" is awkward to capture, name the
exact value the action must produce.

The tell is an assertion that could be moved *above* the line that does the work and still pass.

A fixture can be vacuous the same way. A test that spells out the value under test — the filter,
the query, the format string — and then asserts against its own copy agrees with itself whatever
the program does. Five tests of snoozing passed while `mailo list` ignored the feature entirely,
because each built the inbox filter itself instead of asking for it. **Ask the program for the
thing you are testing.** If the test has to restate it, the restatement is what is being tested.
When one can, it is not testing that line. Ask it of every new assertion that uses `>`, `<`,
`is_empty`, `is_some`, `contains` or `!` — those are where a condition wide enough to be
accidentally satisfied hides.
