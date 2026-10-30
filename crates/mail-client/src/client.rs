//! The [`Client`]: who holds the store, reads what its callers ask for, and reads it again when
//! the store changes.
//!
//! # Threads
//!
//! None of them a caller's.
//!
//! - **Readers** (`mailo-read-N`), one fewer than the store has reader connections, so one is
//!   always free for the background watch and the session. Each takes a stale value from the
//!   queue and reads it, in one moment of the database.
//! - **Changes** (`mailo-changes`) hears every commit the store makes, gathers the ones that come
//!   together, and marks stale every value that reads what they touched.
//! - **Act** (`mailo-act`) applies [`Act`]s, one at a time, in the order they were given: the
//!   only one that waits on the store's writer.
//! - **Clock** (`mailo-clock`) marks stale what reads the clock when a snooze ends (`clock`).
//! - **External** (`mailo-external`) watches the store's file for another process's commits
//!   (`external`).
//! - **Fetch** (`mailo-fetch`), once a caller asks for mail to be fetched ([`Client::fetch`]),
//!   runs the scheduler that fetches it, on a runtime of its own (`fetch`).
//!
//! A value is in the queue at most once, and read by at most one thread at a time; one that goes
//! stale while it is read is read again when that reading ends. So the queue is never longer than
//! the values held, however many commits a sync makes.

use crate::act::{Act, Ticket};
use crate::fetch::{Fetched, Fetcher, Seams};
use crate::live::{Entry, Job, Live, Query};
use chrono::{DateTime, Utc};
use mail_core::fetch::Event;
use mail_core::mail::{Clock, SystemClock};
use mail_domain::MailboxRef;
use mail_runtime::Logged;
use mail_store::{Checkpoint, Committed, SqliteStore, Topics};
use parking_lot::{Condvar, Mutex};
use porter_core::AccountId;
use std::any::{Any, TypeId};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, OnceLock, Weak};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// How long the change thread waits for more commits after one before it marks anything stale:
/// a sync commits every few hundred messages, and one recount for a burst is enough.
const GATHER: Duration = Duration::from_millis(30);

/// The longest a commit waits to be acted on while more keep coming.
const AT_MOST: Duration = Duration::from_millis(150);

/// How a [`Client`] is made.
#[derive(Clone)]
pub struct ClientConfig {
    /// What time it is, for what reads the clock (a snooze ending, a scheduled send).
    pub clock: Arc<dyn Clock>,
    /// Whether to watch the store's file for other processes' commits. Off for a store no other
    /// process opens.
    pub external: bool,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            clock: Arc::new(SystemClock),
            external: true,
        }
    }
}

impl std::fmt::Debug for ClientConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientConfig")
            .field("external", &self.external)
            .finish_non_exhaustive()
    }
}

/// Mail as values, read and kept read. Clone it freely: every clone is the same client, and one
/// process needs only one.
#[derive(Clone)]
pub struct Client(Arc<Inner>);

/// A client that does not keep the client going: for what the client itself holds (the seams it
/// fetches through). A closure the client owns that held a [`Client`] would be an `Arc` cycle:
/// the client's threads would never be told to stop. Hold this and [`upgrade`](Self::upgrade) it
/// for the length of one call.
#[derive(Clone)]
pub struct WeakClient(Weak<Inner>);

impl std::fmt::Debug for WeakClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("WeakClient")
    }
}

impl WeakClient {
    /// The client, if some other holder still has it.
    pub fn upgrade(&self) -> Option<Client> {
        self.0.upgrade().map(Client)
    }
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Client")
    }
}

/// The client, with the threads it started: stopped when the last clone goes.
struct Inner {
    shared: Arc<Shared>,
    threads: Mutex<Vec<JoinHandle<()>>>,
}

