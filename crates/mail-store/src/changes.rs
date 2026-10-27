//! What a commit changed, said to whoever is listening.
//!
//! Every write to the mail goes through the store, so the store is the one place that can say
//! that something changed and roughly what. A write method names the [`Topics`] it touches when
//! it takes the writer; once the outermost hold of the writer is let go of after a commit, the
//! store sends [`Committed`] to every subscriber. A commit that named nothing says [`Topics::ALL`],
//! so a write that forgot to say what it touched costs a recount, never a stale window.
//!
//! The listener is `mail-client`, which reads again what depends on what changed. The store
//! itself never waits on a listener: a subscriber is a channel, and sending to one does not block.

use parking_lot::Mutex;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};

bitflags::bitflags! {
    /// What a write touched, by what a reader would read again because of it.
    ///
    /// Coarse on purpose: one per kind of thing a window shows, not one per table. A topic is
    /// what a query says it depends on, so a finer one is only worth having when some query
    /// would read less because of it.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub struct Topics: u32 {
        /// The accounts, their plans and capabilities.
        const ACCOUNTS = 1 << 0;
        /// The labels and which messages carry them.
        const LABELS = 1 << 1;
        /// The server folders an account lists.
        const FOLDERS = 1 << 2;
        /// Messages, threads and their summaries: what every list and every unread count reads.
        const SUMMARIES = 1 << 3;
        /// The drafts, and the messages held to leave later.
        const DRAFTS = 1 << 4;
        /// The work queued for a server.
        const OUTBOX = 1 << 5;
        /// The saved views.
        const VIEWS = 1 << 6;
        /// The address book: contacts, their groups and the books they came from.
        const CONTACTS = 1 << 7;
        /// The filing rules and the vacation reply.
        const RULES = 1 << 8;
        /// OpenPGP keys, Autocrypt peers and S/MIME certificates.
        const KEYS = 1 << 9;
        /// The templates.
        const TEMPLATES = 1 << 10;
        /// The identities an account sends as.
        const IDENTITIES = 1 << 11;
        /// The follow-up reminders on sent mail.
        const FOLLOW_UPS = 1 << 12;
        /// Nothing written: time passed, and what depends on the clock (a snooze ending, a
        /// scheduled send coming due) may read differently now. The store never says it; a
        /// timer does.
        const CLOCK = 1 << 13;
        /// Anything at all.
        const ALL = u32::MAX;
    }
}

/// Which commit a reader has seen up to: each one the store says it has made counts one more.
///
/// A value read before a read began says that read saw at least every commit up to it, which is
/// what lets a window know a list it shows already has the write it made.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Generation(pub u64);

/// One commit, or several made under one hold of the writer, as the store says it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Committed {
    /// What it touched; [`Topics::ALL`] when the write did not say.
    pub topics: Topics,
    /// The store's count of commits, this one included.
    pub generation: Generation,
}

/// One who hears the commits: where they go, and what wakes it once one has gone there.
struct Subscriber {
    to: Sender<Committed>,
    wake: Option<Wake>,
}

/// What a subscriber is woken by, once a commit is waiting for it.
pub type Wake = Arc<dyn Fn() + Send + Sync>;

impl std::fmt::Debug for Subscriber {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Subscriber")
            .field("wakes", &self.wake.is_some())
            .finish()
    }
}

/// The subscribers, and what the writer has done since it was last let go of.
///
/// The writer is one connection behind one lock, so only the thread holding it changes these;
/// they are atomics because the commit hook, which marks a commit, has to own its half.
#[derive(Debug, Default)]
pub(crate) struct Changes {
    subscribers: Mutex<Vec<Subscriber>>,
    generation: AtomicU64,
    /// How deep the thread holding the writer holds it: a write method takes it and then calls
    /// helpers that take it again, and only the outermost letting go says anything.
    depth: AtomicUsize,
    /// What the write methods under the current hold said they touch.
    declared: AtomicU32,
    /// Whether a commit has gone through under the current hold. Set by the connection's commit
    /// hook, which SQLite calls only for a transaction that wrote.
    committed: Arc<AtomicBool>,
    /// The writer's count of rows changed when the current hold began. A `DELETE` that matched
    /// nothing still commits a write transaction, and is not worth telling anyone about.
    rows_at_start: AtomicU64,
}

impl Changes {
    /// The flag the writer's commit hook sets.
    pub(crate) fn commit_flag(&self) -> Arc<AtomicBool> {
        self.committed.clone()
    }

    /// The writer was taken, once more. `rows` is its count of rows changed so far.
    pub(crate) fn enter(&self, rows: u64) {
        if self.depth.fetch_add(1, Ordering::Relaxed) == 0 {
            self.rows_at_start.store(rows, Ordering::Relaxed);
        }
    }

    /// The write under the current hold touches `topics`.
    pub(crate) fn declare(&self, topics: Topics) {
        self.declared.fetch_or(topics.bits(), Ordering::Relaxed);
    }

    /// The writer was let go of, once, with `rows` its count of rows changed so far. The
    /// outermost letting go, after a commit that changed a row, says what was committed; one with
    /// nothing committed (only reads, a transaction rolled back, a write that matched no row)
    /// says nothing. Called while the writer is still held, so the generations are in the order
    /// the commits were made.
    pub(crate) fn leave(&self, rows: u64) {
        if self.depth.fetch_sub(1, Ordering::Relaxed) != 1 {
            return;
        }
        let declared = Topics::from_bits_retain(self.declared.swap(0, Ordering::Relaxed));
        let committed = self.committed.swap(false, Ordering::Relaxed);
        if !committed || rows == self.rows_at_start.load(Ordering::Relaxed) {
            return;
        }
        let topics = if declared.is_empty() {
            Topics::ALL
        } else {
            declared
        };
        self.publish(topics);
    }

    /// Say `topics` changed, to every subscriber still listening.
    fn publish(&self, topics: Topics) {
        let generation = Generation(self.generation.fetch_add(1, Ordering::Release) + 1);
        let committed = Committed { topics, generation };
        // A subscriber that has gone is dropped from the list; the others are told.
        self.subscribers.lock().retain(|subscriber| {
            let heard = subscriber.to.send(committed).is_ok();
            if heard && let Some(wake) = &subscriber.wake {
                wake();
            }
            heard
        });
    }

    /// Hear every commit from now on, `wake` called each time one is waiting.
    pub(crate) fn subscribe(&self, wake: Option<Wake>) -> Receiver<Committed> {
        let (to, rx) = channel();
        self.subscribers.lock().push(Subscriber { to, wake });
        rx
    }

    /// The generation of the last commit said.
    pub(crate) fn generation(&self) -> Generation {
        Generation(self.generation.load(Ordering::Acquire))
    }
}
