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

/// Listen for clients, replacing a socket left behind by a dead daemon.
///
/// The parent directory is created `0700` where the platform supports it: the socket's
/// permissions are the whole of the authentication here, and a mail daemon that any local
/// process could talk to would be a mail daemon that any local process could read mail from.
#[cfg(unix)]
pub fn bind(path: &std::path::Path) -> Result<std::os::unix::net::UnixListener, String> {
    use std::os::unix::fs::PermissionsExt as _;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        let _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
    }
    // A socket that is already answering means a daemon is already running, and taking its
    // address would be two daemons fetching the same mail twice. Checked by *connecting*,
    // because the file existing says nothing about whether anyone is behind it.
    if path.exists() {
        if std::os::unix::net::UnixStream::connect(path).is_ok() {
            return Err(format!(
                "a daemon is already listening on {}",
                path.display()
            ));
        }
        std::fs::remove_file(path).map_err(|e| format!("cannot clear {}: {e}", path.display()))?;
    }
    let listener = std::os::unix::net::UnixListener::bind(path)
        .map_err(|e| format!("cannot listen on {}: {e}", path.display()))?;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    Ok(listener)
}