/// What the client's threads share.
pub(crate) struct Shared {
    store: Arc<SqliteStore>,
    clock: parking_lot::RwLock<Arc<dyn Clock>>,
    /// Every value held, by its query.
    registry: Mutex<Registry>,
    /// The values waiting to be read.
    queue: Mutex<VecDeque<Weak<dyn Job>>>,
    queued: Condvar,
    /// Values in the queue or being read: none is a client at rest.
    busy: AtomicUsize,
    stopping: AtomicBool,
    /// Read on the thread that asks, never on threads of the client's own (`Client::inline`).
    inline: bool,
    /// For an inline client, the commits not acted on yet ([`Client::settle`]).
    heard: Mutex<Option<Receiver<Committed>>>,
    /// For an inline client, one more each time a commit is waiting to be acted on.
    committed: Arc<tokio::sync::watch::Sender<u64>>,
    /// One more each time values were marked stale, by a commit here or another process's.
    moved: tokio::sync::watch::Sender<u64>,
    /// The same, for each set of topics a caller asked to hear alone: one more only when what was
    /// marked stale touches them ([`Client::moved_for`]).
    moved_for: Mutex<Vec<(Topics, tokio::sync::watch::Sender<u64>)>>,
    /// How the external watch is woken: by a write to the store's file, or to stop.
    pub(crate) external: Mutex<Option<std::sync::mpsc::Sender<()>>>,
    /// Where commands go to be applied; `None` for an inline client, and once stopping.
    acts: Mutex<Option<std::sync::mpsc::Sender<Asked>>>,
    /// Fetching, once a caller asked for it.
    fetcher: OnceLock<Fetcher>,
    /// Stops the fetch thread.
    fetch_stop: Mutex<Option<tokio::sync::oneshot::Sender<Duration>>>,
    /// Wakes the clock thread to find its next moment again: set, then told.
    clock_poked: Mutex<bool>,
    clock_told: Condvar,
    /// How servers are reached, once the program says (`net`).
    pub(crate) reach: OnceLock<crate::net::Reach>,
}

/// Fetching run where the caller runs it: what [`Client::fetch`] hands a client that reads on
/// its caller's thread. Runs until it is dropped.
pub type FetchRun = std::pin::Pin<Box<dyn std::future::Future<Output = ()>>>;

/// A change to make in the store, and the telling of what it did, ready to run on the act thread.
type Asked = Box<dyn FnOnce(&SqliteStore) + Send>;

/// The values held of one type of query, found by [`Query::bucket`] and then by equality: a
/// query is asked with what it reads (filters, accounts), which need not hash.
type Held<Q> = HashMap<u64, Vec<(Q, Weak<Entry<Q>>)>>;

#[derive(Default)]
struct Registry {
    /// Each type of query's [`Held`].
    by_query: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
    /// Every value, whatever its query, for the change thread to go through.
    every: Vec<Weak<dyn Job>>,
}

/// How large the write-ahead log may be at start before it is cut down: the size the store
/// caps it to once a checkpoint has run.
const LOG_LARGE: u64 = 64 * 1024 * 1024;

impl Client {
    /// The client over `store`, its threads started.
    pub fn open(store: Arc<SqliteStore>, config: ClientConfig) -> Client {
        let heard = store.subscribe();
        let shared = Arc::new(Shared::new(store, config.clock, false));
        let mut threads = Vec::new();
        for n in 0..readers(&shared.store) {
            let shared = shared.clone();
            // These hold a reader for as long as a read takes, on the readers that are not kept
            // for a thread that must not wait (`SqliteStore::reserve_reader_for_this_thread`).
            threads.push(spawn(&format!("mailo-read-{n}"), move || shared.read()));
        }
        {
            let shared = shared.clone();
            threads.push(spawn("mailo-changes", move || shared.hear(&heard)));
        }
        {
            let (tx, asked) = std::sync::mpsc::channel::<Asked>();
            *shared.acts.lock() = Some(tx);
            let shared = shared.clone();
            threads.push(spawn("mailo-act", move || {
                // Every command given is applied, even once the client is stopping: a change the
                // person made is not dropped because the caller stopped the client after it.
                // A job that panics leaves its ticket unanswered (what it held to answer with is
                // dropped as it unwinds), which says it was not done, and the next job runs.
                for job in asked {
                    survive("a change", || job(&shared.store));
                }
            }));
        }
        // A log left large by an earlier run is cut down here, on a thread of the client's own
        // rather than the caller's: `open` is called while the window is coming up, and a
        // truncate waits for every reader to finish. The thread ends when it is done.
        if shared.store.log_size() > LOG_LARGE {
            let shared = shared.clone();
            threads.push(spawn("mailo-log", move || {
                shared
                    .store
                    .checkpoint(Checkpoint::Truncate)
                    .or_log("the log could not be cut down at start");
            }));
        }
        threads.push(crate::clock::start(shared.clone()));
        if config.external
            && let Some(watching) = crate::external::start(shared.clone())
        {
            threads.push(watching);
        }
        Client(Arc::new(Inner {
            shared,
            threads: Mutex::new(threads),
        }))
    }

