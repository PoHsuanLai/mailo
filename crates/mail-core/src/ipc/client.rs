//! Talking to the daemon, and starting one when there is none.
//!
//! Finding it is [`latchkey`]'s; this is what is said once the door opens.

use super::wire::{self, Mismatch, Request, Response};
use std::io::{BufRead, BufReader, Write};
use std::time::Duration;

/// How long to wait for a daemon we just started to answer.
///
/// It takes its lock and opens its door before it does anything else, so this is process
/// startup, not a sync — which is what lets the number stay this small.
const STARTUP: Duration = Duration::from_secs(5);

/// A connection to the daemon.
#[derive(Debug)]
pub struct Daemon {
    stream: latchkey::Stream,
}

impl Daemon {
    /// Ask one question and read the answer.
    pub fn ask(&mut self, request: Request) -> Result<Response, String> {
        let text = wire::line(request)?;
        self.stream
            .write_all(text.as_bytes())
            .map_err(|e| format!("cannot reach the daemon: {e}"))?;
        self.stream
            .flush()
            .map_err(|e| format!("cannot reach the daemon: {e}"))?;

        let mut reply = String::new();
        BufReader::new(&mut self.stream)
            .read_line(&mut reply)
            .map_err(|e| format!("the daemon stopped mid-answer: {e}"))?;
        if reply.is_empty() {
            return Err("the daemon closed the connection without answering".to_owned());
        }
        match wire::parse::<Response>(&reply) {
            Ok(response) => Ok(response),
            // Reported rather than swallowed: a version mismatch is the one failure here with a
            // specific remedy, and it is the expected state after an upgrade.
            Err(Mismatch::Version { theirs, ours }) => {
                Err(Mismatch::Version { theirs, ours }.to_string())
            }
            Err(other) => Err(other.to_string()),
        }
    }
}

/// Connect to the running daemon, if there is one.
///
/// `Ok(None)` means nothing is listening — not an error, because the usual answer to it is to
/// start one.
pub fn connect() -> Result<Option<Daemon>, String> {
    super::agent()?
        .connect()
        .map(|reached| reached.map(|stream| Daemon { stream }))
        .map_err(|e| e.to_string())
}

/// Connect to a daemon, starting one if needed.
///
/// `mailo daemon` is the subcommand a client launches, and it is this same binary: the daemon a
/// client starts must be the build the client came from, or an upgrade leaves a new CLI talking
/// to whatever old binary happened to be installed first.
pub fn reach() -> Result<Daemon, String> {
    super::agent()?
        .connect_or_start(|| latchkey::spawn(&["daemon"]), STARTUP)
        .map(|stream| Daemon { stream })
        .map_err(|e| e.to_string())
}
