# mail-client: the window's one door to mail

The window draws; it never waits. Everything it shows of the mail comes from a `Client` as a
value that is already there, and everything it changes goes to the `Client` as a command. The
window never holds the store, so no frame can be stuck behind a sync, and no part of the window
has to remember to read again after a write.

## Where it stands

- The UI reads the store while it renders: 196 `consume_context::<Arc<SqliteStore>>` in 92 files,
  the sidebar's memos (`counted`, `tiled`, `scheduled`, `folders`), `PlaceRow`'s `label_of`, and
  `App`'s seeding hook, effect and first-frame badge fallback.
- Badges are joined to places by position (`badges().get(index)`), from two signals that move at
  different times.
- One hand-bumped counter, `revision`, says something changed. A write that forgets to bump it,
  or a commit no one bumps for, leaves the window stale.
- `SqliteStore::data_version` does not count this store's own commits. On macOS the in-process
  background watch (`background::start`) writes through the window's own store, so mail it
  fetches does not move `revision`.
- About 15 `edge::block_on` calls in `ui` wait on the network on the UI thread.

## Crates

```
mail-domain  mime  proto  pim            (pure)
mail-store → mail-runtime → mail-core    (I/O)
mail-client  → mail-core, mail-store, mail-domain, tokio, notify   (no dioxus, no ds)
mail-ui      → mail-client, the pure crates, dioxus, ds            (the last stage)
mail-app     → mail-ui, mail-client, mail-core                     (binary, CLI, background)
```

The fence is the compiler: `mail-ui`'s `Cargo.toml` names no `mail-core`, `mail-store` or
`mail-runtime`, so a store is not something the window can name. `mail-core` re-exports
`SqliteStore` at its root, so dropping a dependency inside `mail-app` would fence nothing, and
module privacy cannot hide another crate's paths. There is no lint and no allowlist: until the
split, the stages simply convert the window, and the split is what keeps it converted.

`mail-client` re-exports only what is pure (`mail_domain` types, `Scope`, `undo`, `when`,
`editor`, the pure parts of `view` and `place`). It never hands out `&SqliteStore` or `Mail`.

`mail-client` is logic only. Its queries are facts about the mail — the accounts with their
folders and labels, the saved views, how many conversations each filter matches — and it has no
word for a sidebar, a place, a badge or a Space. The window puts the facts together and gives them
their meaning: a list of places is the accounts' folders and labels and the saved views, and a
badge is a count of the filter its place lists.

The CLI keeps calling `mail-core`: it has no frame to protect, and the rules it shares with the
window already live in `mail-core`.

## Client

```rust
pub struct Client(Arc<Inner>);   // Clone, Send + Sync; one per process, shared by every window
impl Client {
    pub fn open(store: Arc<SqliteStore>, config: ClientConfig) -> Client;
    pub fn live<Q: Query>(&self, query: Q) -> Live<Q::Output>;   // a registry lookup, never SQL
    pub fn act(&self, command: Command) -> Ticket<Done>;         // never blocks
    pub fn prime(&self, queries: impl IntoIterator<Item = AnyQuery>);
    pub fn shutdown(self, within: Duration);
}
```

- `Inner` holds the store, a `mail_core::Mail`, the change hub, the registry and the queues.
- Threads, none of them the window's:
  - `mailo-read-N`: `READERS - 1` query workers, so one reader stays free for the background
    watch and the session. Each job pins one reader in one read transaction.
  - `mailo-act`: one command thread, applying commands in order; the only thing that waits on
    the writer.
  - `mailo-changes`: hears commits, marks entries dirty, queues recomputes.
  - Network commands (`server_search`, `fetch_body`, `discover`, `unsubscribe`) run as tasks on
    the runtime `edge::runtime()` is today, which moves into `Client`.
- Shutdown closes the queues, lets a write in progress finish, drops read jobs and joins the
  threads within a bound. On macOS the `Client` lives in `main`, not in a window.

## Live<T>

```rust
pub enum Snapshot<T> {
    Loading { last: Option<Arc<T>> },
    Ready { value: Arc<T>, as_of: Generation },
    Failed { error: ClientError, last: Option<Arc<T>> },
}
impl<T> Live<T> {
    pub fn snapshot(&self) -> Snapshot<T>;                       // an Arc clone under a lock
    pub async fn changed(&mut self) -> bool;
    #[cfg(feature = "testing")] pub fn wait(&self) -> Arc<T>;
}
```