    /// A client that reads on the thread that asks, at once, and starts no thread: for a test,
    /// whose first look at a value should find it read, and for a caller with no thread to spare.
    /// What the store commits is acted on at [`Client::settle`].
    pub fn inline(store: Arc<SqliteStore>, clock: Arc<dyn Clock>) -> Client {
        let shared = Shared::new(store.clone(), clock, true);
        let committed = shared.committed.clone();
        let heard = store.subscribe_waking(Arc::new(move || {
            committed.send_modify(|count| *count += 1);
        }));
        let shared = Arc::new(shared);
        *shared.heard.lock() = Some(heard);
        Client(Arc::new(Inner {
            shared,
            threads: Mutex::new(Vec::new()),
        }))
    }

    /// What `query` answers now, read on the caller's thread and not kept: for a caller that
    /// needs the answer at once, and once.
    pub fn read<Q: Query>(&self, query: Q) -> Result<Q::Output, crate::ClientError> {
        query.run(&self.0.shared.store, self.0.shared.now())
    }

    /// The value `query` answers, kept read. Never reads the store here: a value someone already
    /// holds is handed over as it stands, and a new one is queued to be read (or, for an inline
    /// client, read now).
    pub fn live<Q: Query>(&self, query: Q) -> Live<Q::Output> {
        let shared = &self.0.shared;
        let (entry, fresh) = {
            let mut registry = shared.registry.lock();
            let held = registry
                .by_query
                .entry(TypeId::of::<Q>())
                .or_insert_with(|| Box::new(Held::<Q>::new()))
                .downcast_mut::<Held<Q>>()
                .expect("each type of query has a map of its own type");
            let bucket = query.bucket();
            let found = held
                .get(&bucket)
                .into_iter()
                .flatten()
                .find(|(asked, _)| *asked == query)
                .and_then(|(_, entry)| entry.upgrade());
            match found {
                Some(entry) => (entry, false),
                None => {
                    let entry = Entry::new(query.clone());
                    held.entry(bucket)
                        .or_default()
                        .retain(|(_, entry)| entry.strong_count() > 0);
                    held.entry(bucket)
                        .or_default()
                        .push((query, Arc::downgrade(&entry)));
                    // Values nobody holds any more are let go of as the map doubles, so the cost
                    // of finding them is paid once for every one made.
                    if held.len().is_power_of_two() {
                        held.retain(|_, bucket| {
                            bucket.retain(|(_, entry)| entry.strong_count() > 0);
                            !bucket.is_empty()
                        });
                    }
                    let job: Arc<dyn Job> = entry.clone();
                    registry.every.push(Arc::downgrade(&job));
                    (entry, true)
                }
            }
        };
        let live = entry.live();
        if fresh {
            shared.enqueue(Arc::downgrade(&(entry as Arc<dyn Job>)));
        }
        live
    }

    /// Read `queries` before anyone asks, so the first to ask finds them read. Kept for as long as
    /// the [`Live`]s returned are.
    pub fn prime<Q: Query>(&self, queries: impl IntoIterator<Item = Q>) -> Vec<Live<Q::Output>> {
        queries.into_iter().map(|query| self.live(query)).collect()
    }

