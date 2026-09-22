//! Find this user's background agent, or start one. One logic, three platforms.
//!
//! A *latchkey* is the key to your own front door: it lets you in, and it tells you whether
//! anyone is already home. That is the whole of this crate.
//!
//! ```no_run
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let agent = latchkey::Agent::new("mailo")?;
//!
//! // In the agent process:
//! let door = agent.listen()?;             // or Err(Error::AlreadyRunning)
//! for client in door.incoming() {
//!     let mut client = client?;
//!     // `client` is Read + Write. It is a Unix socket on Linux and macOS and a named pipe on
//!     // Windows, and this code does not have to know which.
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ```no_run
//! # fn main() -> Result<(), latchkey::Error> {
//! # let agent = latchkey::Agent::new("mailo")?;
//! // In the client process:
//! let mut daemon = agent.connect_or_start(
//!     || latchkey::spawn(&["daemon"]),    // how to launch one; yours to decide
//!     std::time::Duration::from_secs(5),
//! )?;
//! # Ok(())
//! # }
//! ```
//!
//! # What this is, and what it leaves alone
//!
//! The transport half of interprocess communication is solved: [`interprocess`] already unifies
//! Unix domain sockets and Windows named pipes behind one `Read + Write` stream, and this crate
//! is built on it rather than beside it. What is *not* solved anywhere, and what everyone who
//! needs it reimplements, is the lifecycle around that stream:
//!
//! - **Where does the socket go**, per platform, without exceeding `sun_path`?
//! - **Is one already running**, and can that be answered without a race?
//! - **Who starts it**, and how long does a client wait?
//! - **What is left behind** when it is killed rather than asked to stop?
//!
//! Those four are what this crate decides. What it deliberately does not decide is the
//! conversation: there is no message framing, no protocol version, no request type. Those differ
//! per application and a crate that chose them for you would be one you fought.
//!
//! # The one difference it cannot hide
//!
//! **A busy agent refuses on Windows and queues on Unix.** A Unix domain socket holds pending
//! connections in a backlog, so a client that knocks mid-conversation waits its turn and notices
//! nothing; a Windows named pipe serves one client per instance and turns the rest away. So
//! [`Agent::connect`] has a third answer besides "reached" and "nobody home": [`Error::Busy`],
//! meaning somebody *is* home and cannot come to the door. [`Agent::connect_or_start`] treats it
//! as proof of life and keeps knocking, because starting an agent then would start a rival to
//! one that is demonstrably alive.
//!
//! The knock is bounded for the same reason. `interprocess` defaults to unbounded waiting, which
//! on Windows is `WaitNamedPipeW(NMPWAIT_WAIT_FOREVER)`, so the default "is anyone home?" can
//! block for ever. This crate always passes a timeout.
//!
//! # Why not just use systemd
//!
//! You should, where you can. systemd socket activation and launchd's `Sockets` key are both
//! better than this: the init system binds the socket at login and starts your process on the
//! first connection, so there is no stale socket, no spawn race and no timeout to pick.
//!
//! The catch is that they are three different answers, and the third does not exist. A tool
//! installed by `cargo install` or `brew` or a shell script has no installation step in which to
//! write a unit file, and must work anyway — in a container, over ssh, on a colleague's laptop,
//! on the Mac someone tries it on next. This crate is that case: demand-start, nothing enabled,
//! and a per-user agent that a unit file can later be layered *on top of* to start at login
//! rather than to work at all.
//!
//! # Guarantees
//!
//! - **One agent per user.** Enforced by an advisory file lock, not by looking at the socket.
//!   The difference is [`lock`], and it is the reason this crate exists as something other than
//!   fifty lines in your own repository.
//! - **A killed agent locks nobody out.** The kernel releases the lock with the process,
//!   including under `SIGKILL` and a power cut, so there is no stale state to reap and no
//!   timeout after which a lock is assumed dead.
//! - **A client never deletes anything.** Knocking and tidying are different jobs.
//! - **Every platform rule is a pure function.** [`address::address_on`] takes the environment as
//!   arguments, so the macOS and Windows answers are tested from whatever machine runs
//!   `cargo test`. A rule that reads its own environment can only be checked on the machine it
//!   was written on, which for a cross-platform agent is the one thing that must not be true.

pub mod address;
pub mod find;
pub mod lock;
pub mod serve;

pub use address::{Address, Endpoint, Environment, Host, OwnedEnvironment, address_on, here};
pub use find::spawn;
pub use serve::{Listening, Stream};

use std::path::{Path, PathBuf};
use std::time::Duration;

/// One named agent, for this user, on this machine.
///
/// Built once and kept: it holds the environment it read, so a client and the agent it starts
/// agree on the address even if something changes `TMPDIR` underneath them.
#[derive(Debug, Clone)]
pub struct Agent {
    name: String,
    address: Address,
}

impl Agent {
    /// An agent called `name`, addressed by this platform's rules.
    ///
    /// The name becomes a directory on Unix and part of a machine-wide pipe name on Windows, so
    /// it is restricted to ASCII letters, digits and `-`. Keep it short: on macOS the whole
    /// socket path has 104 bytes to fit in, and `TMPDIR` has already spent forty of them.
    pub fn new(name: &str) -> Result<Self, Error> {
        let env = Environment::read();
        Self::in_environment(name, here(), &env.borrow())
    }

