//! The window hearing about what it did not write.
//!
//! A `mailo watch` stores the mail it fetches. It is another process, and the window's lists are
//! read again only when its revision moves. So the window looks at whether the store has been
//! committed to by another connection ([`SqliteStore::data_version`]), and moves the revision
//! when it has.
//!
//! # Heard, and looking
//!
//! A commit is a write to the database's file or the `-wal` beside it, and the system says when
//! one is written to, so the window looks then: whoever committed, a watch, the daemon or `mailo`
//! on the command line, and whether or not it is still running. The data version, not the event,
//! decides whether anything moved, so the window's own commits (which are writes to the same
//! files) and a pass that stored nothing move nothing, and one commit is never counted twice.
//!
//! The directory is watched rather than the files: SQLite removes the `-wal` when the last
//! connection closes and makes it again with the next, and a watch on the file would go with it.
//!
//! The looking goes on, every [`STEADY`], for whatever the system does not say (a platform that
//! reports a write late, or not at all when it does not grow the file), and every [`LOOK`] when
//! the file could not be watched. Heard while the writer is busy, the look cannot read the data
//! version; it looks again every [`OWED`] until it can, rather than leaving what it heard to the
//! next [`STEADY`].
//!
//! (Until 2026-10 the window subscribed at the watch's door and was told as each pass ended,
//! which held a connection and a thread in the watch for each window; `mail_core::ipc::door`
//! says what that cost.)

use dioxus::prelude::*;
use mail_core::SqliteStore;
use notify::{EventKind, RecursiveMode, Watcher as _};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

/// How often the window looks when the store's file could not be watched.
#[cfg(not(test))]
const LOOK: Duration = Duration::from_secs(2);
#[cfg(test)]
pub(super) const LOOK: Duration = Duration::from_millis(30);

/// How often the window looks while the system says when the store is written to.
#[cfg(not(test))]
const STEADY: Duration = Duration::from_secs(15);
/// Long enough in a test that a change seen within a few looks was heard, not looked for.
#[cfg(test)]
const STEADY: Duration = Duration::from_secs(60);

/// How soon to look again after hearing a write while the writer was busy.
const OWED: Duration = Duration::from_millis(50);

/// Move `revision` whenever another connection has committed to `store`. Once, from
/// [`super::use_fetching`].
pub(super) fn use_external_changes(mut revision: Signal<u64>, store: Arc<SqliteStore>) {
    let _looking = use_future(move || {
        let store = store.clone();
        async move {
            let mut written = store.file().and_then(watch);
            let mut seen = store.data_version();
            let mut owed = false;
            loop {
                let every = match (owed, &written) {
                    (true, _) => OWED,
                    (false, Some(_)) => STEADY,
                    (false, None) => LOOK,
                };
                let heard = next(&mut written, every).await;
                let now = store.data_version();
                // A busy writer answers nothing this time. Having heard a write, look again soon
                // rather than at the next look; otherwise the next look will do.
                owed = now.is_none() && (owed || heard);
                if now.is_some() && now != seen {
                    seen = now;
                    revision += 1;
                }
            }
        }
    });
}

/// The system's word that the store's file was written to: the watch, kept for as long as the
/// window looks, and what it says.
struct Written {
    _watcher: notify::RecommendedWatcher,
    said: UnboundedReceiver<()>,
}

/// Watch the database at `file`, and the `-wal` beside it, or `None` when this system will not.
fn watch(file: &Path) -> Option<Written> {
    let name = file.file_name()?.to_owned();
    let mut wal = name.clone();
    wal.push("-wal");
    let dir = match file.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir,
        _ => Path::new("."),
    };
    let (tx, said) = unbounded_channel();
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        let ours = match event {
            // Reading is not committing, and the window's own readers read all the time.
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
            // Gone only when the window is, which is not a write to say anything about.
            let _ = tx.send(());
        }
    })
    .ok()?;
    watcher.watch(dir, RecursiveMode::NonRecursive).ok()?;
    Some(Written {
        _watcher: watcher,
        said,
    })
}

/// Whether the store was written to before `every` passed. Writes heard together are one look.
async fn next(written: &mut Option<Written>, every: Duration) -> bool {
    let Some(heard) = written.as_mut() else {
        tokio::time::sleep(every).await;
        return false;
    };
    match tokio::time::timeout(every, heard.said.recv()).await {
        Ok(Some(())) => {
            while heard.said.try_recv().is_ok() {}
            true
        }
        Err(_) => false,
        // The watch has ended, so nothing will be said again: look once now, and from then on
        // as often as with no watch.
        Ok(None) => {
            *written = None;
            true
        }
    }
}

#[cfg(test)]
mod tests;