    /// Apply `command`, in order after every command given before it. Never waits on the store:
    /// the ticket says what was done once it has been. An inline client applies it here, now.
    pub fn act<A: Act>(&self, command: A) -> Ticket<A::Done> {
        let (said, ticket) = Ticket::new();
        let shared = &self.0.shared;
        if shared.inline {
            // The ticket is held by the caller, which this returns to.
            let _ = said.send(crate::act::apply(&shared.store, command));
            return ticket;
        }
        if let Some(acts) = shared.acts.lock().as_ref() {
            let job: Asked = Box::new(move |store| {
                // Whoever asked may have stopped waiting; the change is made all the same.
                let _ = said.send(crate::act::apply(store, command));
            });
            // A thread that has gone leaves the ticket unanswered, which says the command was
            // not done.
            if acts.send(job).is_err() {
                log::error!("a change was not made: the thread that makes them has stopped");
            }
        }
        ticket
    }

    /// Mark stale every value that reads any of `topics`, as a commit that touched them would.
    /// What a timer says when the clock moves past something a value counts ([`Topics::CLOCK`]).
    pub fn invalidate(&self, topics: Topics) {
        self.0.shared.invalidate(topics);
    }

    /// Told each time the store has changed, here or in another process: for what still reads
    /// the store itself rather than through a [`Live`], to read again. Commits that come together
    /// are told once.
    pub fn moved(&self) -> tokio::sync::watch::Receiver<u64> {
        self.0.shared.moved.subscribe()
    }

    /// [`Client::moved`], told only when the store moved in something that touches `topics`: for
    /// what reads the store itself and reads only that, so a burst of arriving mail does not read
    /// again what is about accounts, or keys, or rules.
    pub fn moved_for(&self, topics: Topics) -> tokio::sync::watch::Receiver<u64> {
        let mut held = self.0.shared.moved_for.lock();
        if let Some((_, told)) = held.iter().find(|(wanted, _)| *wanted == topics) {
            return told.subscribe();
        }
        let told = tokio::sync::watch::Sender::new(0);
        let heard = told.subscribe();
        held.push((topics, told));
        heard
    }

    /// Read by `clock` from now on, and read again what reads the clock: for a caller whose
    /// time is not the system's, as a test's that jumps a day ahead.
    pub fn set_clock(&self, clock: Arc<dyn Clock>) {
        *self.0.shared.clock.write() = clock;
        self.0.shared.invalidate(Topics::CLOCK);
    }

    /// The next moment what reads the clock reads differently: a snooze ending. A client with
    /// threads of its own marks it stale then, at its clock's time; a caller whose time moves
    /// otherwise (a test's virtual clock), or whose client reads on its thread, waits for it and
    /// says so ([`Client::set_clock`], [`Client::invalidate`]).
    pub fn next_clock(&self) -> Option<DateTime<Utc>> {
        let shared = &self.0.shared;
        crate::clock::next(&shared.store, shared.now())
    }

    /// Whether this client marks stale what reads the clock by itself: one with threads of its
    /// own does, one that reads on its caller's thread does not.
    pub fn keeps_time(&self) -> bool {
        !self.0.shared.inline
    }

    /// The store, for what the client calls on its callers' behalf (`calls`).
    pub(crate) fn store(&self) -> &SqliteStore {
        &self.0.shared.store
    }

    /// The store, for a test's double at a seam to keep what a server would have given: never
    /// in the program, which has no `testing`.
    #[cfg(feature = "testing")]
    pub fn store_for_test(&self) -> Arc<SqliteStore> {
        self.store_arc()
    }

    pub(crate) fn reached(&self) -> &OnceLock<crate::net::Reach> {
        &self.0.shared.reach
    }

    pub(crate) fn store_arc(&self) -> Arc<SqliteStore> {
        self.0.shared.store.clone()
    }

    /// Whether nothing is waiting to be read or being read.
    pub fn idle(&self) -> bool {
        self.0.shared.busy.load(Ordering::Acquire) == 0
    }

