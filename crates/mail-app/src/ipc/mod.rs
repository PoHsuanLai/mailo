//! Where the daemon listens, and how a client finds it.
//!
//! `mailo watch` already holds connections open for as long as the process lives. A daemon is
//! that loop with a door in it, so the command line, the window and anything else become clients
//! of one sync engine rather than three programs opening three sets of sockets to the same
//! servers.
//!
//! **This is the transport, not the daemon.** What is here is the address, the wire and the
//! decision to start one; what runs behind it is still `sync::drive`.
//!
//! # Why not a TCP port on loopback
//!
//! Because anything on the machine could connect to it, and this repository already knows what
//! that costs: `mail-runtime/src/loopback.rs` validates the OAuth `state` parameter precisely
//! because "a loopback listener is reachable by anything on the machine". A Unix socket in a
//! directory only this user can read needs no such argument — the filesystem is the
//! authentication.
//!
//! # The platform seam
//!
//! Three things differ per platform and nothing else does: what kind of address it is, where it
//! lives, and how long it may be. All three are decided by [`endpoint_on`], which takes the
//! environment as arguments so the macOS and Windows answers can be *tested from Linux* — the
//! same shape `attach::downloads_from` uses, and for the same reason: a rule that reads its own
//! environment can only be tested on the machine it was written on.

pub mod client;
pub mod daemon;
pub mod wire;

use std::ffi::OsStr;
use std::path::PathBuf;

/// Which platform's rules to apply.
///
/// A value rather than a `cfg!`, so every branch is reachable from a test on any machine. The
/// real one is chosen once, in [`here`].
///
/// Two of the three are dead code in any one build, which is the point rather than an oversight:
/// the whole reason this is a value is that a Linux machine can check the macOS and Windows
/// rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum Host {
    Linux,
    Mac,
    Windows,
}

/// The address the daemon listens on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Endpoint {
    /// A Unix domain socket. Linux and macOS.
    Socket(PathBuf),
    /// A named pipe. Windows, where `AF_UNIX` exists in the OS since 1803 but not in std or
    /// tokio, so it is not something to build on.
    Pipe(String),
}

/// What [`endpoint_on`] reads, gathered rather than looked up.
#[derive(Debug, Default, Clone)]
pub struct Environment<'a> {
    /// `XDG_RUNTIME_DIR`. Linux only: tmpfs, `0700`, and emptied when the session ends, which is
    /// exactly the lifetime a socket wants.
    pub runtime_dir: Option<&'a OsStr>,
    /// `TMPDIR`. On macOS this is per-user and private (`/var/folders/…`), which makes it the
    /// closest thing that platform has to a runtime directory.
    pub tmpdir: Option<&'a OsStr>,
    pub home: Option<&'a OsStr>,
    /// Used only to name a Windows pipe, which shares one namespace across every session.
    pub user: Option<&'a str>,
}

/// The longest a Unix socket path may be, including its terminating NUL.
///
/// `sun_path` is 108 bytes on Linux and 104 on macOS, and the limit is not advisory: a longer
/// path is silently truncated by some libcs and refused by others. It matters most on macOS,
/// where `TMPDIR` is something like
/// `/var/folders/qw/8p3n1x_d4tz9g7v0_0000gn/T/` before anything is appended.
const fn sun_path_limit(host: Host) -> usize {
    match host {
        Host::Mac => 104,
        _ => 108,
    }
}

/// Where the daemon for this user listens, given an environment.
///
/// Pure, so the macOS and Windows rules are checked by tests running on Linux. The order within
/// each platform is "most appropriate first, then something that always exists".
pub fn endpoint_on(host: Host, env: &Environment<'_>) -> Result<Endpoint, String> {
    if host == Host::Windows {
        // One namespace for the whole machine, so the name carries the user. Two people on one
        // Windows box are two daemons, and without this they would be one.
        let user = env.user.unwrap_or("default");
        return Ok(Endpoint::Pipe(format!(r"\\.\pipe\mailo-{user}")));
    }

    let dir = match host {
        // A runtime directory is the right answer where there is one: the socket goes away with
        // the session, which is what stops a stale one outliving a reboot.
        Host::Linux => env
            .runtime_dir
            .map(PathBuf::from)
            .or_else(|| env.home.map(|h| PathBuf::from(h).join(".cache")))
            .ok_or("neither XDG_RUNTIME_DIR nor HOME is set")?,
        // macOS has no XDG_RUNTIME_DIR. Its `TMPDIR` is per-user and private, which is the
        // property that matters; `~/Library/Caches` is the fallback when it is unset.
        Host::Mac => env
            .tmpdir
            .map(PathBuf::from)
            .or_else(|| env.home.map(|h| PathBuf::from(h).join("Library/Caches")))
            .ok_or("neither TMPDIR nor HOME is set")?,
        Host::Windows => unreachable!("handled above"),
    };

    let path = dir.join("mailo").join("daemon.sock");
    let length = path.as_os_str().len() + 1; // the NUL
    if length > sun_path_limit(host) {
        // Said here rather than discovered as a truncated path that binds somewhere unexpected.
        return Err(format!(
            "the socket path is {length} bytes and this platform allows {}: {}",
            sun_path_limit(host),
            path.display()
        ));
    }
    Ok(Endpoint::Socket(path))
}

