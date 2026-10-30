//! The window keeping the router's index of mail: the recent conversations and the people in the
//! address book, pushed to `org.quire.Intents1.Index` so the desktop's launcher finds them as the
//! person types without starting mail.
//!
//! What is pushed is [`mail_core::findable`]'s, as [`IndexEntry`]s. Only the window pushes:
//! `mailo intents` is started for one request and gone, while the window is open for as long as
//! the person reads mail, and the router finds a running app's entries only while it runs.
//!
//! - **At the start**, and whenever the router comes onto the bus (`NameOwnerChanged`): `Reset`
//!   with a fresh epoch, then everything, in batches of at most [`BATCH`].
//! - **When the store is written to** (`mail_client::on_write`, what the client hears any commit by, its
//!   own and a watch's): what moved since the last push, upserts and removals, at most one push
//!   every [`EVERY`], so a sync's burst of commits is one push.
//! - **When the router refuses a push as `Malformed`**: it has a new epoch or none (it restarted,
//!   or marked mail stale), so `Reset` and everything again.
//! - **With no router on the bus**: nothing is read and nothing pushed; a debug line says so. It
//!   is pushed to when it comes.
//!
//! All of it on threads of its own, so nothing here waits on the thread that draws, and nothing
//! the person sees says the router is away. No crate of the router is depended on: the batch is
//! [`super::wire`]'s, and the calls are two plain method calls.

use super::ROUTER;
use super::wire::{IndexBatch, IndexEntry};
use mail_core::SqliteStore;
use mail_core::findable::{self, Changes, Findables};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};
use zbus::blocking::Connection;
use zbus::blocking::fdo::DBusProxy;

/// Where the router serves its index.
pub const PATH: &str = "/org/quire/Intents1";

/// The router's index interface.
pub const INTERFACE: &str = "org.quire.Intents1.Index";

/// The error the router answers a push in another epoch with.
const MALFORMED: &str = "org.quire.Intents1.Error.Malformed";

/// The most entries, upserts and removals together, in one push.
pub const BATCH: usize = 500;

/// The least time between two pushes after the first.
pub const EVERY: Duration = Duration::from_secs(2);

/// What wakes the pusher.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Nudge {
    /// The store was written to.
    Changed,
    /// The router came onto the bus.
    RouterCame,
}

/// Tells the pusher that the store was written to.
#[derive(Debug, Clone)]
pub struct Changed(Sender<Nudge>);

impl Changed {
    /// The store was written to. Pushed after at most [`EVERY`] (sooner after a quiet spell).
    pub fn said(&self) {
        // Only fails once the pusher has gone, with nobody left to push to.
        let _ = self.0.send(Nudge::Changed);
    }
}

/// Keep the router's index of `store` for as long as the process runs: the window's, once its
/// store is open. Returns at once; a session bus that cannot be reached is a debug line.
pub fn start(store: Arc<SqliteStore>) {
    let started = std::thread::Builder::new()
        .name("mailo-index".to_owned())
        .spawn(move || {
            let connection = match Connection::session() {
                Ok(connection) => connection,
                Err(why) => {
                    log::debug!("the launcher's index of mail is off: the session bus: {why}");
                    return;
                }
            };
            let reading = store.clone();
            let read = move || {
                findable::read(&reading, chrono::Utc::now())
                    .map_err(|why| log::debug!("mail could not be read for the launcher: {why}"))
                    .ok()
            };
            let changed = match push_on(connection, read, EVERY) {
                Ok(changed) => changed,
                Err(why) => {
                    log::debug!("the launcher's index of mail is off: {why}");
                    return;
                }
            };
            // Held for as long as this thread lives, which is as long as the process.
            let _written = store
                .file()
                .and_then(|file| mail_client::on_write(file, move || changed.said()));
            loop {
                std::thread::park();
            }
        });
    if let Err(why) = started {
        log::debug!("the launcher's index of mail is off: {why}");
    }
}

/// Push what `read` reads to the router on `connection`, now and whenever [`Changed::said`] or
/// the router comes onto the bus, at most once every `every`. `read` is asked only while the router
/// is there; `None` (the store could not be read) skips that push.
pub fn push_on<R>(connection: Connection, read: R, every: Duration) -> zbus::Result<Changed>
where
    R: FnMut() -> Option<Findables> + Send + 'static,
{
    let (nudge, nudges) = channel();
    // Listened for before this returns, so a router that comes after it is heard.
    let came = DBusProxy::new(&connection)?.receive_name_owner_changed_with_args(&[(0, ROUTER)])?;
    let router = nudge.clone();
    std::thread::Builder::new()
        .name("mailo-index-router".to_owned())
        .spawn(move || {
            for change in came {
                let here = change.args().is_ok_and(|args| args.new_owner().is_some());
                if here && router.send(Nudge::RouterCame).is_err() {
                    return;
                }
            }
        })
        .map_err(|why| zbus::Error::Failure(why.to_string()))?;
    std::thread::Builder::new()
        .name("mailo-index-push".to_owned())
        .spawn(move || run(&connection, read, &nudges, every))
        .map_err(|why| zbus::Error::Failure(why.to_string()))?;
    Ok(Changed(nudge))
}