    /// For an inline client: act on every commit made since the last time, here. Nothing for a
    /// client with threads of its own, which does that by itself.
    pub fn settle(&self) {
        self.0.shared.settle();
    }

    /// For an inline client, told each time the store commits, so its caller can
    /// [`settle`](Self::settle) it on the thread it reads on. `None` for a client with threads of
    /// its own, which hears its commits by itself.
    pub fn committed(&self) -> Option<tokio::sync::watch::Receiver<u64>> {
        let shared = &self.0.shared;
        shared.inline.then(|| shared.committed.subscribe())
    }

    /// A handle that does not keep this client's threads from stopping ([`WeakClient`]).
    pub fn downgrade(&self) -> WeakClient {
        WeakClient(Arc::downgrade(&self.0))
    }

    /// Fetch mail: every account's link, polled, pushed to and fetched on request by the one
    /// scheduler, reaching the servers through `seams`. Started once; a later call changes
    /// nothing and is handed `None`, so the seams are those of the first window to ask and any
    /// later window's are dropped.
    ///
    /// A client with threads of its own runs it on one, until the client stops. One that reads
    /// on its caller's thread hands back what to run, on a tokio runtime, for as long as fetching
    /// should go on.
    pub fn fetch(&self, seams: Seams) -> Option<FetchRun> {
        let shared = &self.0.shared;
        // Asking reads nothing: the accounts are found by the fetching thread ([`Fetcher::begin`]),
        // so this is safe to call from the window's own thread. Only the first window's seams
        // are kept; a later call is handed `None` and its seams are dropped.
        if shared.fetcher.set(Fetcher::new(seams)).is_err() {
            return None;
        }
        if shared.inline {
            return crate::fetch::run(shared.clone()).map(|run| Box::pin(run) as FetchRun);
        }
        let (stop, stopped) = tokio::sync::oneshot::channel::<Duration>();
        *shared.fetch_stop.lock() = Some(stop);
        let shared = shared.clone();
        self.0.threads.lock().push(spawn("mailo-fetch", move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("the system starts a runtime");
            // How long a stop gives the passes to end: what `stop` was given, or a short while
            // when the client is dropped without being stopped.
            let mut within = Duration::from_millis(500);
            if let Some(run) = crate::fetch::run(shared) {
                within = runtime.block_on(async {
                    tokio::select! {
                        () = run => within,
                        // Told to stop, or the client gone.
                        told = stopped => told.unwrap_or(within),
                    }
                });
            }
            // Stopped: the scheduler is dropped, and with it the sender of each running pass's
            // cancel signal, which ends the pass; the runtime then waits, up to `within`, for
            // the passes and watches on their threads to have ended.
            runtime.shutdown_timeout(within);
        }));
        None
    }

    /// Where every account's fetching stands, told each time it moves; `None` until
    /// [`Client::fetch`].
    pub fn fetched(&self) -> Option<tokio::sync::watch::Receiver<Arc<Fetched>>> {
        self.0.shared.fetcher().map(Fetcher::fetched)
    }

    /// Tell `account`'s link that `event` happened: a pass asked for, a credential replaced.
    /// Never waits; nothing before [`Client::fetch`].
    pub fn tell(&self, account: AccountId, event: Event) {
        if let Some(fetcher) = self.0.shared.fetcher() {
            fetcher.tell(account, event);
        }
    }

    /// A folder's place was chosen: fetch it now, unless it was a moment ago or a pass is
    /// running on its account.
    pub fn open_folder(&self, mailbox: MailboxRef) {
        let shared = &self.0.shared;
        if let Some(fetcher) = shared.fetcher() {
            fetcher.open_folder(mailbox, shared.now());
        }
    }

    /// Every folder fetched on opening is fetched again the next time it is opened.
    pub fn forget_folders(&self) {
        let shared = &self.0.shared;
        if let Some(fetcher) = shared.fetcher() {
            fetcher.forget_folders(shared.now());
        }
    }

    /// Stop the client's threads, waiting at most `within` for them: a value being read is let
    /// finish and its answer thrown away, and nothing waiting is read. Every clone of this client
    /// stops with it.
    pub fn shutdown(self, within: Duration) {
        self.0.stop(within);
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        self.stop(Duration::from_millis(500));
    }
}

