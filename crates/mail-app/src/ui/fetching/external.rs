//! The window hearing about what it did not write.
//!
//! A `mailo watch` stores the mail it fetches. It is another process, and the window's lists are
//! read again only when its revision moves. So the window looks at whether the store has been
//! committed to by another connection ([`SqliteStore::data_version`]), and moves the revision
//! when it has.
//!
//! # Told, and looking
//!
//! Looking every [`LOOK`] puts up to that long between the watch storing a message and the
//! window showing it. Where a watch (or the daemon) is running, the window subscribes at its door
//! (`mail_core::ipc::changes`) and is told as each pass ends, and it looks then. What it does on
//! being told is the same look: the data version, not the message, decides whether anything
//! moved, so a pass that stored nothing moves nothing and being told and looking never count one
//! commit twice.
//!
//! The looking goes on while subscribed to the watch, every [`TOLD`] rather than every
//! [`LOOK`], for what no pass says: `mailo` on the command line, or a daemon that stored
//! something and then failed to write the line. Subscribed to the daemon it stays at [`LOOK`]:
//! the daemon is not what stores pushed mail, and a watch started after it would otherwise go
//! unheard for a [`TOLD`]. When the subscription ends — the watch stopped, or was restarted into
//! a new build — the window is back to looking every [`LOOK`] at once, and knocks again every
//! [`KNOCK`] until something answers.
//!
//! Told while the writer is busy, the look cannot read the data version; it looks again every
//! [`OWED`] until it can, rather than leaving what it was told about to the next [`TOLD`].

use dioxus::prelude::*;
use mail_core::SqliteStore;
use mail_core::ipc::client::Changes;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

/// How often the window looks when nothing will tell it.
#[cfg(not(test))]
const LOOK: Duration = Duration::from_secs(2);
#[cfg(test)]
pub(super) const LOOK: Duration = Duration::from_millis(30);

/// How often the window looks while a daemon tells it of each pass.
#[cfg(not(test))]
const TOLD: Duration = Duration::from_secs(15);
/// Long enough in a test that a change seen within a few looks was told, not looked for.
#[cfg(test)]
pub(super) const TOLD: Duration = Duration::from_secs(60);

/// How soon to look again after being told while the writer was busy.
const OWED: Duration = Duration::from_millis(50);

/// How long after finding no door, or losing one, the window knocks again.
#[cfg(not(test))]
const KNOCK: Duration = Duration::from_secs(10);
#[cfg(test)]
pub(super) const KNOCK: Duration = Duration::from_millis(30);

/// Where the window subscribes: the doors to knock on, in order, the first that answers kept,
/// with how often to look while subscribed there. The real ones unless a test provided its own,
/// as [`super::Passer`] is.
#[derive(Clone)]
pub(in crate::ui) struct Doors(pub Arc<dyn Fn() -> Option<(Changes, Duration)> + Send + Sync>);

impl Doors {
    /// The watch's door, then the daemon's. The watch is the one that holds the connections and
    /// stores pushed mail; the daemon stores only what it is asked to sync.
    #[cfg(not(test))]
    pub(in crate::ui) fn server() -> Self {
        Self(Arc::new(|| {
            [
                (mail_core::ipc::watching::agent(), TOLD),
                (mail_core::ipc::agent(), LOOK),
            ]
            .into_iter()
            .filter_map(|(agent, every)| Some((agent.ok()?, every)))
            // A daemon from another build answers with an error. Not said: the window cannot
            // fix it and looks instead, and `mailo ping` says it to whoever can.
            .find_map(|(agent, every)| {
                let changes = mail_core::ipc::client::subscribe(&agent).ok().flatten()?;
                Some((changes, every))
            })
        }))
    }

    /// Not in a test build: the person's real watch, if there is one, must not move what a test
    /// sees.
    #[cfg(test)]
    pub(in crate::ui) fn server() -> Self {
        Self(Arc::new(|| None))
    }
}

/// What the subscribing thread tells the looking loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Heard {
    /// A door answered: the passes will be told, so looking can slow down to this.
    Subscribed(Duration),
    /// A pass ended that may have stored something.
    Changed,
    /// The subscription ended: back to looking often.
    Dropped,
}

/// Move `revision` whenever another connection has committed to `store`. Once, from
/// [`super::use_fetching`].
pub(super) fn use_external_changes(
    mut revision: Signal<u64>,
    store: Arc<SqliteStore>,
    doors: Doors,
) {
    let _looking = use_future(move || {
        let store = store.clone();
        let doors = doors.clone();
        async move {
            let mut heard = subscribe(doors);
            let mut seen = store.data_version();
            let mut every = LOOK;
            let mut owed = false;
            loop {
                let wait = if owed { OWED } else { every };
                let said = next(&mut heard, wait).await;
                match said {
                    Some(Heard::Subscribed(pace)) => every = pace,
                    Some(Heard::Dropped) => every = LOOK,
                    Some(Heard::Changed) | None => {}
                }
                let now = store.data_version();
                // A busy writer answers nothing this time. Told of a pass, look again soon
                // rather than at the next look; otherwise the next look will do.
                owed = now.is_none() && (owed || said == Some(Heard::Changed));
                if now.is_some() && now != seen {
                    seen = now;
                    revision += 1;
                }
            }
        }
    });
}

/// What the thread says next, or `None` when `every` passes first.
async fn next(heard: &mut Option<UnboundedReceiver<Heard>>, every: Duration) -> Option<Heard> {
    let Some(rx) = heard else {
        tokio::time::sleep(every).await;
        return None;
    };
    match tokio::time::timeout(every, rx.recv()).await {
        Ok(Some(said)) => Some(said),
        Err(_) => None,
        // The thread could not be started, or has ended: nothing will be said again, so the
        // rest is looking, as often as with no door.
        Ok(None) => {
            *heard = None;
            Some(Heard::Dropped)
        }
    }
}

/// Knock on `doors` on a thread of its own, and keep knocking, for as long as the window listens.
///
/// A thread rather than a task: a subscription is a blocking read that can last the whole
/// session, and the window's runtime should not lend a worker to it.
fn subscribe(doors: Doors) -> Option<UnboundedReceiver<Heard>> {
    let (tx, rx) = unbounded_channel();
    std::thread::Builder::new()
        .name("mailo-changes".to_owned())
        .spawn(move || knock(&doors, &tx))
        .ok()?;
    Some(rx)
}

/// Subscribe, listen until the subscription ends, and again, until the window stops listening.
fn knock(doors: &Doors, tx: &UnboundedSender<Heard>) {
    while !tx.is_closed() {
        if let Some((mut changes, every)) = (doors.0)() {
            if tx.send(Heard::Subscribed(every)).is_err() {
                return;
            }
            while changes.wait().is_ok() {
                if tx.send(Heard::Changed).is_err() {
                    return;
                }
            }
            if tx.send(Heard::Dropped).is_err() {
                return;
            }
        }
        std::thread::sleep(KNOCK);
    }
}

#[cfg(test)]
mod tests;