    /// The same, told where it is and what the environment says.
    ///
    /// For tests, and for a program that wants its agent somewhere specific.
    pub fn in_environment(name: &str, host: Host, env: &Environment<'_>) -> Result<Self, Error> {
        Ok(Self {
            name: name.to_owned(),
            address: address_on(name, host, env)?,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn address(&self) -> &Address {
        &self.address
    }

    /// The socket, for printing. `None` on Windows, where the address is a pipe name.
    pub fn socket(&self) -> Option<&Path> {
        match &self.address.endpoint {
            Endpoint::Socket(path) => Some(path),
            Endpoint::Pipe(_) => None,
        }
    }

    /// Become the agent, or report that another process already is.
    pub fn listen(&self) -> Result<Listening, Error> {
        serve::listen(&self.address)
    }

    /// Knock. `Ok(None)` means nobody is home.
    pub fn connect(&self) -> Result<Option<Stream>, Error> {
        find::connect(&self.address)
    }

    /// Knock, and start one if nobody answers.
    pub fn connect_or_start(
        &self,
        start: impl FnOnce() -> Result<(), Error>,
        wait: Duration,
    ) -> Result<Stream, Error> {
        find::connect_or_start(&self.address, start, wait)
    }

    /// Is one running? Asked by taking the lock and giving it straight back.
    ///
    /// Racy by nature — the answer can be stale before it is returned — so it is for reporting
    /// to a person, never for deciding whether to start. [`Agent::listen`] is that decision, and
    /// it is one step rather than two for exactly this reason.
    pub fn is_running(&self) -> bool {
        // Only `AlreadyRunning` counts. A directory that cannot be created, or a filesystem that
        // will not lock, is a different problem and answering "yes, one is running" to it would
        // send the caller looking for an agent that is not there.
        matches!(lock::take(&self.address.lock), Err(Error::AlreadyRunning))
    }
}

/// Everything that can go wrong finding, starting or becoming an agent.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// Another process holds the lock. Not a failure: it is the answer to "should I start?".
    AlreadyRunning,
    /// An agent is alive and every connection slot is in use.
    ///
    /// Windows only in practice: a named pipe serves one client per instance and refuses the
    /// rest, where a Unix socket holds them in its backlog and says nothing. Distinct from
    /// "nobody home" because the right response is to knock again, never to start a rival.
    Busy,
    /// The name is not one that can safely become a path or a pipe.
    BadName(String),
    /// Nothing in the environment says where a per-user runtime directory is.
    Homeless(&'static str),
    /// The socket path exceeds `sun_path`. Says all three numbers because nothing can be done
    /// about it without them.
    TooLong {
        path: PathBuf,
        length: usize,
        limit: usize,
    },
    /// A file or directory would not cooperate.
    Io {
        path: PathBuf,
        cause: std::io::Error,
    },
    /// The door would not open.
    Listen {
        endpoint: Endpoint,
        cause: std::io::Error,
    },
    /// The door would not answer, for a reason other than being shut.
    Connect {
        endpoint: Endpoint,
        cause: std::io::Error,
    },
    /// An agent was started and never answered within the time allowed.
    NeverAnswered(Duration),
    /// `current_exe` failed, so there is no binary to start.
    NoSelf(std::io::Error),
    /// The agent process would not start.
    CannotSpawn(std::io::Error),
}

impl Error {
    fn at(path: &Path, cause: std::io::Error) -> Self {
        Error::Io {
            path: path.to_path_buf(),
            cause,
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::AlreadyRunning => write!(f, "an agent is already running for this user"),
            Error::Busy => write!(f, "the agent is busy with another client"),
            Error::BadName(name) => write!(
                f,
                "{name:?} is not a usable agent name: ASCII letters, digits and '-' only"
            ),
            Error::Homeless(what) => write!(f, "nowhere to put the socket: {what}"),
            Error::TooLong {
                path,
                length,
                limit,
            } => write!(
                f,
                "the socket path is {length} bytes and this platform allows {limit}: {}",
                path.display()
            ),
            Error::Io { path, cause } => write!(f, "{}: {cause}", path.display()),
            Error::Listen { endpoint, cause } => write!(f, "cannot listen on {endpoint}: {cause}"),
            Error::Connect { endpoint, cause } => write!(f, "cannot reach {endpoint}: {cause}"),
            Error::NeverAnswered(wait) => write!(
                f,
                "started an agent and it did not answer within {}s",
                wait.as_secs()
            ),
            Error::NoSelf(cause) => write!(f, "cannot find this binary: {cause}"),
            Error::CannotSpawn(cause) => write!(f, "cannot start the agent: {cause}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io { cause, .. }
            | Error::Listen { cause, .. }
            | Error::Connect { cause, .. }
            | Error::NoSelf(cause)
            | Error::CannotSpawn(cause) => Some(cause),
            _ => None,
        }
    }
}

impl std::fmt::Display for Endpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Endpoint::Socket(path) => write!(f, "{}", path.display()),
            Endpoint::Pipe(name) => write!(f, "pipe {name}"),
        }
    }
}