impl Inner {
    fn stop(&self, within: Duration) {
        self.shared.stopping.store(true, Ordering::Release);
        // Nothing more is taken; what was given is still applied.
        self.shared.acts.lock().take();
        if let Some(stop) = self.shared.fetch_stop.lock().take() {
            // A fetch thread that has ended has nothing left to stop.
            let _ = stop.send(within);
        }
        self.shared.queued.notify_all();
        self.shared.poke_clock();
        crate::external::wake(&self.shared);
        let deadline = Instant::now() + within;
        for thread in self.threads.lock().drain(..) {
            while !thread.is_finished() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(2));
            }
            if thread.is_finished() {
                // A thread that panicked has already said so; there is nothing left to stop.
                let _ = thread.join();
            }
        }
        // What the last changes wrote is folded back into the database, once they have all been
        // applied. Passive, because this runs on the quitting thread: it never waits on a reader
        // something else still holds, and a log it leaves large is cut down at the next start.
        self.shared
            .store
            .checkpoint(Checkpoint::Passive)
            .or_log("the log could not be folded back at quit");
    }
}

/// How many threads read: one fewer than the store's readers, and at least one.
fn readers(store: &SqliteStore) -> usize {
    store.reader_count().saturating_sub(1).max(1)
}

/// Run `work` for one of the client's threads, and say what it came to: `None` when it panicked,
/// which is logged. A thread that lets a panic unwind is gone for good, and what it did for the
/// client (applying the person's changes, hearing commits) stops without a word, so each pass of
/// its loop is run through this and the next one goes on.
pub(crate) fn survive<T>(what: &str, work: impl FnOnce() -> T) -> Option<T> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)) {
        Ok(done) => Some(done),
        Err(panicked) => {
            let why = panicked
                .downcast_ref::<&str>()
                .map(|text| (*text).to_owned())
                .or_else(|| panicked.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "no reason given".to_owned());
            log::error!("{what} panicked: {why}");
            None
        }
    }
}

pub(crate) fn spawn(name: &str, run: impl FnOnce() + Send + 'static) -> JoinHandle<()> {
    std::thread::Builder::new()
        .name(name.to_owned())
        .spawn(run)
        .expect("the system starts a thread")
}

impl Shared {
    fn new(store: Arc<SqliteStore>, clock: Arc<dyn Clock>, inline: bool) -> Self {
        Self {
            store,
            clock: parking_lot::RwLock::new(clock),
            registry: Mutex::new(Registry::default()),
            queue: Mutex::new(VecDeque::new()),
            queued: Condvar::new(),
            busy: AtomicUsize::new(0),
            stopping: AtomicBool::new(false),
            inline,
            heard: Mutex::new(None),
            committed: Arc::new(tokio::sync::watch::Sender::new(0)),
            moved: tokio::sync::watch::Sender::new(0),
            moved_for: Mutex::new(Vec::new()),
            external: Mutex::new(None),
            acts: Mutex::new(None),
            fetcher: OnceLock::new(),
            fetch_stop: Mutex::new(None),
            clock_poked: Mutex::new(false),
            clock_told: Condvar::new(),
            reach: OnceLock::new(),
        }
    }

    pub(crate) fn store(&self) -> &SqliteStore {
        &self.store
    }

    pub(crate) fn store_arc(&self) -> Arc<SqliteStore> {
        self.store.clone()
    }

    /// Sleep the clock thread up to `wait`, or until it is told to look again.
    pub(crate) fn clock_sleep(&self, wait: Duration) {
        let mut poked = self.clock_poked.lock();
        if !*poked && !self.stopping() {
            self.clock_told.wait_for(&mut poked, wait);
        }
        *poked = false;
    }

    fn poke_clock(&self) {
        *self.clock_poked.lock() = true;
        self.clock_told.notify_all();
    }

