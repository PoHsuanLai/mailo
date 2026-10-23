# Contributing

How mailo's code is written, and the checks that hold it there. If a rule here blocks you, say
so in the pull request rather than working around it.

## Crate layering

Effects live at the edges, and the layering encodes that:

- `mail-domain`, `mail-mime`, `mail-proto` and `mail-pim` are pure. They reach no socket, clock,
  disk, runtime or window. When one seems to need an effect, the effect belongs to the caller:
  `Op::apply` returns a `RemoteIntent` rather than performing one, and `Machine::feed` returns an
  `IoNeed` rather than satisfying it.
- `mail-store` may touch the disk, and is the only crate that speaks SQL.
- `mail-runtime` is the only crate that opens a socket, spawns a task or reads the clock.
- `mail-core` drives them and may use tokio, but has no window and no terminal in it.
- `mail-app` draws the window (`ui`) and the terminal (`cli`) over `mail-core`. The two never
  name each other: a helper both want belongs in `mail-core`.

The libraries start no runtime, read no environment variable and keep no process-wide state.
Their I/O is `async`, and `mail-app`'s `edge` owns the one runtime and the environment, handing
them in through `mail_core::Mail`.

## The frozen interface

`crates/mail-domain/src/**` and `crates/mail-proto/src/machine.rs` are the interface every other
crate codes against. A change to a public signature, a public field or variant, or a `#[serde]`
attribute there is a deliberate, reviewed change across the workspace, never a side effect of
other work.

## Design rules

- **Make illegal states unrepresentable.** Newtype every identifier and unit (`AccountId`, not
  `Uuid`). A closed set is an enum, not a string. No `bool` in domain state and no `bool`
  parameters: `send(Confirm::Yes)`, not `send(true)`. Prefer an empty collection to
  `Option<Vec<T>>`.
- **Parse, don't validate.** Untrusted input becomes a type that cannot be wrong, once, at the
  boundary. A fallible constructor returns `Result`; one that cannot fail returns `Self`.
- **Traits are seams.** A trait earns its place when two implementations are swapped at that
  boundary (`Store`, `Backend`, `Secrets`). Closed vocabulary (`Op`, `Filter`, `RemoteRef`) is an
  exhaustive enum, so a new variant lists every site that must change.
- **Data apart from logic.** A type's own `impl` holds construction, derivation and accessors;
  a decision about several values is a free function or lives with the type that owns it.
- Keep functions short enough to hold in your head. Prefer returning a new value to mutating an
  argument.

## Derives

Public types derive `Debug, Clone, PartialEq, Eq, Serialize, Deserialize`, in that order, then
`Copy` (one word or smaller), `Hash` (IDs and map keys), `PartialOrd, Ord` (only where the order
means something) and `Default` (only where a default is meaningful). No floats in domain types.
A type that holds a password or token implements `Debug` by hand, redacted.

## Serde and persisted types

The serde form of a persisted type (`AccountPlan`, `RemoteRef`, `ProtoOp`, `SyncCursor`, `Patch`,
`View`, …) is a schema: changing it is a migration, not a refactor.

- Enums with data are adjacently tagged: `#[serde(tag = "kind", content = "v", rename_all =
  "snake_case")]`. Fieldless enums are `rename_all = "snake_case"`.
- Never `deny_unknown_fields` on a persisted type.
- A field added later carries `#[serde(default)]` when the default is right. Where it is not (an
  empty recipient list would be a message that silently goes nowhere), the field is mandatory and
  the reason is written beside the fixture.
- Every persisted type has a round-trip test in `crates/mail-domain/tests/domain/serde.rs` and a
  frozen fixture in `crates/mail-domain/tests/fixtures/`. Add fixtures; never edit one.
- `#[non_exhaustive]` goes on growing public enums and structs in the library crates (error
  enums first), never on a persisted type and never on a closed vocabulary.

## Errors

One `error.rs` per crate, built with `thiserror`, carrying the cause a caller can act on as a
field. Every error type implements `mail_domain::Retryable`, because the outbox must choose
between backing off, asking for reauthentication and giving up.

- No `anyhow` below `mail-runtime`.
- No `unwrap()` or `expect()` outside tests unless a comment on the line proves the invariant.
- `panic!` is for programmer error only. Malformed mail is never a panic: every byte from the
  network is hostile.

## Time

`now: DateTime<Utc>` is an argument. There is no `Clock` trait and no `Utc::now()` below
`mail-runtime`, tests included: tests use fixed instants. `Duration` for configured intervals,
`chrono::TimeDelta` only for arithmetic on `DateTime`.

## Naming

Enums read as values (`ReadState::Unread`). No `get_` prefix. Wire vocabulary stays out of domain
names (`ArchiveMeans::DropInbox`, not `GmailStyle`).

## Modules and files

One concept per file; a file over ~400 lines wants splitting. `lib.rs` declares modules and
re-exports the public surface, and holds no definitions. Public items carry a doc comment saying
what they mean, not what they are.

