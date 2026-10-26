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
//! - **Act** (`mailo-act`) applies [`Command`]s, one at a time, in the order they were given: the
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

use crate::act::{Acted, Command, Ticket};
use crate::fetch::{Fetched, Fetcher, Seams};
use crate::live::{Entry, Job, Live, Query};
use chrono::{DateTime, Utc};
use mail_core::fetch::Event;
use mail_core::mail::{Clock, SystemClock};
use mail_domain::MailboxRef;
use mail_store::{Committed, SqliteStore, Topics};
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
    /// One more each time values were marked stale, by a commit here or another process's.
    moved: tokio::sync::watch::Sender<u64>,
    /// How the external watch is woken: by a write to the store's file, or to stop.
    pub(crate) external: Mutex<Option<std::sync::mpsc::Sender<()>>>,
    /// Where commands go to be applied; `None` for an inline client, and once stopping.
    acts: Mutex<Option<std::sync::mpsc::Sender<Asked>>>,
    /// Fetching, once a caller asked for it.
    fetcher: OnceLock<Fetcher>,
    /// Stops the fetch thread.
    fetch_stop: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    /// Wakes the clock thread to find its next moment again: set, then told.
    clock_poked: Mutex<bool>,
    clock_told: Condvar,
    /// How servers are reached, once the program says (`net`).
    pub(crate) reach: OnceLock<crate::net::Reach>,
}

/// Fetching run where the caller runs it: what [`Client::fetch`] hands a client that reads on
/// its caller's thread. Runs until it is dropped.
pub type FetchRun = std::pin::Pin<Box<dyn std::future::Future<Output = ()>>>;

/// A command, and where to say what it did.
type Asked = (Command, tokio::sync::oneshot::Sender<Acted>);

#[derive(Default)]
struct Registry {
    /// The values of each type of query, found by equality: a query is asked with what it reads
    /// (filters, accounts), which need not hash, and a caller holds a handful, not thousands.
    by_query: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
    /// Every value, whatever its query, for the change thread to go through.
    every: Vec<Weak<dyn Job>>,
}

impl Client {
    /// The client over `store`, its threads started.
    pub fn open(store: Arc<SqliteStore>, config: ClientConfig) -> Client {
        let heard = store.subscribe();
        let shared = Arc::new(Shared::new(store, config.clock, false));
        let mut threads = Vec::new();
        for n in 0..readers(&shared.store) {
            let shared = shared.clone();
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
                // person made is not dropped because the window closed after it.
                for (command, said) in asked {
                    // Whoever asked may have stopped waiting; the change is made all the same.
                    let _ = said.send(crate::act::apply(&shared.store, command));
                }
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
        let heard = store.subscribe();
        let shared = Arc::new(Shared::new(store, clock, true));
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
                .or_insert_with(|| Box::new(Vec::<(Q, Weak<Entry<Q>>)>::new()))
                .downcast_mut::<Vec<(Q, Weak<Entry<Q>>)>>()
                .expect("each type of query has a list of its own type");
            let found = held
                .iter()
                .find(|(asked, _)| *asked == query)
                .and_then(|(_, entry)| entry.upgrade());
            match found {
                Some(entry) => (entry, false),
                None => {
                    held.retain(|(_, entry)| entry.strong_count() > 0);
                    let entry = Entry::new(query.clone());
                    held.push((query, Arc::downgrade(&entry)));
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
    /// the [`Live`]s
    /// returned are.
    pub fn prime<Q: Query>(&self, queries: impl IntoIterator<Item = Q>) -> Vec<Live<Q::Output>> {
        queries.into_iter().map(|query| self.live(query)).collect()
    }

    /// Apply `command`, in order after every command given before it. Never waits on the store:
    /// the ticket says what was done once it has been. An inline client applies it here, now.
    pub fn act(&self, command: Command) -> Ticket {
        let (said, ticket) = Ticket::new();
        let shared = &self.0.shared;
        if shared.inline {
            // The ticket is held by the caller, which this returns to.
            let _ = said.send(crate::act::apply(&shared.store, command));
            return ticket;
        }
        if let Some(acts) = shared.acts.lock().as_ref() {
            // A thread that has gone leaves the ticket unanswered, which says the command was
            // not done.
            let _ = acts.send((command, said));
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

    /// Fetch mail: every account's link, polled, pushed to and fetched on request by the one
    /// scheduler, reaching the servers through `seams`. Started once; a later call changes
    /// nothing and is handed `None`.
    ///
    /// A client with threads of its own runs it on one, until the client stops. One that reads
    /// on its caller's thread hands back what to run, on a tokio runtime, for as long as fetching
    /// should go on.
    pub fn fetch(&self, seams: Seams) -> Option<FetchRun> {
        let shared = &self.0.shared;
        let fetcher = Fetcher::new(seams, &shared.store, shared.now());
        if shared.fetcher.set(fetcher).is_err() {
            return None;
        }
        if shared.inline {
            return crate::fetch::run(shared.clone()).map(|run| Box::pin(run) as FetchRun);
        }
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        *shared.fetch_stop.lock() = Some(stop);
        let shared = shared.clone();
        self.0.threads.lock().push(spawn("mailo-fetch", move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("the system starts a runtime");
            if let Some(run) = crate::fetch::run(shared) {
                runtime.block_on(async {
                    tokio::select! {
                        () = run => {}
                        // Told to stop, or the client gone.
                        _ = stopped => {}
                    }
                });
            }
            // A pass or a watch parked in a socket is not waited for.
            runtime.shutdown_background();
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
            let _ = stop.send(());
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
    }
}

/// How many threads read: one fewer than the store's readers, and at least one.
fn readers(store: &SqliteStore) -> usize {
    store.reader_count().saturating_sub(1).max(1)
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
            moved: tokio::sync::watch::Sender::new(0),
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
            self.invalidate(topics);
        }
    }
}
