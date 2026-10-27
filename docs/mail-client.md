# mail-client: the window's one door to mail

The window draws; it never waits. Everything it shows of the mail comes from a `Client` as a
value that is already there, and everything it changes goes to the `Client` as a command. The
window never holds the store, so no frame can be stuck behind a sync, and no part of the window
has to remember to read again after a write.

## Where it stands

- The UI reads the store while it renders, through `Client::stored()`: 84 calls in 46 files, among
  them the sidebar's memos `counted` (the Favourites' counts) and `scheduled`.
- One hand-bumped counter, `revision`, says something changed in what the window still reads
  itself. A write that forgets to bump it leaves that stale; the counter moves with every commit
  the client hears, so a commit no one bumps for, the background watch's or another process's,
  no longer does.
- About 15 `edge::block_on` calls in `ui` waited on the network on the UI thread (gone with
  stage 4).

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
    pub fn act(&self, command: Command) -> Ticket;               // never blocks
    pub fn fetch(&self, seams: Seams) -> Option<FetchRun>;        // once; the scheduler
    pub fn fetched(&self) -> Option<watch::Receiver<Arc<Fetched>>>;
    pub fn tell(&self, account: AccountId, event: Event);
    pub fn prime<Q: Query>(&self, queries: impl IntoIterator<Item = Q>) -> Vec<Live<Q::Output>>;
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
  - `mailo-clock`: marks stale what reads the clock when a snooze ends.
  - `mailo-fetch`: the scheduler (`mail_core::schedule`), once a caller asks for mail to be
    fetched, on a current-thread runtime of its own. Passes, watches and folder fetches run on
    its blocking threads.
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
  invalidated at the next due instant by `mailo-clock`, which finds that instant again whenever
  a commit touches the conversations. A caller whose clock the client cannot sleep on (a test's
  window, on quire's virtual time, or one whose client reads on its thread) waits for
  `Client::next_clock` itself and says when it has passed (`use_snooze_wake`). A caller whose
  time is not the system's (a test's window, on quire's virtual clock) gives the client its own
  with `Client::set_clock`, which reads again what reads the clock.

## Invalidation

Every write goes through the store, so the store says what changed.

- **Topics** are bitflags in `mail-store`: `ACCOUNTS, LABELS, FOLDERS, SUMMARIES, DRAFTS,
  OUTBOX, VIEWS, VIPS, CONTACTS, RULES, KEYS, TEMPLATES, SIGNATURES, IDENTITIES, PREFS,
  FOLLOW_UPS, CLOCK`. Each query declares the topics it reads.
- **In this process.** `connection()` returns a `Writer` guard that counts how deep the thread
  holds the writer. A `commit_hook` marks the store dirty (a rolled-back transaction never
  reaches it, so there is no `rollback_hook`); `update_hook` cannot say which tables, because
  SQLite does not call it for WITHOUT ROWID tables (`folders`, `vips`, `message_labels`,
  `sync_state`, `pending_changes`). Write methods say what they touch
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

- `act` returns a `Ticket` that resolves to `Acted { done, generation }`: `done` is
  `Done::Performed`, `TookBack` or `Destroyed`, saying what the command did, and `generation` is
  the commit a list read at or after it has. Commands are `Perform`, `TakeBack` and `Destroy`;
  the act thread calls `mail_core::act::perform`, `take_back` and `destroy`, the same rules the
  CLI uses. An op the store refuses is missing from `Performed`, and a ticket the client stopped
  before answering resolves to `None`.
- Undo and toasts stay in the window: it sends the command, awaits the ticket in a task of its
  own (`motion::when_done`), then records the undo and says what happened.
- Optimism is explicit: the list hides on the press what an op takes out of it
  (`list_query::hide`), so rows leave at once. It lets go once the list is read at the ticket's
  generation, or when the command is refused. It cannot stick, and a list cannot show the world
  before the write once it has shown it after.

## In Dioxus

```rust
pub fn use_live<Q: Query>(key: impl FnMut() -> Q + 'static) -> Memo<Snapshot<Q::Output>>
```

- The key is a memo, so it follows the signals it reads, and an equal key does not resubscribe.
- Mounting asks the client for the value (a lookup, never SQL) and starts from `snapshot()`:
  `Ready` on the first frame when it was primed or someone else already holds it.
- A new key swaps the handle and shows `Loading { last }`, so a Space switch does not flash
  empty; a task follows `changed()` into the signal. Unmounting drops both.
- `use_client()` reads the `Client` root context `launch/native.rs` provides.

## Testing

- `Client::inline(store, clock)` starts no thread and computes on subscribe, synchronously, so
  SSR tests see data on the first render; what the store has committed since is acted on at
  `client.settle()`. `client.idle()` says nothing is queued or running. `ClientConfig::clock`
  takes the harness clock.
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
2. List and reader, in three steps.
   **a.** The list: `Conversations` (a place's query, the returned and waiting lists, history, a
   search) answers `Found`, the rows with what a search found on top; the window keeps the
   question it drew and says whether a new one is another list or a deeper page.
   **b.** The reader: `Conversation(ThreadId)` answers the conversation with its messages, or
   `None` once it is gone. The list's top and the open conversation's neighbours are kept read
   (`use_warming`), so opening one is a lookup; a value for another conversation is never drawn
   under this one. Marking read on open stays a write in the window until c.
   **c.** Writes through `Client::act`: `Command::{Perform, TakeBack, Destroy}` on the
   `mailo-act` thread, in order, each answering a `Ticket` with what was done and the generation
   it committed at. The window hides on the press what an op takes out of the place's list
   (`act::leaves`, from the row's summary; never for a search, whose rows only a reading can
   tell), and shows it again when the list is read at that generation or the op is refused. The
   undo stack and the toast stay the window's, filled in when the ticket is answered. Settings,
   labels, views, VIPs and sends become commands with stage 4.
3. Fetching moves into the client. `Client::fetch(Seams)` starts the one scheduler, on
   `mailo-fetch` (an inline client hands back the future for its caller to run). What reaches a
   server is lent as `Seams` (pass, watch, pushers, daemon, who schedules, folder fetch, timing):
   the window builds them from its contexts, so a test lends its own. Every account's link, the
   offer beside a failed pass and each folder fetched on opening are one `Fetched` value behind a
   watch channel; the window follows it into the signals its status line, sidebar and reauth
   read, and keeps the spinner's token. The accounts are followed from `ACCOUNTS` commits, not a
   memo. A send that comes due (queued for later, or held for Undo) asks its account for a pass
   (`fetch::outbox`), whether or not the compose pill is on screen. Folder refusals
   carry the server's words only. The clock's timer is the client's (`mailo-clock`). `revision`
   stays until stage 4 converts the memos that read it.
4. Everything else reaches the store through the client, on the caller's thread.
   **a.** What the window called in `mail-core` with the store, it calls on a handle named for
   the module, taking what the function takes but the store (`calls`): `client.compose()`,
   `template()`, `book()`, `keys()`, `certs()`, `rules_manage()`, `transfer()`, `print()`, ….
   The store's own reads, and the few writes made by hand, are `stored()`; bodies and attachments
   are `blobs()`; the reader's caches are filled through `looking(&looks)` and
   `framing(&frames)`; a query read once, now, is `read(query)`. What reaches a server is
   `net()`, waiting on the runtime the program says to reach servers with (`Reach`,
   `Client::reach`; a client never told reaches them with no keyring): a folder fetched now, a
   body, a part, a drain, rules pushed, a key looked up, a server search, an unsubscribe,
   discovery, adding, signing in and removing an account, and the passes and watches the fetch
   seams lend (`net().pass()`, `net().listen()`). No `edge::block_on` or `edge::mail` is left in
   `ui`. The root still hands the store to `launch` (the program's) and to a test's window, whose
   `use_client()` makes an inline client over it; stage 5 takes it away.
   **b.** `revision` goes: what moved is the client's to say (`Client::moved`).
5. The fence: `ui`, `settings` and the platform seams move to `mail-ui`, whose `Cargo.toml` has
   no store.