## Tests

Run them with [cargo-nextest](https://nexte.st), and the doctests separately, since nextest does
not run them:

```sh
cargo nextest run --workspace
cargo test --doc --workspace
```

- Unit tests sit beside the code in `#[cfg(test)] mod tests`; integration tests are under
  `crates/<crate>/tests/`.
- Table-driven wherever there is more than one case, and a failure names its case.
- `proptest` is required for two properties: `Op::apply` then its inverse restores the state, and
  `Filter::fit` agrees with the SQL the store compiles it to.
- Protocol tests are byte transcripts (`crates/mail-proto/tests/traces/FORMAT.md`).
- No test touches the network. Live tests are `#[ignore]`d and run by `./scripts/live-tests.sh`,
  which starts the servers they need from committed scripts.
- A conclusion about what the user sees is checked in the real window: `ds_harness::Harness`
  drives Blitz headlessly (`crates/mail-app/tests/app/native_harness.rs`).

### An assertion that was already true proves nothing

Assert the difference an action makes, not a state it happens to be compatible with: read the
quantity before, act, compare (`drafts_before + 1`, not `> 0`). If an assertion could move above
the line that does the work and still pass, it tests nothing. Likewise, ask the program for the
thing under test instead of restating it in the test. Be wary of every `>`, `<`, `is_empty`,
`is_some`, `contains` and `!`.

## Substrings are not tokens

A domain, a capability atom, a status code and a word are tokens. Match them at their boundaries,
never with `contains`, `starts_with` or `ends_with`: `ends_with("office365.com")` matches
`evil-office365.com`, and `contains("MOVE")` matches `REMOVE`. Use the helper that exists
(`imap::has_capability`, whole-label domain comparison). Before reading a server's prose at all,
ask what the protocol already said: a `NO` to `LOGIN` is a rejected credential whatever its text.

## The checks

All of these must pass before a change is merged:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo nextest run --workspace
cargo test --doc --workspace
for s in scripts/check-*.sh; do bash "$s"; done
```

- `check-boundary.sh`: the pure crates reach none of tokio, rusqlite, dioxus, reqwest, the
  keyring or the bus; `mail-core` reaches nothing that draws; `mail-store` and `mail-runtime`
  reach no anyview; `ui` and `cli` do not name each other. It also ratchets the lines in
  `mail-app` that name a lower crate instead of `mail_core` (`scripts/app-lower-crate-uses.txt`)
  and the `mail-core` code listed in `scripts/core-prose-allowlist.txt` and
  `scripts/core-result-string-allowlist.txt`.
- `check-discards.sh`: in the library crates, a `let _ =`, `.ok()` or `unwrap_or_default()` with
  no comment on or above its line is counted against `scripts/discards.txt`. A fallible result
  is propagated, shown to the person, logged with `Logged::or_log`, or ignored with a reason.
- `check-copy.sh`: visible string literals under `crates/mail-app/src/ui` outside `ui/copy/` are
  counted against `scripts/copy-literals.txt`.
- `check-fork-patches.sh`: the `[patch.crates-io]` block matches quire's at the pinned rev.
- `check-nodef.sh`: every library crate builds with no default features.

A ratchet fails when a count rises, and also when it falls until the list is updated to match:
the lists only shrink.

## Comments and copy

Comments are plain declarative English: what a thing is, or why it is that way. State the
invariant or the reason, not the history of how the code got here, and no ticket numbers. Cite
behaviour, not another client's source: "Gmail answers `X`", pointing at a trace, not
"taken from mbsync". A quirk without a trace is a rumour.

Every word the window shows lives in `crates/mail-app/src/ui/copy/`, one module per kind. Title
Case for buttons, menu items, tooltips, titles and accessible names; sentence case for sentences.
Ellipsis is `…` and the apostrophe `’`. `docs/design-language.md` has the rest.

## CSS

mailo's stylesheets (`crates/mail-app/src/ui/style/*.css`) say where things go, never how they
look: every colour, size, radius, weight, shadow and duration is a quire token, every control a
quire component. The style tests hold four coherence rules:

1. mailo's stylesheet lints clean at quire's `Profile::Strict`, spacing included.
2. The rendered window has no class nothing styles, no raw control, no raw vector and no literal
   colour in a `style`.
3. A value or component quire lacks is requested from quire, never written as a literal or
   patched here. An exception in `ui/style/exceptions.rs` names its selector and its reason.
4. Motion is quire's, timed by quire's motion clock, never by an `animationend` event.

## Borrowing from other projects

mailo is `MIT OR Apache-2.0`. Permissive code (MIT, Apache-2.0, BSD) may be copied with
attribution. Weak copyleft (MPL, LGPL) may be depended on, never pasted: MPL's unit is the file.
Strong copyleft (GPL, AGPL, EUPL) may be read for facts and never copied. Reduce what you read to
a sentence about what a server does, close the file, and implement from the sentence.
