//! A value read from the store and kept read: what a [`Query`] answers, as a [`Live`].

use crate::ClientError;
use chrono::{DateTime, Utc};
use mail_store::{Generation, SqliteStore, Topics};
use parking_lot::Mutex;
use std::sync::Arc;

/// Something read from the store.
///
/// Its value is its key: two components asking an equal query share one value and one reading
/// of it ([`crate::Client::live`]). So a query holds only what it is asked with, and anything it
/// depends on that is not in the database is part of it.
pub trait Query: Clone + Eq + Send + Sync + 'static {
    /// What it answers.
    type Output: PartialEq + Send + Sync + 'static;

    /// What it reads: a commit that touched none of these leaves its answer as it was.
    const READS: Topics;

    /// Read it. Every read made here is of one moment of the database
    /// ([`SqliteStore::read_pinned`]), and `now` is the client's clock.
    fn run(&self, store: &SqliteStore, now: DateTime<Utc>) -> Result<Self::Output, ClientError>;
}

/// A value as it stands: being read for the first time, read, or failing to be read again.
#[derive(Debug)]
pub enum Snapshot<T> {
    /// Not read yet. `last` is what an earlier query showed, for a front end to keep showing
    /// rather than flash empty; the client itself never sets it.
    Loading { last: Option<Arc<T>> },
    /// Read, and every commit up to `as_of` is in it.
    Ready { value: Arc<T>, as_of: Generation },
    /// The last reading failed. `last` is the one before that succeeded, if any did.
    Failed {
        error: ClientError,
        last: Option<Arc<T>>,
    },
}

impl<T> Clone for Snapshot<T> {
    fn clone(&self) -> Self {
        match self {
            Snapshot::Loading { last } => Snapshot::Loading { last: last.clone() },
            Snapshot::Ready { value, as_of } => Snapshot::Ready {
                value: value.clone(),
                as_of: *as_of,
            },
            Snapshot::Failed { error, last } => Snapshot::Failed {
                error: error.clone(),
                last: last.clone(),
            },
        }
    }
}

impl<T: PartialEq> PartialEq for Snapshot<T> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Snapshot::Loading { last: a }, Snapshot::Loading { last: b }) => a == b,
            (
                Snapshot::Ready {
                    value: a,
                    as_of: at,
                },
                Snapshot::Ready {
                    value: b,
                    as_of: bt,
                },
            ) => at == bt && a == b,
            (Snapshot::Failed { error: a, last: al }, Snapshot::Failed { error: b, last: bl }) => {
                a == b && al == bl
            }
            _ => false,
        }
    }
}

impl<T> Snapshot<T> {
    /// The value to show: the one read, or while loading or failing the last one there was.
    pub fn value(&self) -> Option<&Arc<T>> {
        match self {
            Snapshot::Ready { value, .. } => Some(value),
            Snapshot::Loading { last } | Snapshot::Failed { last, .. } => last.as_ref(),
        }
    }

    /// The same state, holding `last` as the value to show until there is one of its own.
    pub fn showing(self, last: Option<Arc<T>>) -> Self {
        match self {
            Snapshot::Loading { last: None } => Snapshot::Loading { last },
            Snapshot::Failed { error, last: None } => Snapshot::Failed { error, last },
            kept => kept,
        }
    }
}

/// A value the client keeps read, held. Cheap to clone; when the last clone of every `Live` for
/// a query is dropped, the client stops reading it.
pub struct Live<T> {
    source: Arc<dyn Source<T>>,
    told: tokio::sync::watch::Receiver<u64>,
}

impl<T> Clone for Live<T> {
    fn clone(&self) -> Self {
        Self {
            source: self.source.clone(),
            told: self.told.clone(),
        }
    }
}

impl<T> std::fmt::Debug for Live<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Live")
    }
}

impl<T: Send + Sync + 'static> Live<T> {
    /// The value as it stands. Never reads the store and never waits on a reading: a lock held
    /// for as long as it takes to clone an `Arc`.
    pub fn snapshot(&self) -> Snapshot<T> {
        self.source.snapshot()
    }

    /// Resolves once the value has changed since this `Live` last saw it, with `false` when the
    /// client has stopped and it never will again. A new answer equal to the last one is not a
    /// change.
    pub async fn changed(&mut self) -> bool {
        self.told.changed().await.is_ok()
    }

    /// Wait for the value to be read, here on this thread. For tests.
    #[cfg(feature = "testing")]
    pub fn wait(&self) -> Arc<T> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            match self.snapshot() {
                Snapshot::Ready { value, .. } => return value,
                Snapshot::Failed { error, .. } => panic!("the value could not be read: {error}"),
                Snapshot::Loading { .. } => {}
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the value was not read within ten seconds"
            );
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }
}

