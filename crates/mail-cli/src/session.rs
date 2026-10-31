//! What `mailo watch` does as the session's member, beyond fetching: it keeps the unread count on
//! the launcher, with the window open or closed.
//!
//! The count is every account's unread conversations (`launcher::unread` over no scope, in the
//! places `notifications.badge` says), the same question the window asks for the Space it shows.
//! The window, where there is a watch running, leaves the launcher to this thread
//! (`ui::launch::native`), because a dock forgets a count when its sender leaves the bus: a
//! window that closed would take the badge with it while the mail stayed unread.
//!
//! Read when the store is written to. The store is shared by the watch, the window and the command
//! line, and what changes the count is any of them reading or archiving a message, each of which
//! is a commit to the database's file or the `-wal` beside it; the system says when one is
//! written to (`mail_client::on_write`, what the client hears another process's commits by), so the count
//! is read then, at most once every [`EVERY`], and not at all while nothing is written. Without
//! that word (a store in memory, or a system that will not watch the file) it is read every
//! [`EVERY`]. The count is read and said again every [`REPEAT`] even when nothing was written,
//! so a dock that started after mailo, or restarted, learns it within that time.

use chrono::{DateTime, Utc};
use mail_core::Store;
use mail_logic::scope::Scope;
use mail_os::launcher::{Badge, Unread};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How often the count is read at most, and always when the store's file cannot be watched.
pub const EVERY: Duration = Duration::from_secs(2);

/// How long a count that has not changed goes unsaid.
pub const REPEAT: Duration = Duration::from_secs(60);

/// Whether a count read now is worth saying.
#[derive(Debug, Default)]
pub struct Beat {
    said: Option<(Unread, Instant)>,
}

impl Beat {
    /// Yes when it differs from the last said, or the last said is [`REPEAT`] old. Remembers that
    /// it was said when it answers yes.
    pub fn due(&mut self, now: Instant, count: Unread) -> bool {
        let due = match self.said {
            Some((last, at)) => last != count || now.duration_since(at) >= REPEAT,
            None => true,
        };
        if due {
            self.said = Some((count, now));
        }
        due
    }
}

/// Keep this platform's launcher showing the unread count, for as long as the process runs. `None`
/// when the platform has no launcher to tell. For `mailo watch`, which has no window of its own.
/// `file` is the database the store is in, when it is in one.
///
/// `None` on macOS too: the Dock tile is mailo.app's own, which no other process can badge, and
/// the app keeps its count itself while it runs, with a window open or not (`ui::launch::native`).
pub fn keep_the_launchers_badge<S>(
    store: Arc<S>,
    file: Option<PathBuf>,
) -> Option<std::io::Result<()>>
where
    S: Store + Send + Sync + ?Sized + 'static,
{
    if cfg!(target_os = "macos") {
        return None;
    }
    let launcher = mail_os::launcher::platform()?;
    Some(keep_the_badge(store, file, launcher.0, EVERY).map(|_| ()))
}

/// Read the count when `file` (the store's database) has been written to, at most every `every`,
/// and tell `badge` when it is due, on a thread of its own, for as long as the process runs. With
/// no file to watch, read it every `every`. A read that fails (the database is busy, or gone) is
/// skipped: the next one says it.
pub fn keep_the_badge<S>(
    store: Arc<S>,
    file: Option<PathBuf>,
    badge: Arc<dyn Badge>,
    every: Duration,
) -> std::io::Result<std::thread::JoinHandle<()>>
where
    S: Store + Send + Sync + ?Sized + 'static,
{
    std::thread::Builder::new()
        .name("mailo-badge".to_owned())
        .spawn(move || {
            let (tx, written) = std::sync::mpsc::channel();
            // Gone only with this thread, which is not a write to say anything about.
            let watching = file.as_deref().and_then(|file| {
                mail_client::on_write(file, move || {
                    let _ = tx.send(());
                })
            });
            let mut beat = Beat::default();
            loop {
                if let Ok(count) = unread(
                    store.as_ref(),
                    &Scope::All,
                    chrono::Utc::now(),
                    mail_ui::settings::badge_places(),
                ) && beat.due(Instant::now(), count)
                {
                    badge.show(count);
                }
                std::thread::sleep(every);
                if watching.is_some() {
                    // A write heard while sleeping is here already. Writes heard together are
                    // one read; a watch that has ended answers at once, every `every`.
                    if written.recv_timeout(REPEAT.saturating_sub(every)).is_ok() {
                        while written.try_recv().is_ok() {}
                    }
                }
            }
        })
}

/// [`keep_the_badge`] for a badge already found, over the store the app opened: what the macOS
/// app keeps its Dock tile with.
pub fn keep_the_badge_over(
    store: Arc<mail_core::SqliteStore>,
    file: Option<PathBuf>,
    badge: Arc<dyn Badge>,
) -> std::io::Result<()> {
    keep_the_badge(store, file, badge, EVERY).map(|_| ())
}

/// The filter the launcher counts with: [`mail_os::launcher::filter_over`] over the mailboxes
/// the accounts' servers list as followed, when `places` counts them.
pub fn filter<S: Store + ?Sized>(
    store: &S,
    scope: &Scope,
    places: mail_core::notify::Places,
) -> Result<mail_domain::Filter, mail_core::StoreError> {
    let followed = match places {
        mail_core::notify::Places::Inbox => Vec::new(),
        mail_core::notify::Places::Followed => store.followed_folders()?,
    };
    Ok(mail_os::launcher::filter_over(&followed, scope, places))
}

/// Count them, as `places` says. One indexed query (`Store::count`), made off the thread that
/// draws.
pub fn unread<S: Store + ?Sized>(
    store: &S,
    scope: &Scope,
    now: DateTime<Utc>,
    places: mail_core::notify::Places,
) -> Result<Unread, mail_core::StoreError> {
    store.count(&filter(store, scope, places)?, now).map(Unread)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_count_is_said_when_it_changes_and_now_and_then_when_it_does_not() {
        let t0 = Instant::now();
        let mut beat = Beat::default();
        let after = |s: u64| t0 + Duration::from_secs(s);
        let steps: &[(&str, u64, u64, bool)] = &[
            ("the first count", 0, 3, true),
            ("the same count soon after", 2, 3, false),
            ("a change", 4, 4, true),
            ("the same, still within the repeat", 62, 4, false),
            ("the same, past the repeat", 64, 4, true),
            ("down to none", 66, 0, true),
        ];
        for &(name, at, count, expect) in steps {
            assert_eq!(beat.due(after(at), Unread(count)), expect, "{name}");
        }
    }
}