/// Which platform this build is for.
pub const fn here() -> Host {
    #[cfg(target_os = "macos")]
    {
        Host::Mac
    }
    #[cfg(target_os = "windows")]
    {
        Host::Windows
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        Host::Linux
    }
}

/// Where the daemon for this user listens.
pub fn endpoint() -> Result<Endpoint, String> {
    let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR");
    let tmpdir = std::env::var_os("TMPDIR");
    let home = std::env::var_os("HOME");
    let user = std::env::var("USER").ok();
    endpoint_on(
        here(),
        &Environment {
            runtime_dir: runtime_dir.as_deref(),
            tmpdir: tmpdir.as_deref(),
            home: home.as_deref(),
            user: user.as_deref(),
        },
    )
}

/// Listening, and the proof that we are the only one doing so.
///
/// The listener and the lock are one object because their lifetimes are one fact: while this
/// value exists, this user has exactly one mailo daemon, and when it goes away the socket goes
/// with it.
#[cfg(unix)]
#[derive(Debug)]
pub struct Listening {
    listener: std::os::unix::net::UnixListener,
    socket: std::path::PathBuf,
    /// Never read. Held open because dropping it releases the lock, which is the entire point.
    _lock: std::fs::File,
}

#[cfg(unix)]
impl Listening {
    pub fn incoming(&self) -> std::os::unix::net::Incoming<'_> {
        self.listener.incoming()
    }
}

#[cfg(unix)]
impl Drop for Listening {
    /// Unlink first, release second.
    ///
    /// Field drops run after this body, so the lock is still held while the socket is removed. A
    /// daemon waiting on the lock therefore never sees the moment where it is free and the old
    /// socket is still on disk.
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.socket);
    }
}

/// The file whose lock means "this user's daemon is running".
///
/// Beside the socket rather than inside it: a socket cannot be locked, and the lock has to
/// survive the socket being replaced.
#[cfg(unix)]
fn lock_beside(socket: &std::path::Path) -> std::path::PathBuf {
    socket.with_extension("lock")
}

/// Listen for clients, refusing if this user already has a daemon.
///
/// The parent directory is created `0700` where the platform supports it: the socket's
/// permissions are the whole of the authentication here, and a mail daemon that any local
/// process could talk to would be a mail daemon that any local process could read mail from.
///
/// # Why a lock file and not a connection attempt
///
/// This used to ask "is anything answering on the socket?" and clear the file when nothing was.
/// That reads correctly and is wrong under concurrency: between one daemon's failed connect and
/// its `remove_file`, another daemon can bind, and the first then deletes a working daemon's
/// door. Two clients running `mailo ping` at the same moment on a cold machine is all it takes.
///
/// An advisory lock has no such window, because the kernel does the arbitration rather than this
/// code. It also answers the stale question exactly, instead of by inference: a lock is released
/// when the holding *process* dies, including under `SIGKILL` where no destructor runs, so a
/// lock that can be taken proves there is no daemon — where a socket that refuses connections
/// only suggests it.
///
/// The lock file itself is never removed. Unlinking it would reintroduce the race in a worse
/// form, since a second daemon could be holding the lock on the very inode being deleted and
/// a third would then lock a fresh file and see no conflict. It is empty, it lives in the
/// runtime directory, and leaving it there costs an inode.
#[cfg(unix)]
pub fn bind(path: &std::path::Path) -> Result<Listening, String> {
    use std::os::unix::fs::PermissionsExt as _;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        let _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
    }

    let guard = lock_beside(path);
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&guard)
        .map_err(|e| format!("cannot open {}: {e}", guard.display()))?;
    let _ = std::fs::set_permissions(&guard, std::fs::Permissions::from_mode(0o600));
    match lock.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => {
            // Two daemons on one mailbox would fetch everything twice and race each other's
            // writes into the same SQLite file.
            return Err(format!(
                "a daemon is already listening on {}",
                path.display()
            ));
        }
        Err(std::fs::TryLockError::Error(e)) => {
            return Err(format!("cannot lock {}: {e}", guard.display()));
        }
    }

    // Past the lock, so any socket still here was left by a process that is gone: no live daemon
    // could be behind it, because a live daemon would still hold the lock.
    if path.exists() {
        std::fs::remove_file(path).map_err(|e| format!("cannot clear {}: {e}", path.display()))?;
    }
    let listener = std::os::unix::net::UnixListener::bind(path)
        .map_err(|e| format!("cannot listen on {}: {e}", path.display()))?;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    Ok(Listening {
        listener,
        socket: path.to_path_buf(),
        _lock: lock,
    })
}
