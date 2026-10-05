//! What `mailo watch` does as the session's member, beyond fetching: it keeps the unread count on
//! the launcher, with the window open or closed.
//!
//! The count is every account's unread inbox conversations (`ui::launcher::unread` over no
//! scope), the same question the window asks for the Space it shows. The window, where there is
//! a watch running, leaves the launcher to this thread (`ui::launch::native`), because a dock
//! forgets a count when its sender leaves the bus: a window that closed would take the badge with
//! it while the mail stayed unread.
//!
//! Polled rather than pushed. The store is shared by the watch, the window and the command line,
//! and what changes the count is any of them reading or archiving a message; one indexed
//! `COUNT` every [`EVERY`] is the cheapest way to see all of those without teaching each of them
//! to tell this thread. The count is said again every [`REPEAT`] even when it has not changed, so
//! a dock that started after mailo, or restarted, learns it within that time.

use crate::ui::launcher::{Badge, Unread, unread};
use mail_store::Store;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How often the count is read.
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

/// Read the count every `every` and tell `badge` when it is due, on a thread of its own, for as
/// long as the process runs. A read that fails (the database is busy, or gone) is skipped: the
/// next one says it.
pub fn keep_the_badge<S>(
    store: Arc<S>,
    badge: Arc<dyn Badge>,
    every: Duration,
) -> std::io::Result<std::thread::JoinHandle<()>>
where
    S: Store + Send + Sync + ?Sized + 'static,
{
    std::thread::Builder::new()
        .name("mailo-badge".to_owned())
        .spawn(move || {
            let mut beat = Beat::default();
            loop {
                if let Ok(count) = unread(
                    store.as_ref(),
                    &crate::ui::space::Scope::All,
                    chrono::Utc::now(),
                ) && beat.due(Instant::now(), count)
                {
                    badge.show(count);
                }
                std::thread::sleep(every);
            }
        })
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
