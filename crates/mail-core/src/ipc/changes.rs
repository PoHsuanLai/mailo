//! Telling whoever asked that a pass has stored something, and the door both daemons keep.
//!
//! The window reads the same SQLite file the daemon writes, and until it is told it can only find
//! out by looking (`SqliteStore::data_version`, every couple of seconds). A [`Request::Subscribe`]
//! turns that into being told: the connection stays open and the daemon writes one
//! [`Response::Changed`] line on it after each pass that may have stored something.
//!
//! # One thread per listener
//!
//! The pass that has just stored mail is the one that tells, so whatever telling costs is paid by
//! the sync engine. A write to a socket blocks once the reader stops reading and its buffer
//! fills, and a window that froze, or was stopped in a debugger, would then stall every account's
//! next pass behind it. So [`Subscribers::tell`] only puts the news on a channel, which never
//! blocks, and each listener has a thread of its own that does the writing. A listener that has
//! gone is found the next time its thread writes and fails, and its channel is dropped with it.

use super::wire::{self, Mismatch, Request, Response};
use porter_core::AccountId;
use std::io::{BufRead, BufReader, Write};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex, PoisonError};

/// Everyone listening for [`Response::Changed`] on one daemon.
#[derive(Debug, Clone, Default)]
pub struct Subscribers {
    listening: Arc<Mutex<Vec<Sender<AccountId>>>>,
}

impl Subscribers {
    /// Answer a [`Request::Subscribe`] on `stream`, and keep telling it until it hangs up.
    pub(crate) fn add(&self, stream: latchkey::Stream) {
        let (tx, rx) = channel::<AccountId>();
        // On the list before the listener is answered, so a pass that ends the moment it has its
        // answer is one it hears of.
        self.held().push(tx);
        // A thread that could not be started is a listener that is not told: its channel goes
        // with the closure, the next tell finds it gone, and the listener sees its connection
        // close, which is what it is told when the daemon goes, and looks for itself.
        let _ = std::thread::Builder::new()
            .name("mailo-subscriber".to_owned())
            .spawn(move || {
                let mut stream = stream;
                if say(&mut stream, Response::Subscribed).is_err() {
                    return;
                }
                // Ends when the write fails (the listener left) or the sender is dropped.
                while let Ok(account) = rx.recv() {
                    if say(&mut stream, Response::Changed { account }).is_err() {
                        return;
                    }
                }
            });
    }

    /// A pass on `account` has ended and may have stored something. Never blocks.
    pub fn tell(&self, account: AccountId) {
        self.held().retain(|tx| tx.send(account.clone()).is_ok());
    }

    /// How many are listening, as far as the last [`Subscribers::tell`] could tell: one that
    /// left since is still counted.
    pub fn count(&self) -> usize {
        self.held().len()
    }

    fn held(&self) -> std::sync::MutexGuard<'_, Vec<Sender<AccountId>>> {
        // Nothing done under the lock can leave the list half-changed, so a panic elsewhere while
        // it was held is no reason to stop telling anyone.
        self.listening
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

/// What a daemon does with a request it has read.
pub(crate) enum Answer {
    /// Say this, and go on to the next client.
    Say(Response),
    /// Hand the connection to the [`Subscribers`], which answer it.
    Subscribe,
    /// Say this, and stop serving.
    Last(Response),
}

/// Serve clients on `listening` until a request is answered with [`Answer::Last`].
///
/// One connection at a time, deliberately. Every request here either answers immediately or
/// hands work to a thread, so a slow client cannot hold the door; concurrency belongs in the
/// sync engine, which already has it, rather than in the doorman. A subscription is handed to a
/// thread too, which is why one listener that never hangs up does not keep the door shut.
pub(crate) fn door(
    listening: &latchkey::Listening,
    subscribers: &Subscribers,
    mut on: impl FnMut(Request) -> Answer,
) {
    for connection in listening.incoming() {
        let mut stream = match connection {
            Ok(stream) => stream,
            Err(e) => {
                eprintln!("a client could not be accepted: {e}");
                continue;
            }
        };
        let mut line = String::new();
        if BufReader::new(&mut stream).read_line(&mut line).is_err() || line.is_empty() {
            continue;
        }
        let request = match wire::parse::<Request>(&line) {
            Ok(request) => request,
            Err(Mismatch::Version { theirs, ours }) => {
                // Answered rather than dropped: a client from a different build needs to be told
                // which way round the mismatch is, and silence would look like a dead daemon.
                let _ = say(
                    &mut stream,
                    Response::WrongVersion {
                        daemon: ours,
                        client: theirs,
                    },
                );
                continue;
            }
            Err(other) => {
                let _ = say(&mut stream, Response::Refused(other.to_string()));
                continue;
            }
        };
        match on(request) {
            Answer::Say(reply) => {
                let _ = say(&mut stream, reply);
            }
            Answer::Subscribe => subscribers.add(stream),
            Answer::Last(reply) => {
                let _ = say(&mut stream, reply);
                return;
            }
        }
    }
}

/// The answer to [`Request::Ping`], the same from either daemon.
pub(crate) fn pong() -> Response {
    Response::Pong {
        pid: std::process::id(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
    }
}

/// Write one message on `stream`.
pub(crate) fn say(stream: &mut latchkey::Stream, response: Response) -> Result<(), String> {
    let text = wire::line(response)?;
    stream
        .write_all(text.as_bytes())
        .map_err(|e| e.to_string())?;
    stream.flush().map_err(|e| e.to_string())
}