/// Push now, then once for each burst of nudges, for as long as anyone can nudge.
fn run(
    connection: &Connection,
    mut read: impl FnMut() -> Option<Findables>,
    nudges: &Receiver<Nudge>,
    every: Duration,
) {
    let mut pusher = Pusher::new(connection);
    pusher.push(&mut read);
    let mut pushed = Instant::now();
    while let Ok(first) = nudges.recv() {
        std::thread::sleep(every.saturating_sub(pushed.elapsed()));
        let came = std::iter::once(first)
            .chain(nudges.try_iter())
            .any(|nudge| nudge == Nudge::RouterCame);
        if came {
            pusher.forget();
        }
        pusher.push(&mut read);
        pushed = Instant::now();
    }
}

/// Why a push did not go through.
enum Fault {
    /// The router has another epoch: reset, and push everything.
    Refused,
    /// The bus, or the router, failed otherwise.
    Bus(zbus::Error),
}

impl From<zbus::Error> for Fault {
    fn from(why: zbus::Error) -> Self {
        match &why {
            zbus::Error::MethodError(name, _, _) if name.as_str() == MALFORMED => Fault::Refused,
            _ => Fault::Bus(why),
        }
    }
}

/// What the router has been told, and in which epoch.
struct Pusher<'c> {
    connection: &'c Connection,
    /// The epoch the router holds what was pushed in; `None` when the next push must reset.
    epoch: Option<u64>,
    /// The last epoch started, so the next is another.
    started: u64,
    /// What the router holds.
    pushed: Findables,
}

impl<'c> Pusher<'c> {
    fn new(connection: &'c Connection) -> Self {
        Pusher {
            connection,
            epoch: None,
            started: 0,
            pushed: Findables::new(),
        }
    }

    /// The next push starts over.
    fn forget(&mut self) {
        self.epoch = None;
        self.pushed.clear();
    }

    /// Whether the router is on the bus. Asked first, so a router that is not running is not
    /// started by a call, and the store is not read for nobody.
    fn router_here(&self) -> bool {
        let here = DBusProxy::new(self.connection)
            .and_then(|bus| Ok(bus.name_has_owner(ROUTER.try_into()?)?));
        matches!(here, Ok(true))
    }

    /// Push what moved since the last push; everything, after a reset, when the router has
    /// another epoch or none.
    fn push(&mut self, read: &mut impl FnMut() -> Option<Findables>) {
        if !self.router_here() {
            log::debug!("no intent router on the bus: the launcher's index of mail waits for it");
            self.forget();
            return;
        }
        let Some(now) = read() else {
            return;
        };
        // A refusal once is a router that restarted or forgot mail; twice in a row is not one a
        // reset mends, and waits for the next change.
        for _ in 0..2 {
            match self.send(&now) {
                Ok(()) => {
                    self.pushed = now;
                    return;
                }
                Err(Fault::Refused) => self.forget(),
                Err(Fault::Bus(why)) => {
                    log::debug!("the launcher's index of mail was not pushed: {why}");
                    self.forget();
                    return;
                }
            }
        }
    }

    /// Send what `now` says that the router does not hold, resetting first when it must.
    fn send(&mut self, now: &Findables) -> Result<(), Fault> {
        let epoch = match self.epoch {
            Some(epoch) => epoch,
            None => {
                let epoch = fresh(self.started);
                self.started = epoch;
                self.call("Reset", &(epoch,))?;
                self.pushed.clear();
                self.epoch = Some(epoch);
                epoch
            }
        };
        for batch in batches(epoch, findable::changes(&self.pushed, now)) {
            let text = serde_json::to_string(&batch)
                .map_err(|why| Fault::Bus(zbus::Error::Failure(why.to_string())))?;
            self.call("Push", &(text,))?;
        }
        Ok(())
    }

    fn call<B>(&self, member: &str, body: &B) -> Result<(), Fault>
    where
        B: serde::Serialize + zbus::zvariant::DynamicType,
    {
        self.connection
            .call_method(Some(ROUTER), PATH, Some(INTERFACE), member, body)?;
        Ok(())
    }
}

/// An epoch after `last`: the time in nanoseconds, which another run of the window will not
/// have used either.
fn fresh(last: u64) -> u64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| {
            u64::try_from(since.as_nanos()).unwrap_or(u64::MAX)
        });
    now.max(last.saturating_add(1))
}

/// `changes` in `epoch`, at most [`BATCH`] entries to a push: the removals first, then the
/// upserts. Nothing to push is no batch.
fn batches(epoch: u64, changes: Changes) -> Vec<IndexBatch> {
    let empty = || IndexBatch {
        epoch,
        upserts: Vec::new(),
        removals: Vec::new(),
    };
    let mut all = Vec::new();
    let mut batch = empty();
    let mut room = |batch: &mut IndexBatch| {
        if batch.upserts.len() + batch.removals.len() == BATCH {
            all.push(std::mem::replace(batch, empty()));
        }
    };
    for (_, key) in changes.removals {
        room(&mut batch);
        batch.removals.push(key);
    }
    for found in &changes.upserts {
        room(&mut batch);
        batch.upserts.push(IndexEntry::of(found));
    }
    if !batch.upserts.is_empty() || !batch.removals.is_empty() {
        all.push(batch);
    }
    all
}

#[cfg(test)]
mod tests;
