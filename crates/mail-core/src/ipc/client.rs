//! Talking to the daemon, and starting one when there is none.
//!
//! Finding it is [`latchkey`]'s; this is what is said once the door opens.

use super::wire::{self, Mismatch, Request, Response};
use mail_domain::AccountId;
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

        read(&mut BufReader::new(&mut self.stream))
    }
}

/// Read one answer, and say what a failure to was in a person's words.
fn read(reader: &mut impl BufRead) -> Result<Response, String> {
    let mut reply = String::new();
    reader
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

/// A subscription: what a daemon says after each pass, as it says it.
///
/// The reader is kept for the life of the connection, not made per line as [`Daemon::ask`] makes
/// one. A daemon may say [`Response::Changed`] in the same write as [`Response::Subscribed`], and
/// a reader dropped after the first line would take the second with it.
#[derive(Debug)]
pub struct Changes {
    reader: BufReader<latchkey::Stream>,
}

impl Changes {
    /// Wait for the next pass that may have stored something, and say which account it was on.
    ///
    /// Blocks, for as long as the daemon is quiet. An error is the subscription's end: the
    /// daemon went away, or said something that is not a change.
    pub fn wait(&mut self) -> Result<AccountId, String> {
        match read(&mut self.reader)? {
            Response::Changed { account } => Ok(account),
            other => Err(format!(
                "the daemon said {other:?} where a change was expected"
            )),
        }
    }
}

/// Subscribe to the daemon at `agent`, if one is there.
///
/// `Ok(None)` means nothing is listening, which is the usual state of a session with no watch
/// running; nothing is started, because a listener has no reason to want a daemon that would
/// not otherwise run. An error is a daemon that is there and would not subscribe it, most often
/// one from another build ([`Response::WrongVersion`], or a version-1 daemon's line that this
/// build cannot read).
pub fn subscribe(agent: &latchkey::Agent) -> Result<Option<Changes>, String> {
    let Some(mut stream) = agent.connect().map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    let text = wire::line(Request::Subscribe)?;
    stream
        .write_all(text.as_bytes())
        .map_err(|e| format!("cannot reach the daemon: {e}"))?;
    stream
        .flush()
        .map_err(|e| format!("cannot reach the daemon: {e}"))?;
    let mut reader = BufReader::new(stream);
    match read(&mut reader)? {
        Response::Subscribed => Ok(Some(Changes { reader })),
        Response::WrongVersion { daemon, client } => Err(Mismatch::Version {
            theirs: daemon,
            ours: client,
        }
        .to_string()),
        other => Err(format!("the daemon would not subscribe: {other:?}")),
    }
}

/// Connect to the running daemon, if there is one.
///
/// `Ok(None)` means nothing is listening — not an error, because the usual answer to it is to
/// start one.
pub fn connect() -> Result<Option<Daemon>, String> {
    connect_at(&super::agent()?)
}

/// Connect to whatever is listening at `agent`: [`connect`], for the watch's door or a test's.
pub fn connect_at(agent: &latchkey::Agent) -> Result<Option<Daemon>, String> {
    agent
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
