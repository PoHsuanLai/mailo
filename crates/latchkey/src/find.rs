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
use interprocess::ConnectWaitMode;
use interprocess::local_socket::{ConnectOptions, GenericFilePath, GenericNamespaced, prelude::*};
use std::time::{Duration, Instant};

/// How often to knock while waiting for an agent we just started.
const RETRY: Duration = Duration::from_millis(50);

/// How long one knock may take before it is abandoned.
///
/// Not a nicety. `interprocess` defaults to [`ConnectWaitMode::Unbounded`], which on Windows
/// means `WaitNamedPipeW(NMPWAIT_WAIT_FOREVER)` when every instance of the pipe is busy — so a
/// client that knocks while the agent is mid-conversation with somebody else waits for ever,
/// with no timeout and nothing to cancel it. That is not hypothetical: it hung a CI job for an
/// hour and forty minutes before it was cancelled.
///
/// A bounded knock is also the honest shape of the operation. `connect` answers "is anyone
/// home?", and a question that can take unbounded time is not that question.
const KNOCK: Duration = Duration::from_secs(2);

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
    let name = match &at.endpoint {
        Endpoint::Socket(path) => {
            if !path.exists() {
                return Ok(None);
            }
            path.as_os_str()
                .to_fs_name::<GenericFilePath>()
                .map_err(|e| Error::at(path, e))?
        }
        Endpoint::Pipe(pipe) => pipe
            .as_str()
            .to_ns_name::<GenericNamespaced>()
            .map_err(|e| Error::Listen {
                endpoint: at.endpoint.clone(),
                cause: e,
            })?,
    };
    let reached = ConnectOptions::new()
        .name(name)
        .wait_mode(ConnectWaitMode::Timeout(KNOCK))
        .connect_sync();

    match reached {
        Ok(stream) => Ok(Some(stream)),
        // The ways "nobody is home" arrives. `ConnectionRefused` is what a socket whose listener
        // has gone answers; `NotFound` is a race with an agent tidying on the way out, and the
        // name a Windows pipe gives when it was never created at all.
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
        // Somebody *is* home and cannot come to the door: every instance of the pipe is in use,
        // or the connect timed out waiting for one. This is its own answer and not `None`,
        // because the right response to it is to knock again — starting a second agent would be
        // starting a rival to one that is demonstrably alive.
        Err(e) if busy(&e) => Err(Error::Busy),
        Err(e) => Err(Error::Connect {
            endpoint: at.endpoint.clone(),
            cause: e,
        }),
    }
}

/// "Someone is home but cannot come to the door."
///
/// Windows named pipes serve one client per instance, so a busy agent refuses rather than
/// queues — where a Unix socket would hold the connection in its backlog and say nothing. The
/// kinds are checked as well as the raw code because which one `ERROR_PIPE_BUSY` maps to has
/// moved between Rust releases, and an unrecognised busy would become a hard error.
fn busy(e: &std::io::Error) -> bool {
    const ERROR_PIPE_BUSY: i32 = 231;
    matches!(
        e.kind(),
        std::io::ErrorKind::ResourceBusy
            | std::io::ErrorKind::TimedOut
            | std::io::ErrorKind::WouldBlock
    ) || e.raw_os_error() == Some(ERROR_PIPE_BUSY)
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
    match connect(at) {
        Ok(Some(live)) => return Ok(live),
        Ok(None) => {}
        // Busy means an agent is alive and occupied, so starting one would start a rival. Fall
        // through to the waiting loop without calling `start`.
        Err(Error::Busy) => return wait_for(at, wait),
        Err(other) => return Err(other),
    }
    start()?;

    let deadline = Instant::now() + wait;
    loop {
        match connect(at) {
            Ok(Some(live)) => return Ok(live),
            // Still busy, still alive: keep knocking rather than give up on it.
            Ok(None) | Err(Error::Busy) => {}
            Err(other) => return Err(other),
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

/// Knock until an agent that is known to exist can actually talk.
fn wait_for(at: &Address, wait: Duration) -> Result<Stream, Error> {
    let deadline = Instant::now() + wait;
    loop {
        match connect(at) {
            Ok(Some(live)) => return Ok(live),
            Ok(None) | Err(Error::Busy) => {}
            Err(other) => return Err(other),
        }
        if Instant::now() >= deadline {
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
///
/// # Windows: the agent inherits the starting client's handles
///
/// This is a real limitation and not a footnote. `CreateProcess` is called with
/// `bInheritHandles: TRUE` — the standard library has no stable way to say otherwise — and, as
/// its own source puts it, "once an inheritable handle is created, *any* spawned child will
/// inherit that handle". Setting the agent's own stdio to null does not help: the handles at
/// issue are the *client's*, and they were made inheritable by whoever started the client.
///
/// The consequence is specific. If the client's stdout is a pipe that someone is reading to
/// end-of-file — `$(mytool status)` in a shell, `Command::output()` in a test, any CI step that
/// captures output — that read does not finish when the client exits, because the agent is still
/// holding the write end and the agent is meant to live for hours. The reader waits for the
/// agent, which is waiting for a request, which will not come.
///
/// Two ways around it, in order of preference:
///
/// 1. **Have the client write somewhere that has no end-of-file**, such as a file or the
///    terminal. This is a one-line change in the client and costs nothing.
/// 2. **Supply your own `start`.** [`connect_or_start`] takes it as a closure precisely so this
///    function is a convenience rather than a constraint; a caller who needs `STARTUPINFOEX` and
///    `PROC_THREAD_ATTRIBUTE_HANDLE_LIST` can have them.
///
/// Fixing it here needs one `unsafe` call to `SetHandleInformation` or an equivalent dependency.
/// It is not done because this crate compiles under `unsafe_code = "forbid"` and the workaround
/// above is cheap; if that trade stops being the right one, this is where it changes.
pub fn spawn(args: &[&str]) -> Result<(), Error> {
    let exe = std::env::current_exe().map_err(Error::NoSelf)?;
    let mut command = std::process::Command::new(exe);
    command
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    detach(&mut command);
    command.spawn().map(|_| ()).map_err(Error::CannotSpawn)
}

/// Cut the agent loose from the console that started it.
///
/// `DETACHED_PROCESS` stops the agent joining the client's console, so closing the terminal that
/// ran the first command does not take the agent with it, and `CREATE_NO_WINDOW` stops a console
/// window appearing for an agent nobody is looking at. Neither affects handle inheritance — see
/// [`spawn`] — they are the part of "detached" that *is* expressible in safe code.
#[cfg(windows)]
fn detach(command: &mut std::process::Command) {
    use std::os::windows::process::CommandExt as _;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW);
}

/// Nothing to do: on Unix the child already outlives its parent, and `Stdio::null` has already
/// severed the three handles that matter. Pipes are `CLOEXEC`, so nothing else is inherited.
#[cfg(not(windows))]
fn detach(_command: &mut std::process::Command) {}
