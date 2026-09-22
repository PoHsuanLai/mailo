//! The one mechanism that makes "one agent per user" true.
//!
//! # Why a lock and not a look
//!
//! The obvious way to find out whether an agent is running is to look at its socket: is the file
//! there, and does anything answer on it? That reads correctly and is wrong under concurrency,
//! in two ways that are easy to write and hard to see.
//!
//! *A client that tidies races every other client.* Between a failed connect and the
//! `remove_file` it does on the strength of it, another client's agent can bind — and the
//! tidying client then deletes a working agent's door. Two people running the same command at
//! the same moment on a cold machine is all it takes.
//!
//! *An agent that clears races every other agent.* Connect, fail, remove, bind: four steps, and
//! two agents can interleave at every one of them. Both clear, both bind, and the second unlinks
//! the first's socket out from under it. The first is then listening on an inode nothing can
//! reach — invisible, holding no clients, and never exiting.
//!
//! Both come from the same mistake: inferring liveness from a file. A socket that refuses
//! connections only *suggests* nobody is home. An advisory lock *is* the answer, because the
//! kernel releases it when the holding process dies — including under `SIGKILL` and a power cut,
//! where no destructor runs. So the stale case needs no heuristic, and the race needs no
//! ordering argument, because there is nothing to order: one `try_lock` decides it.
//!
//! # Why the file is never deleted
//!
//! Unlinking a lock file reintroduces the race in a worse form. A second process can be holding
//! the lock on the very inode being deleted, while a third creates a fresh file at the same path
//! and locks that — two holders, no conflict, and the kernel is right both times. So the file
//! stays. It is empty, it lives beside the socket, and leaving it there costs an inode.

use crate::Error;
use std::path::Path;

/// Proof that this process is the agent.
///
/// Held for as long as the agent runs. Dropping it — or the process ending by any route at all —
/// releases the lock and lets the next one in.
#[derive(Debug)]
pub struct Held {
    /// Never read. Open because dropping it releases the lock, which is the entire point.
    _file: std::fs::File,
}

/// Take the lock, or report who has it.
///
/// Creates the parent directory, `0700` on Unix, because on the socket platforms the directory's
/// permissions are the whole of the authentication: an agent any local process could talk to
/// would be an agent any local process could use.
pub fn take(path: &Path) -> Result<Held, Error> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| Error::at(parent, e))?;
        owner_only(parent);
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)
        .map_err(|e| Error::at(path, e))?;
    owner_only(path);
    match file.try_lock() {
        Ok(()) => Ok(Held { _file: file }),
        Err(std::fs::TryLockError::WouldBlock) => Err(Error::AlreadyRunning),
        Err(std::fs::TryLockError::Error(e)) => Err(Error::at(path, e)),
    }
}

/// Owner-only, where the platform has such a notion.
///
/// Best effort on purpose: a filesystem that cannot express it — a FAT-formatted volume, a
/// container bind mount — should not stop the agent starting. Where it works it is the
/// authentication; where it does not, the enclosing directory usually still is.
#[cfg(unix)]
pub(crate) fn owner_only(path: &Path) {
    use std::os::unix::fs::PermissionsExt as _;
    let mode = if path.is_dir() { 0o700 } else { 0o600 };
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
}

#[cfg(not(unix))]
pub(crate) fn owner_only(_path: &Path) {
    // Windows inherits the ACL of `%LOCALAPPDATA%`, which is already per-user.
}
