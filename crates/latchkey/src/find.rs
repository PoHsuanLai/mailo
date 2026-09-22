//! The client end: knock, and start one if nobody answers.
//!
//! The pattern is `ssh-agent`'s and `gpg-agent`'s, and it is chosen for what it does *not*
//! require. Nothing is installed, nothing is enabled, and the first command that wants an agent
//! starts it. A systemd user unit or a launchd agent then becomes "start it at login" rather
//! than "make it work at all", which is what keeps the same story true on a machine that has
//! neither — a Mac, a container, a colleague's laptop, an ssh session.

use crate::Error;
use crate::address::{Address, Endpoint};
use crate::serve::Stream;
use interprocess::local_socket::{GenericFilePath, GenericNamespaced, prelude::*};
use std::time::{Duration, Instant};

/// How often to knock while waiting for an agent we just started.
const RETRY: Duration = Duration::from_millis(50);

/// Connect to the running agent, if there is one.
///
/// `Ok(None)` means nobody answered, which is not an error: the usual reply to it is to start
/// one. A socket file with nothing behind it reads as `None` too, because a killed process
/// leaves its door standing and only knocking tells the difference.
///
/// Nothing is removed here, ever. A client that tidies races every other client doing the same —
/// see [`crate::lock`] — so clearing a stale socket is [`crate::serve::listen`]'s job, under the
/// lock that proves it is nobody's.
pub fn connect(at: &Address) -> Result<Option<Stream>, Error> {
    let reached = match &at.endpoint {
        Endpoint::Socket(path) => {
            if !path.exists() {
                return Ok(None);
            }
            let name = path
                .as_os_str()
                .to_fs_name::<GenericFilePath>()
                .map_err(|e| Error::at(path, e))?;
            Stream::connect(name)
        }
        Endpoint::Pipe(pipe) => {
            let name = pipe
                .as_str()
                .to_ns_name::<GenericNamespaced>()
                .map_err(|e| Error::Listen {
                    endpoint: at.endpoint.clone(),
                    cause: e,
                })?;
            Stream::connect(name)
        }
    };
    match reached {
        Ok(stream) => Ok(Some(stream)),
        // The three ways "nobody is home" arrives. `ConnectionRefused` is what a socket whose
        // listener has gone answers; `NotFound` is a race with an agent tidying on the way out,
        // and the name a Windows pipe gives when it was never created at all.
        Err(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::ConnectionRefused
                    | std::io::ErrorKind::NotFound
                    | std::io::ErrorKind::ConnectionReset
            ) =>
        {
            Ok(None)
        }
        Err(e) => Err(Error::Connect {
            endpoint: at.endpoint.clone(),
            cause: e,
        }),
    }
}

/// Connect, and start an agent if nobody answers.
///
/// `start` is a closure rather than a `Command` this crate builds, because how an agent is
/// launched is the one part of this that is genuinely the caller's: a subcommand of the same
/// binary, a separate executable, a service the caller asks the platform to start. [`spawn`]
/// covers the common case.
///
/// `wait` is process startup, not the agent's first piece of work — an agent should take its
/// lock and open its door before it does anything slow, precisely so this number can stay small.
pub fn connect_or_start(
    at: &Address,
    start: impl FnOnce() -> Result<(), Error>,
    wait: Duration,
) -> Result<Stream, Error> {
    if let Some(live) = connect(at)? {
        return Ok(live);
    }
    start()?;

    let deadline = Instant::now() + wait;
    loop {
        if let Some(live) = connect(at)? {
            return Ok(live);
        }
        if Instant::now() >= deadline {
            // Said rather than waited out. A client blocked for ever on an agent that failed to
            // start is worse than one that gives up, because the second can be retried by a
            // person who can also read why.
            return Err(Error::NeverAnswered(wait));
        }
        std::thread::sleep(RETRY);
    }
}

/// Launch this same binary, detached, with the given arguments.
///
/// `current_exe` rather than a name on `PATH`: the agent a client starts must be the build the
/// client came from, or an upgrade leaves a new client talking to whichever old binary happened
/// to be installed first — and since agents are long-lived, that is not a rare state.
///
/// The child's output goes nowhere by design. An agent writing to the terminal of whichever
/// client happened to start it is an agent that scribbles over an unrelated session; anything it
/// has to say belongs in a log it chooses or in an answer to a request.
pub fn spawn(args: &[&str]) -> Result<(), Error> {
    let exe = std::env::current_exe().map_err(Error::NoSelf)?;
    std::process::Command::new(exe)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(Error::CannotSpawn)
}