/// What a [`Live`] reads its value from: an [`Entry`], seen without its query's type.
trait Source<T>: Send + Sync {
    fn snapshot(&self) -> Snapshot<T>;
}

/// An entry seen as work: what the client's threads and the change thread hold.
pub(crate) trait Job: Send + Sync {
    /// What it reads.
    fn reads(&self) -> Topics;
    /// The store changed under it. Whether it should be put in the queue: not when it already
    /// is, and not while it is being read, when it is read again as that reading ends.
    fn stale(&self) -> bool;
    /// Read it, and keep the answer. Whether it went stale while being read and wants the queue
    /// again.
    fn run(&self, store: &SqliteStore, now: DateTime<Utc>) -> bool;
}

/// One query's value, shared by every [`Live`] for it.
pub(crate) struct Entry<Q: Query> {
    query: Q,
    state: Mutex<State<Q::Output>>,
    tell: tokio::sync::watch::Sender<u64>,
}

struct State<T> {
    snapshot: Snapshot<T>,
    /// In the queue, not yet taken.
    queued: bool,
    /// Being read now.
    running: bool,
    /// Went stale while being read: read again as soon as this reading ends.
    again: bool,
}

impl<Q: Query> Entry<Q> {
    /// A value not read yet, and marked as queued: whoever makes one queues it, or reads it.
    pub(crate) fn new(query: Q) -> Arc<Self> {
        Arc::new(Self {
            query,
            state: Mutex::new(State {
                snapshot: Snapshot::Loading { last: None },
                queued: true,
                running: false,
                again: false,
            }),
            tell: tokio::sync::watch::Sender::new(0),
        })
    }

    /// A [`Live`] of this entry, which keeps it alive.
    pub(crate) fn live(self: &Arc<Self>) -> Live<Q::Output> {
        Live {
            told: self.tell.subscribe(),
            source: self.clone(),
        }
    }
}

impl<Q: Query> Source<Q::Output> for Entry<Q> {
    fn snapshot(&self) -> Snapshot<Q::Output> {
        self.state.lock().snapshot.clone()
    }
}

impl<Q: Query> Job for Entry<Q> {
    fn reads(&self) -> Topics {
        Q::READS
    }

    fn stale(&self) -> bool {
        let mut state = self.state.lock();
        if state.running {
            state.again = true;
            return false;
        }
        if state.queued {
            return false;
        }
        state.queued = true;
        true
    }

    fn run(&self, store: &SqliteStore, now: DateTime<Utc>) -> bool {
        {
            let mut state = self.state.lock();
            state.queued = false;
            state.running = true;
        }
        // Before the reading begins, so every commit up to it is one the reading sees.
        let as_of = store.generation();
        // A reading that panics is a failed reading, not a reader thread gone: `running` is
        // cleared below either way, so the entry reads again at the next commit.
        let answer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            store
                .read_pinned(|| self.query.run(store, now))
                .map_err(ClientError::from)
                .and_then(|answer| answer)
        }))
        .unwrap_or_else(|panicked| {
            let what = panicked
                .downcast_ref::<&str>()
                .map(|text| (*text).to_owned())
                .or_else(|| panicked.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "a reading panicked".to_owned());
            Err(ClientError::Panicked(what))
        });
        let mut state = self.state.lock();
        state.running = false;
        let last = state.snapshot.value().cloned();
        let (next, news) = match answer {
            // The same answer again is not news, but it is newer: a front end waiting for its
            // write to be in the value reads `as_of`.
            Ok(value) => match last {
                Some(last) if *last == value => (
                    Snapshot::Ready { value: last, as_of },
                    matches!(state.snapshot, Snapshot::Failed { .. }),
                ),
                _ => (
                    Snapshot::Ready {
                        value: Arc::new(value),
                        as_of,
                    },
                    true,
                ),
            },
            Err(error) => {
                log::warn!("a value could not be read: {error}");
                (Snapshot::Failed { error, last }, true)
            }
        };
        state.snapshot = next;
        let again = std::mem::take(&mut state.again);
        if again {
            state.queued = true;
        }
        drop(state);
        if news {
            self.tell.send_modify(|told| *told += 1);
        }
        again
    }
}
