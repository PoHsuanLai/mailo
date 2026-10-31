//! Talking to the daemon, and starting one when there is none.
//!
//! Finding it is [`latchkey`]'s; this is what is said once the door opens.

use super::wire::{self, Request, Response};
use mail_core::CoreError;
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
    pub fn ask(&mut self, request: Request) -> Result<Response, CoreError> {
        let text = wire::line(request)?;
        self.stream
            .write_all(text.as_bytes())
            .map_err(|e| CoreError::cannot("reach the daemon", e))?;
        self.stream
            .flush()
            .map_err(|e| CoreError::cannot("reach the daemon", e))?;

        read(&mut BufReader::new(&mut self.stream))
    }
}

/// Read one answer, and say what a failure to was in a person's words.
fn read(reader: &mut impl BufRead) -> Result<Response, CoreError> {
    let mut reply = String::new();
    reader
        .read_line(&mut reply)
        .map_err(|e| CoreError::context("the daemon stopped mid-answer", e))?;
    if reply.is_empty() {
        return Err(CoreError::DaemonSilent);
    }
    match wire::parse::<Response>(&reply) {
        Ok(response) => Ok(response),
        // Reported rather than swallowed: a version mismatch is the one failure here with a
        // specific remedy, and it is the expected state after an upgrade.
        Err(mismatch) => Err(mismatch.into()),
    }
}

/// Connect to the running daemon, if there is one.
///
/// `Ok(None)` means nothing is listening — not an error, because the usual answer to it is to
/// start one.
pub fn connect() -> Result<Option<Daemon>, CoreError> {
    connect_at(&super::agent()?)
}

/// Connect to whatever is listening at `agent`: [`connect`], for the watch's door or a test's.
pub fn connect_at(agent: &latchkey::Agent) -> Result<Option<Daemon>, CoreError> {
    Ok(agent.connect()?.map(|stream| Daemon { stream }))
}

/// Connect to a daemon, starting one if needed.
///
/// `mailo daemon` is the subcommand a client launches, and it is this same binary: the daemon a
/// client starts must be the build the client came from, or an upgrade leaves a new CLI talking
/// to whatever old binary happened to be installed first.
pub fn reach() -> Result<Daemon, CoreError> {
    Ok(super::agent()?
        .connect_or_start(|| latchkey::spawn(&["daemon"]), STARTUP)
        .map(|stream| Daemon { stream })?)
}
