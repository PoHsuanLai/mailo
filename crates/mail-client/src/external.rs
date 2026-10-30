//! Hearing what another process commits.
//!
//! A `mailo watch` stores the mail it fetches, and `mailo` on the command line files, labels and
//! deletes: other processes, whose commits go to the database's file without passing through
//! this client's store, so the store cannot say them ([`mail_store::changes`]). So the client
//! looks at whether another connection has committed ([`SqliteStore::data_version`]) and, when
//! one has, marks every value stale.
//!
//! # Heard, and looking
//!
//! A commit is a write to the database's file or the `-wal` beside it, and the system says when
//! one is written to, so the client looks then. The data version, not the event, decides whether
//! anything moved, so this process's own commits (which are writes to the same files) and a pass
//! that stored nothing move nothing, and one commit is never counted twice.
//!
//! The directory is watched rather than the files: SQLite removes the `-wal` when the last
//! connection closes and makes it again with the next, and a watch on the file would go with it.
//!
//! The looking goes on, every [`STEADY`], for whatever the system does not say (a platform that
//! reports a write late, or not at all when it does not grow the file), and every [`LOOK`] when
//! the file could not be watched. Heard while the writer is busy, the look cannot read the data
//! version; it looks again every [`OWED`] until it can. A write is heard as it reaches the file,
//! which can be before its commit is told to other connections, so a heard write that moved
//! nothing is looked at once more, [`OWED`] later.

use crate::client::Shared;
use mail_store::{SqliteStore, Topics};
use notify::{EventKind, RecursiveMode, Watcher as _};
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::thread::JoinHandle;
use std::time::Duration;

/// How often to look when the store's file could not be watched.
#[cfg(not(feature = "testing"))]
const LOOK: Duration = Duration::from_secs(2);
#[cfg(feature = "testing")]
const LOOK: Duration = Duration::from_millis(30);

/// How often to look while the system says when the store is written to.
const STEADY: Duration = Duration::from_secs(15);

/// How soon to look again after hearing a write that this look could not tell had committed.
const OWED: Duration = Duration::from_millis(50);

/// Start looking, on a thread of its own, or `None` for a store with no file.
pub(crate) fn start(shared: Arc<Shared>) -> Option<JoinHandle<()>> {
    let file = shared.store().file()?.to_owned();
    // Taken here, before the client is handed to anyone: a commit made once it is is one the
    // looking will see moved. Taken on the thread, it could miss one made before the thread ran.
    let seen = shared.store().data_version();
    let (tx, rx) = channel();
    *shared.external.lock() = Some(tx.clone());
    let watcher = on_write(&file, move || {
        // Gone only when the client is, which is not a write to say anything about.
        let _ = tx.send(());
    });
    std::thread::Builder::new()
        .name("mailo-external".to_owned())
        .spawn(move || {
            // Kept for as long as the looking goes on: dropped, it stops saying.
            let kept = watcher;
            look(&shared, &rx, kept.is_some(), seen);
        })
        .ok()
}

/// Wake the looking, so a client that is stopping is heard to.
pub(crate) fn wake(shared: &Shared) {
    if let Some(tx) = shared.external.lock().take() {
        // A look that has already ended has nothing to wake.
        let _ = tx.send(());
    }
}

fn look(shared: &Shared, written: &Receiver<()>, watched: bool, mut seen: Option<i64>) {
    let store: &SqliteStore = shared.store();
    let mut owed = false;
    while !shared.stopping() {
        let every = match (owed, watched) {
            (true, _) => OWED,
            (false, true) => STEADY,
            (false, false) => LOOK,
        };
        let heard = match written.recv_timeout(every) {
            Ok(()) => {
                // Writes heard together are one look.
                while written.try_recv().is_ok() {}
                true
            }
            Err(RecvTimeoutError::Timeout) => false,
            Err(RecvTimeoutError::Disconnected) => return,
        };
        if shared.stopping() {
            return;
        }
        let now = crate::client::survive("looking at the store", || store.data_version()).flatten();
        let moved = now.is_some() && now != seen;
        // Having heard a write, look again soon rather than at the next look when this look
        // could not tell: the writer was busy and answered nothing, or the write was heard
        // before it was committed — the commit itself is told to other connections through the
        // shared memory beside the file, which the system says nothing about.
        owed = match now {
            None => owed || heard,
            Some(_) => heard && !moved,
        };
        if moved {
            seen = now;
            crate::client::survive("marking what changed", || shared.invalidate(Topics::ALL));
        }
    }
}

/// Call `said` whenever the database at `file`, or the `-wal` beside it, is written to, for as
/// long as the watcher returned is kept; `None` when this system will not watch it.
pub fn on_write(
    file: &Path,
    said: impl Fn() + Send + 'static,
) -> Option<notify::RecommendedWatcher> {
    let name = file.file_name()?.to_owned();
    let mut wal = name.clone();
    wal.push("-wal");
    let dir = match file.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir,
        _ => Path::new("."),
    };
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        let ours = match event {
            // Reading is not committing, and this process's readers read all the time.
            Ok(event) => {
                !matches!(event.kind, EventKind::Access(_))
                    && event
                        .paths
                        .iter()
                        .any(|path| path.file_name().is_some_and(|it| it == name || it == wal))
            }
            // What could not be said (a queue that overflowed) may have been a commit.
            Err(_) => true,
        };
        if ours {
            said();
        }
    })
    .ok()?;
    watcher.watch(dir, RecursiveMode::NonRecursive).ok()?;
    Some(watcher)
}
