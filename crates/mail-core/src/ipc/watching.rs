//! One `mailo watch` per user, and a way for everything else to tell whether it is running.
//!
//! `mailo watch` is the session's sync daemon (`dist/mailo-watch.service`, started at login), and
//! it announces new mail and shows the unread count on the launcher. A second one — a person
//! typing `mailo watch` in a terminal while the unit runs — would announce every message twice,
//! so it is refused. The window asks the same question to decide whether it should show the
//! launcher count itself: with a watch running, the watch is the one voice, and it keeps the count
//! when the window closes (a dock forgets a count when its sender leaves the bus).
//!
//! The answer is [`latchkey`]'s advisory lock, a kernel fact that dies with the process, the same
//! as the daemon's own (`ipc::agent`), under its own name.
//!
//! # A door for the window
//!
//! The watch is the process that stores pushed mail while a window is open, and the one whose
//! passes a window's Sync must reach. It listens behind its lock, as the daemon does behind its
//! own, and answers [`Request::Ping`]. A window learns what the watch stored from the store's
//! file, not from here (`super::door`). [`Request::SyncNow`] is a person asking for their mail now,
//! and is answered with a pass of every account on the watch's own scheduler
//! ([`Watching::asked`]): a request that is refused, or heard and dropped, is a Sync button that
//! does nothing. [`Request::Shutdown`] is refused: a watch is stopped by its unit or by Ctrl-C.

use super::door::{self, Answer};
use super::wire::{Request, Response};
use crate::error::CoreError;
use std::sync::Arc;
use tokio::sync::watch;

/// What a watch's scheduler listens on for a pass someone asked for at the door: it moves once
/// for each [`Request::SyncNow`].
pub type Asked = watch::Receiver<u64>;

/// The name the watch's lock and door live under, beside the daemon's.
const NAME: &str = "mailo-watch";

/// Proof that this process is the one watching.
///
/// The lock is held for as long as the process runs, whatever becomes of the door: the process
/// ending is what lets the next one in.
#[derive(Debug)]
pub struct Watching {
    /// Why there is no door, when it could not be opened.
    doorless: Option<String>,
    /// Moved by the door each time a pass is asked for.
    asks: Arc<watch::Sender<u64>>,
}

impl Watching {
    /// Why this watch has no door, if it has none: it watches all the same, and a window looks
    /// at the store for what it stores rather than being told.
    pub fn doorless(&self) -> Option<&str> {
        self.doorless.as_deref()
    }

    /// The passes asked for at the door, for the watch's scheduler to start
    /// (`SyncOps::watch`). Without a door it never moves.
    pub fn asked(&self) -> Asked {
        self.asks.subscribe()
    }
}

/// Why this process may not watch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refused {
    /// Another `mailo watch` holds the lock.
    AlreadyWatching,
    /// The lock could not be taken, or the door opened, for another reason.
    Failed(String),
}

/// This user's watch, wherever this platform puts such a thing: where its lock is, and the door
/// the window knocks on.
pub fn agent() -> Result<latchkey::Agent, CoreError> {
    latchkey::Agent::new(NAME).map_err(CoreError::from)
}

/// Become this user's watch, or say why not.
pub fn claim() -> Result<Watching, Refused> {
    claim_at(&agent().map_err(|e| Refused::Failed(e.to_string()))?)
}

/// Become the watch at `agent`'s address: [`claim`], for a test that keeps its own.
///
/// The door is a help to a window, not a condition of watching: one that cannot be opened (a
/// socket refused, a pipe taken) leaves the watch holding its lock alone, with the reason in
/// [`Watching::doorless`], rather than ending it — under its unit that would be a restart loop.
pub fn claim_at(agent: &latchkey::Agent) -> Result<Watching, Refused> {
    let asks = Arc::new(watch::channel(0u64).0);
    let listening = match agent.listen() {
        Ok(listening) => listening,
        Err(latchkey::Error::AlreadyRunning) => return Err(Refused::AlreadyWatching),
        Err(door) => {
            let held = latchkey::lock::take(&agent.address().lock).map_err(|e| match e {
                latchkey::Error::AlreadyRunning => Refused::AlreadyWatching,
                other => Refused::Failed(other.to_string()),
            })?;
            // Held until the process ends, as a door's would be.
            Box::leak(Box::new(held));
            return Ok(Watching {
                doorless: Some(door.to_string()),
                asks,
            });
        }
    };
    // Leaked, so that the lock inside it lives as long as the process even if the thread serving
    // the door ends early: a door that stopped answering must not let a second watch in.
    let listening: &'static latchkey::Listening = Box::leak(Box::new(listening));
    let asking = asks.clone();
    std::thread::Builder::new()
        .name("mailo-watch-door".to_owned())
        .spawn(move || {
            door::door(listening, |request| match request {
                Request::Ping => Answer::Say(door::pong()),
                // Started, not finished: the pass runs on the scheduler, as the daemon's does on
                // its thread, and the answer goes back before it ends.
                Request::SyncNow => {
                    asking.send_modify(|asked| *asked = asked.wrapping_add(1));
                    Answer::Say(Response::Started)
                }
                Request::Shutdown => Answer::Say(Response::Refused(
                    "a watch is stopped where it was started, not through its door".to_owned(),
                )),
            });
        })
        .map_err(|e| Refused::Failed(e.to_string()))?;
    Ok(Watching {
        doorless: None,
        asks,
    })
}

/// Whether a watch is running now, for this user.
pub fn running() -> bool {
    agent().is_ok_and(|agent| agent.is_running())
}

#[cfg(test)]
mod tests {
    // One test, because the lock is per user and tests share a process: a second test claiming it
    // at the same moment would see the first's. The environment is the scratch one's, never the
    // person's runtime directory.
    #[test]
    fn one_watch_per_user_and_the_lock_goes_with_it() {
        let scratch = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        // The address this platform would give the watch in a directory of its own (a socket there,
        // or on Windows a named pipe with its lock there), asked for rather than reached by
        // changing this process's environment. The directory's name keeps two runs' pipes apart.
        let unique = scratch
            .path()
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("test");
        let environment = latchkey::Environment {
            runtime_dir: Some(scratch.path().as_os_str().to_owned()),
            tmpdir: Some(scratch.path().as_os_str().to_owned()),
            home: None,
            local_app_data: Some(scratch.path().as_os_str().to_owned()),
            user: Some(unique.to_owned()),
        };
        let agent =
            latchkey::Agent::in_environment("mailo-watch-test", latchkey::here(), &environment)
                .unwrap_or_else(|e| panic!("{e}"));
        assert!(!agent.is_running());
        // Running means answering a knock (latchkey 99cc9de), so the watch listens, as the real
        // one does: holding the lock alone is not enough.
        let first = agent.listen().unwrap_or_else(|e| panic!("{e}"));
        assert!(agent.is_running(), "listening, so running");
        assert!(matches!(
            latchkey::lock::take(&agent.address().lock),
            Err(latchkey::Error::AlreadyRunning)
        ));
        drop(first);
        assert!(!agent.is_running(), "the lock went with its holder");
    }
}
