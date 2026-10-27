//! When an unread conversation the reader shows stops being unread (`reading.mark_read`): as it
//! opens, once it has been shown for a while, or never (Mark as Read alone does it).
//!
//! The wait is the reader's to keep and to let go of: one conversation at most is waiting
//! ([`Pending`]), and showing another, or closing the reader, lets it go, so a conversation
//! passed over on the way to another is left as it was. Marking it, when it comes, is the
//! same as opening one always did: on the server too, where the server keeps flags, with no
//! toast and no undo.

use crate::settings::{MarkRead, MarkReadDelay};
use mail_domain::ThreadId;
use std::time::{Duration, Instant};

/// What showing a conversation asks of the reader.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Mark {
    /// Mark it read now.
    Now,
    /// Mark it read if it is still shown after this long.
    After(Duration),
    /// Leave it as it is.
    Leave,
}

/// What `setting`, waiting `delay` where it waits, asks of showing a conversation that is
/// `unread` or not.
pub(super) fn on_show(setting: MarkRead, delay: MarkReadDelay, unread: bool) -> Mark {
    match (unread, setting) {
        (false, _) | (true, MarkRead::Never) => Mark::Leave,
        (true, MarkRead::OnOpen) => Mark::Now,
        (true, MarkRead::AfterDelay) => Mark::After(delay.duration()),
    }
}

/// The one conversation waiting to be marked read, and when it is due.
#[derive(Debug, Default)]
pub(super) struct Pending(Option<(ThreadId, Instant)>);

impl Pending {
    /// `thread` is shown from `now` and is due after `wait`; whatever waited before is let go.
    pub(super) fn shown(&mut self, thread: ThreadId, now: Instant, wait: Duration) {
        self.0 = Some((thread, now + wait));
    }

    /// The reader moved off what it showed, to something that does not wait, or closed.
    pub(super) fn left(&mut self) {
        self.0 = None;
    }

    /// Whether `thread` is to be marked read at `now`: only while it is the one waiting and its
    /// time has come, and then once.
    pub(super) fn due(&mut self, thread: ThreadId, now: Instant) -> bool {
        match self.0 {
            Some((waiting, at)) if waiting == thread && now >= at => {
                self.0 = None;
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn thread(n: u128) -> ThreadId {
        ThreadId::from_uuid(uuid::Uuid::from_u128(n))
    }

    #[test]
    fn what_showing_a_conversation_asks_by_the_setting() {
        // (setting, delay, unread, asked)
        let cases = [
            (MarkRead::OnOpen, MarkReadDelay::Two, true, Mark::Now),
            (MarkRead::OnOpen, MarkReadDelay::Two, false, Mark::Leave),
            (
                MarkRead::AfterDelay,
                MarkReadDelay::Two,
                true,
                Mark::After(Duration::from_secs(2)),
            ),
            (
                MarkRead::AfterDelay,
                MarkReadDelay::Ten,
                true,
                Mark::After(Duration::from_secs(10)),
            ),
            (
                MarkRead::AfterDelay,
                MarkReadDelay::Five,
                false,
                Mark::Leave,
            ),
            (MarkRead::Never, MarkReadDelay::Two, true, Mark::Leave),
        ];
        for (setting, delay, unread, want) in cases {
            assert_eq!(
                on_show(setting, delay, unread),
                want,
                "{setting:?} {delay:?} unread {unread}"
            );
        }
    }

    /// On a clock the test moves: due once its wait has passed, not before and not twice.
    #[test]
    fn a_shown_conversation_is_due_once_its_wait_has_passed() {
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        let mut pending = Pending::default();
        pending.shown(thread(1), at(0), Duration::from_secs(2));
        assert!(!pending.due(thread(1), at(1_999)), "before its time");
        assert!(pending.due(thread(1), at(2_000)), "at its time");
        assert!(!pending.due(thread(1), at(5_000)), "a second time");
    }

    /// Moving on before the wait is over lets the first go: it is never due, and the one shown
    /// now waits from when it was shown.
    #[test]
    fn moving_on_before_the_wait_is_over_leaves_the_first_unread() {
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        let wait = Duration::from_secs(5);
        let mut pending = Pending::default();
        pending.shown(thread(1), at(0), wait);
        pending.shown(thread(2), at(1_000), wait);
        assert!(!pending.due(thread(1), at(5_000)), "the one passed over");
        assert!(!pending.due(thread(2), at(5_000)), "the next, early");
        assert!(pending.due(thread(2), at(6_000)), "the next, on time");

        // Closing the reader, or showing one that does not wait, lets it go too.
        pending.shown(thread(3), at(7_000), wait);
        pending.left();
        assert!(!pending.due(thread(3), at(60_000)), "after the reader left");
    }
}
