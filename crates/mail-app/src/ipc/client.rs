//! Finding the daemon, or starting one.
//!
//! The pattern is `ssh-agent`'s and `gpg-agent`'s: nothing is installed, nothing is enabled, and
//! the first command that wants a daemon starts it. A systemd user unit or a launchd agent then
//! becomes optional — "start it at login" rather than "make it work at all" — which is what keeps
//! the same story true on a Mac that has neither.
//!
//! The awkward case is not "no daemon". It is a socket *file* with nothing behind it: the
//! process was killed, the machine lost power, and what is left looks exactly like a daemon
//! until you try to talk to it. A client tells the difference by connecting, and then does
//! nothing about it — clearing the file is [`super::bind`]'s job, under a lock, because a
//! client that tidies races every other client that is doing the same.

use super::wire::{self, Mismatch, Request, Response};
use super::{Endpoint, endpoint};
use std::io::{BufRead, BufReader, Write};
use std::time::{Duration, Instant};

/// How long to wait for a daemon we just started to answer.
///
/// It binds its socket before it does anything else, so this is process startup, not a sync.
const STARTUP: Duration = Duration::from_secs(5);

/// How often to retry while waiting for that.
const RETRY: Duration = Duration::from_millis(50);

/// A connection to the daemon.
#[derive(Debug)]
pub struct Daemon {
    stream: std::os::unix::net::UnixStream,
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
        BufReader::new(&self.stream)
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
/// `Ok(None)` means nothing is listening — which is not an error, because the usual answer to it
/// is to start one.
pub fn connect() -> Result<Option<Daemon>, String> {
    let Endpoint::Socket(path) = endpoint()? else {
        // The Windows half of the seam. Named pipes are `interprocess` or
        // `tokio::net::windows::named_pipe`, and neither compiles here, so this is the honest
        // state rather than a silent fallback to something that is not a pipe.
        return Err("named pipes are not implemented yet; this build is Unix only".to_owned());
    };
    connect_at(&path)
}

/// The same, at a named address.
///
/// Separate because `connect` reads the environment, and a test that used it would be talking to
/// whatever daemon this user happens to have running — which is both a flaky test and a rude one.
pub fn connect_at(path: &std::path::Path) -> Result<Option<Daemon>, String> {
    if !path.exists() {
        return Ok(None);
    }
    match std::os::unix::net::UnixStream::connect(path) {
        Ok(stream) => Ok(Some(Daemon { stream })),
        // The stale case: a file is there and nothing is behind it. `ECONNREFUSED` is what a
        // socket whose listener has gone answers, and `ENOENT` is a race with someone tidying.
        Err(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound
            ) =>
        {
            Ok(None)
        }
        Err(e) => Err(format!("cannot open {}: {e}", path.display())),
    }
}

/// Connect, and start a daemon if nothing answers.
///
/// `start` is how to launch one — a parameter rather than a call to `std::process::Command`, so
/// a test can substitute something that is not a mail daemon and this function can be checked
/// without one.
pub fn connect_or_start(
    path: &std::path::Path,
    start: impl FnOnce() -> Result<(), String>,
    wait: Duration,
) -> Result<Daemon, String> {
    if let Some(live) = connect_at(path)? {
        return Ok(live);
    }

    // Nothing is removed here, deliberately. Clearing the socket on a failed connect looks
    // tidy and is a race: between this client's failed connect and its `remove_file`, another
    // client's daemon can bind, and this one would then delete a working daemon's door. Two
    // people running `mailo ping` at the same moment on a cold machine is all it takes.
    //
    // Only `ipc::bind` clears a socket, and it does so holding the lock that proves no daemon
    // owns it. A client's job is to knock, not to tidy.
    start()?;

    let deadline = Instant::now() + wait;
    loop {
        if let Some(live) = connect_at(path)? {
            return Ok(live);
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "started a daemon and it did not answer within {}s",
                wait.as_secs()
            ));
        }
        std::thread::sleep(RETRY);
    }
}

/// Launch this same binary as a daemon.
///
/// `current_exe` rather than a name on `PATH`: the daemon a client starts must be the build the
/// client came from, or an upgrade would leave a new CLI talking to whatever old binary happened
/// to be installed first.
pub fn spawn_here() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("cannot find this binary: {e}"))?;
    std::process::Command::new(exe)
        .arg("daemon")
        // Detached: the daemon outlives the command that started it, which is the whole point.
        // Its output goes nowhere by design — a daemon writing to the terminal of whichever
        // client happened to start it is a daemon that scribbles over an unrelated session.
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("cannot start the daemon: {e}"))
}

/// Connect to a daemon, starting one if needed, with the usual timeout.
pub fn reach() -> Result<Daemon, String> {
    let Endpoint::Socket(path) = endpoint()? else {
        return Err("named pipes are not implemented yet; this build is Unix only".to_owned());
    };
    connect_or_start(&path, spawn_here, STARTUP)
}