    pub(crate) fn fetcher(&self) -> Option<&Fetcher> {
        self.fetcher.get()
    }

    /// Fetching may have written to the store. Heard by itself through the store's commits;
    /// a client that reads on its caller's thread acts on them now, since fetching runs there.
    pub(crate) fn stored(&self) {
        if self.inline {
            self.settle();
        }
    }

    fn settle(&self) {
        let topics = self
            .heard
            .lock()
            .as_ref()
            .map(|heard| {
                heard
                    .try_iter()
                    .fold(Topics::empty(), |all, commit| all | commit.topics)
            })
            .unwrap_or_default();
        if !topics.is_empty() {
            self.invalidate(topics);
        }
    }

    pub(crate) fn stopping(&self) -> bool {
        self.stopping.load(Ordering::Acquire)
    }

    pub(crate) fn now(&self) -> DateTime<Utc> {
        self.clock.read().now()
    }

    /// Put `job` in the queue, already marked as queued. An inline client reads it now instead.
    fn enqueue(&self, job: Weak<dyn Job>) {
        if self.inline {
            if let Some(job) = job.upgrade() {
                while job.run(&self.store, self.now()) {}
            }
            return;
        }
        self.busy.fetch_add(1, Ordering::AcqRel);
        self.queue.lock().push_back(job);
        self.queued.notify_one();
    }

    /// Mark stale every value held that reads any of `topics`.
    pub(crate) fn invalidate(&self, topics: Topics) {
        if let Some(fetcher) = self.fetcher() {
            if topics.intersects(Topics::ACCOUNTS) {
                fetcher.accounts(&self.store);
            }
            if topics.intersects(Topics::OUTBOX) {
                fetcher.outbox_moved();
            }
        }
        if topics.intersects(crate::clock::MOVES) {
            self.poke_clock();
        }
        self.moved.send_modify(|moved| *moved += 1);
        for (wanted, told) in self.moved_for.lock().iter() {
            if wanted.intersects(topics) {
                told.send_modify(|moved| *moved += 1);
            }
        }
        let stale: Vec<Arc<dyn Job>> = {
            let mut registry = self.registry.lock();
            registry.every.retain(|job| job.strong_count() > 0);
            registry
                .every
                .iter()
                .filter_map(Weak::upgrade)
                .filter(|job| job.reads().intersects(topics))
                .collect()
        };
        for job in stale {
            if job.stale() {
                self.enqueue(Arc::downgrade(&job));
            }
        }
    }

    /// A reader thread: read what the queue holds until the client stops.
    fn read(&self) {
        loop {
            let job = {
                let mut queue = self.queue.lock();
                loop {
                    if self.stopping() {
                        return;
                    }
                    if let Some(job) = queue.pop_front() {
                        break job;
                    }
                    self.queued.wait(&mut queue);
                }
            };
            // A value nobody holds any more is not read: dropping it took it out of the queue.
            if let Some(job) = job.upgrade()
                && job.run(&self.store, self.now())
                && !self.stopping()
            {
                self.enqueue(Arc::downgrade(&job));
            }
            self.busy.fetch_sub(1, Ordering::AcqRel);
        }
    }

    /// The change thread: hear each commit, gather the ones that come together, and mark stale
    /// what reads what they touched.
    fn hear(&self, heard: &Receiver<Committed>) {
        while !self.stopping() {
            let first = match heard.recv_timeout(Duration::from_millis(100)) {
                Ok(commit) => commit,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => return,
            };
            let mut topics = first.topics;
            let until = Instant::now() + AT_MOST;
            loop {
                let left = until.saturating_duration_since(Instant::now());
                match heard.recv_timeout(GATHER.min(left)) {
                    Ok(commit) if !left.is_zero() => topics |= commit.topics,
                    Ok(commit) => {
                        topics |= commit.topics;
                        break;
                    }
                    Err(RecvTimeoutError::Timeout) => break,
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            }
            survive("marking what changed", || self.invalidate(topics));
        }
    }
}
