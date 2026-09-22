//! The agent end: take the lock, open the door, and put both away together.

use crate::address::{Address, Endpoint};
use crate::{Error, lock};
use interprocess::local_socket::{GenericFilePath, GenericNamespaced, ListenerOptions, prelude::*};

/// What a client's connection looks like from inside the agent.
///
/// One type on every platform — a Unix domain socket here, a named pipe there — because a caller
/// that has to know which one it got is a caller writing the code this crate exists to delete.
pub type Stream = interprocess::local_socket::Stream;

/// Listening, and the proof that we are the only one doing so.
///
/// The listener and the lock are one value because their lifetimes are one fact: while this
/// exists, this user has exactly one agent, and when it goes the door goes with it. Nothing else
/// in the program has to remember to clean up, which is the difference between a rule that holds
/// and a rule that holds on the paths someone thought about.
#[derive(Debug)]
pub struct Listening {
    listener: interprocess::local_socket::Listener,
    /// The socket to unlink on the way out. `None` on Windows, where a pipe is not a file and
    /// vanishes with the process that made it.
    socket: Option<std::path::PathBuf>,
    /// Never read. Held because dropping it releases the lock.
    _lock: lock::Held,
}

impl Listening {
    /// Wait for the next client.
    pub fn accept(&self) -> std::io::Result<Stream> {
        self.listener.accept()
    }

    /// Every client, forever, as an iterator.
    pub fn incoming(&self) -> impl Iterator<Item = std::io::Result<Stream>> + '_ {
        std::iter::repeat_with(move || self.accept())
    }

    /// Where this agent is listening, for printing or passing to a child.
    pub fn address(&self) -> Option<&std::path::Path> {
        self.socket.as_deref()
    }
}

impl Drop for Listening {
    /// Unlink first, release second.
    ///
    /// The lock is a field, so it is dropped *after* this body runs. An agent waiting on the
    /// lock therefore never observes the moment where the lock is free and the old socket is
    /// still on disk — which is the moment it would bind over.
    fn drop(&mut self) {
        if let Some(socket) = &self.socket {
            let _ = std::fs::remove_file(socket);
        }
    }
}

/// Become this user's agent, or report that one already is.
///
/// Returns [`Error::AlreadyRunning`] if another process holds the lock. That is the answer to
/// "should I start?", and it is a kernel fact rather than a guess about a file — see
/// [`crate::lock`] for the two races that reading the filesystem instead would open.
pub fn listen(at: &Address) -> Result<Listening, Error> {
    let held = lock::take(&at.lock)?;

    let socket = match &at.endpoint {
        Endpoint::Socket(path) => {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| Error::at(parent, e))?;
            }
            // Past the lock, so a socket still here was left by a process that is gone: no live
            // agent could be behind it, because a live agent would still hold the lock. This is
            // the *only* place in the crate that removes a socket, and it does so knowing rather
            // than inferring.
            if path.exists() {
                std::fs::remove_file(path).map_err(|e| Error::at(path, e))?;
            }
            Some(path.clone())
        }
        Endpoint::Pipe(_) => None,
    };

    let listener = options(&at.endpoint)?
        .create_sync()
        .map_err(|e| Error::Listen {
            endpoint: at.endpoint.clone(),
            cause: e,
        })?;

    if let Some(path) = &socket {
        lock::owner_only(path);
    }

    Ok(Listening {
        listener,
        socket,
        _lock: held,
    })
}

/// The listener's settings, which are mostly one setting.
fn options(endpoint: &Endpoint) -> Result<ListenerOptions<'_>, Error> {
    let options = ListenerOptions::new();
    // `reclaim_name` is `interprocess`'s own answer to the stale socket: delete it and carry on.
    // Turned off, because it is the racy answer — it cannot tell a dead agent's socket from a
    // live one's, and deleting the wrong one is silent. The lock above already decided, and by
    // the time we get here the socket has been cleared under it.
    let options = options.reclaim_name(false);
    Ok(match endpoint {
        Endpoint::Socket(path) => options.name(
            path.as_os_str()
                .to_fs_name::<GenericFilePath>()
                .map_err(|e| Error::at(path, e))?,
        ),
        Endpoint::Pipe(name) => options.name(
            name.as_str()
                .to_ns_name::<GenericNamespaced>()
                .map_err(|e| Error::Listen {
                    endpoint: endpoint.clone(),
                    cause: e,
                })?,
        ),
    })
}
