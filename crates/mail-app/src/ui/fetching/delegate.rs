//! Who schedules fetching: the window, or the `mailo watch` that runs beside it.
//!
//! A watch (`mail_core::ipc::watching`) is the session's sync engine: it holds IDLE where the
//! server offers it, polls where it does not, and drains the outbox. A window that also polled
//! and listened would be a second writer on the same SQLite file, and two of them sometimes end
//! a pass with `database is locked`. So where a watch runs the window leaves scheduled fetching
//! to it and keeps the requests a person makes: Sync, a folder opened on demand, a send that
//! came due, signing in again.
//!
//! What the window drops is what a timer or the server would have started: a poll, a push and a
//! wake. A poll or wake is not forgotten: it is asked again one interval later, so that a watch
//! that stops hands fetching back to the window within an interval. And an account that has never
//! fetched anything is still the window's to fetch, because the watch reads its accounts when it
//! starts and does not know one made since.
//!
//! The mail the watch stores is not the window's doing: [`super::external`] tells the window that
//! something it did not write has landed.

use mail_core::fetch::{Event, Link, Trigger};
use std::sync::Arc;

/// Who decides when mail is fetched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum Schedule {
    /// This window: timers, polls and pushes are its own.
    Window,
    /// A `mailo watch`: the window fetches only when a person asks.
    Watch,
}

/// What reports who schedules. The real one unless a test provided its own, as
/// [`super::Passer`] is.
#[derive(Clone)]
pub(in crate::ui) struct Delegate(pub Arc<dyn Fn() -> Schedule + Send + Sync>);

impl Delegate {
    /// Asks the watch's lock ([`mail_core::ipc::watching::running`]), every time it is asked: a
    /// watch may start or stop while the window is open.
    #[cfg(not(test))]
    pub(in crate::ui) fn server() -> Self {
        Self(Arc::new(|| {
            if mail_core::ipc::watching::running() {
                Schedule::Watch
            } else {
                Schedule::Window
            }
        }))
    }

    /// Not in a test build: the person's real watch, if there is one, must not change what a
    /// test sees.
    #[cfg(test)]
    pub(in crate::ui) fn server() -> Self {
        Self(Arc::new(|| Schedule::Window))
    }

    /// Who schedules now.
    pub(in crate::ui) fn schedule(&self) -> Schedule {
        (self.0)()
    }
}

/// What the runner does with an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Verdict {
    /// Give it to the link.
    Run,
    /// A timer's request: ask again one interval later, and give the link nothing now.
    Defer,
    /// The server's request: the watch heard it too.
    Drop,
}

/// What to do with `event` for `link`, given who schedules.
pub(super) fn verdict(schedule: Schedule, link: &Link, event: &Event) -> Verdict {
    // A pass in progress ignores a start anyway, and a link with nothing fetched is a new
    // account that the watch does not know.
    if schedule == Schedule::Window || link.is_busy() || matches!(link, Link::Fresh) {
        return Verdict::Run;
    }
    match event {
        Event::Start(Trigger::Poll) | Event::Tick => Verdict::Defer,
        Event::Start(Trigger::Push) => Verdict::Drop,
        _ => Verdict::Run,
    }
}

#[cfg(test)]
mod tests;
