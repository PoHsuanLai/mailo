//! The door both daemons keep: one request, one answer, and the connection closes.
//!
//! Nothing that comes through it stays. A window learns that another process stored mail from
//! the store's file, which the system says has changed (`mail-app`'s `fetching::external`), not
//! from a connection held open here; so neither daemon keeps any state about who is listening,
//! and a client that goes away leaves nothing behind. (Until 2026-10 a window subscribed here and
//! was told as each pass ended, through a connection and a thread per window. A watch whose
//! passes all failed told nobody, never found that its windows had gone, and ran out of file
//! descriptors.)

use super::wire::{self, Mismatch, Request, Response};
use crate::error::CoreError;
use interprocess::local_socket::traits::Stream as _;
use std::io::{BufRead, BufReader, Read, Write};
use std::time::Duration;

/// How long the door waits after it could not accept a client before it tries again.
///
/// An accept fails at once, again and again, for as long as its cause lasts (the process out of
/// file descriptors is the one that has happened), and a door that retried at once would spin a
/// core and write the same line to the journal millions of times. A client waits this long more,
/// once.
const AFTER_A_FAILED_ACCEPT: Duration = Duration::from_secs(1);

/// How long a client has to say its request. The door serves one client at a time, so one that
/// connects and says nothing would otherwise hold it shut for everyone after.
const REQUEST_WAIT: Duration = Duration::from_secs(5);

/// The longest request line read. Every request is a few hundred bytes; a client sending more
/// is not one this build wrote, and reading on would grow the daemon's memory for it.
const MAX_REQUEST: usize = 64 * 1024;

/// What a daemon does with a request it has read.
pub(crate) enum Answer {
    /// Say this, and go on to the next client.
    Say(Response),
    /// Say this, and stop serving.
    Last(Response),
}

/// Serve clients on `listening` until a request is answered with [`Answer::Last`].
///
/// One connection at a time, deliberately. Every request here either answers immediately or
/// hands work to a thread, so a slow client cannot hold the door; concurrency belongs in the
/// sync engine, which already has it, rather than in the doorman.
pub(crate) fn door(listening: &latchkey::Listening, mut on: impl FnMut(Request) -> Answer) {
    // Said once when accepting starts to fail, and once when it works again: what changed, not
    // every attempt.
    let mut failing = false;
    for connection in listening.incoming() {
        let mut stream = match connection {
            Ok(stream) => {
                if std::mem::take(&mut failing) {
                    log::info!("clients are accepted again");
                }
                stream
            }
            Err(e) => {
                if !std::mem::replace(&mut failing, true) {
                    log::warn!(
                        "a client could not be accepted, and the door will try again each second: {e}"
                    );
                }
                std::thread::sleep(AFTER_A_FAILED_ACCEPT);
                continue;
            }
        };
        // Where the system has no receive timeout the read waits as it did; the cap still holds.
        let _ = stream.set_recv_timeout(Some(REQUEST_WAIT));
        let Some(line) = request_line(&mut stream) else {
            continue;
        };
        let reply = match wire::parse::<Request>(&line) {
            Ok(request) => on(request),
            // Answered rather than dropped: a client from a different build needs to be told
            // which way round the mismatch is, and silence would look like a dead daemon.
            Err(Mismatch::Version { theirs, ours }) => Answer::Say(Response::WrongVersion {
                daemon: ours,
                client: theirs,
            }),
            // A request this build does not know, such as the `subscribe` a window from before
            // 2026-10 still sends, is refused, and that window goes on looking at the store.
            Err(other) => Answer::Say(Response::Refused(other.to_string())),
        };
        match reply {
            Answer::Say(reply) => {
                // A client gone before its answer reaches it has nobody to tell.
                let _ = say(&mut stream, reply);
            }
            Answer::Last(reply) => {
                // A client gone before its answer reaches it has nobody to tell.
                let _ = say(&mut stream, reply);
                return;
            }
        }
    }
}

/// The first line `stream` sends, ending in a newline and at most [`MAX_REQUEST`] bytes, or
/// `None` for one that does not arrive, is longer, or stops short of its newline.
fn request_line(stream: impl Read) -> Option<String> {
    let mut line = String::new();
    let mut reader = BufReader::new(stream.take(MAX_REQUEST as u64));
    match reader.read_line(&mut line) {
        Ok(_) if line.ends_with('\n') => Some(line),
        _ => None,
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
pub(crate) fn say(stream: &mut latchkey::Stream, response: Response) -> Result<(), CoreError> {
    let text = wire::line(response)?;
    stream.write_all(text.as_bytes())?;
    Ok(stream.flush()?)
}

#[cfg(test)]
mod tests {
    use super::{MAX_REQUEST, request_line};
    use std::io::Cursor;

    #[test]
    fn a_request_is_one_whole_line_no_longer_than_the_cap() {
        let fits = format!("{}\n", "x".repeat(MAX_REQUEST - 1));
        let over = format!("{}\n", "x".repeat(MAX_REQUEST));
        let cases: [(&str, &[u8], bool); 4] = [
            ("a line", b"{\"ping\":{}}\nmore", true),
            ("at the cap", fits.as_bytes(), true),
            ("past the cap", over.as_bytes(), false),
            ("never ended", b"{\"ping\"", false),
        ];
        for (name, bytes, read) in cases {
            assert_eq!(request_line(Cursor::new(bytes)).is_some(), read, "{name}");
        }
    }
}