- **Shared.** The registry keys entries by the query (`Hash + Eq`) and holds them weakly: two
  components asking the same thing share one value and one computation. A key holds parameters
  only (the Space's scope, the pressed account, `Show`, pins); anything a query depends on that
  is not in the database is part of its key.
- **Dropped.** When the last `Live` goes, the entry goes. A queued job finds it gone and does
  not run; a running one finishes and its answer is thrown away.
- **Coalesced.** Each entry is queued at most once, with at most one run in flight, so the queue
  is never longer than the live entries. Commits are gathered over about 30 ms, at most 150 ms,
  so an ingest committing every 200 entries does not recount badges per chunk. An answer equal
  to the last one wakes no one.
- **Time.** Snoozes, follow-ups and scheduled sends change as time passes: the `CLOCK` topic is
  invalidated (`Client::invalidate`) by a timer at the next due instant. The timer comes with the
  fetching, in stage 3; until then the window's own wake does it.

## Invalidation

Every write goes through the store, so the store says what changed.

- **Topics** are bitflags in `mail-store`: `ACCOUNTS, LABELS, FOLDERS, SUMMARIES, DRAFTS,
  OUTBOX, VIEWS, VIPS, CONTACTS, RULES, KEYS, TEMPLATES, SIGNATURES, IDENTITIES, PREFS,
  FOLLOW_UPS, CLOCK`. Each query declares the topics it reads.
- **In this process.** `connection()` returns a `Writer` guard that counts how deep the thread
  holds the writer. A `commit_hook` marks the store dirty and a `rollback_hook` clears it;
  `update_hook` cannot, because SQLite does not call it for WITHOUT ROWID tables (`folders`,
  `vips`, `message_labels`, `sync_state`, `pending_changes`). Write methods say what they touch
  (`self.touch(Topics::FOLDERS)`). When the outermost guard drops, after the commit is visible
  to readers, the store publishes `Committed { topics, generation }`. A commit that said nothing
  publishes `ALL`, so a missing declaration costs a recount, never a stale window.
  `remove_account` says `ALL`: its cascade reaches every table an account owns.
- **Other processes.** The watch in `ui/fetching/external.rs` (the db and `-wal` files, then
  `data_version`) moves into `mail-client` as it is, and publishes `ALL`.
- **One snapshot per answer.** `read_pinned` locks one reader for the thread and opens a read
  transaction on it; `reader()` hands that thread the same reader again. A sidebar's twenty
  counts are one moment of the database, and `as_of` is the generation read before it began.

Each value is read in one snapshot, and a count is answered beside the filter it counted
(`Counts`), never by position: so a number cannot land on another place's row while the list of
places is being read again, and an account's labels go with it, in the same value.

## Commands

- `act` returns a `Ticket` that resolves to `Done { said, undo: Option<Undo>, generation }`.
  Commands are `Ops`, `Folder`, `TakeBack`, `Restore`, label, view and VIP edits, and later sends.
  The act thread calls `mail_core::act::perform`, `folder::change` and `act::take_back`, the
  same rules the CLI uses.
- Undo and toasts stay in the window: `use_act()` sends the command, awaits the ticket in a task,
  then records the undo and says what happened.
- Errors are `ClientError { Refused, Store, Network }`, put into words by `ui/copy`.
- Optimism is explicit: the window keeps `Pending { hides, until: Generation }`, so rows leave on
  the press. It lets go once the list's `as_of` reaches the ticket's generation, or when the
  command fails (with a toast). It cannot stick, and a list cannot show the world before the
  write once it has shown it after.

## In Dioxus

```rust
pub fn use_live<Q: Query>(key: impl Fn() -> Q + 'static) -> ReadSignal<Snapshot<Q::Output>>
```

- The key is a memo, so it follows the signals it reads, and an equal key does not resubscribe.
- Mounting asks the client for the value (a lookup, never SQL) and starts from `snapshot()`:
  `Ready` on the first frame when it was primed or someone else already holds it.
- A new key swaps the handle and shows `Loading { last }`, so a Space switch does not flash
  empty; a task follows `changed()` into the signal. Unmounting drops both.
- `use_client()` reads the `Client` root context `launch/native.rs` provides.

## Testing

- `Client::builder(store).inline()` (testing feature) computes on subscribe and on invalidation,
  synchronously, so SSR tests see data on the first render. `client.idle()` says nothing is
  queued or running. `ClientConfig::clock` takes the harness clock.
- Lanes keep seeding through the store; publishing makes the window notice.
- `mail-client`'s own tests, on temp-dir stores: coalescing, sharing, a drop cancels a queued
  job, an undeclared commit publishes `ALL`, `remove_account` publishes `ACCOUNTS`, a pinned
  read is one snapshot across a concurrent commit, a commit through a second store on the same
  file is heard.

## Stages

Each leaves the app working.

1. **a.** The crate, store publishing, `Live`, the registry, workers, the external watch, tests.
   No window files.
   Queries: `Accounts`, `Views`, `Counts`.
   **b.** The window's sidebar built from them; `App`'s seeding hook, badge filters and fallback
   go. `external.rs` becomes a follower of the client's generation that still moves `revision`,
   so memos not yet converted hear in-process and background commits too.
2. List and reader: `Live<ListPage>`, `Live<ThreadView>`, writes through `act` with the overlay.
3. Fetching moves into the client: the scheduler, the background watch and the outbox. The
   shared `Revisions` retires for store data.
4. Everything else: compose, settings, contacts, rules, keys, add account, the palette,
   downloads. Every `edge::block_on` in `ui` becomes a command.
5. The fence: `ui`, `settings` and the platform seams move to `mail-ui`, whose `Cargo.toml` has
   no store.
