//! Who schedules fetching: this process, or the `mailo watch` that runs beside it.
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

use crate::fetch::{Event, Link, Trigger};

/// Who decides when mail is fetched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Schedule {
    /// This process: timers, polls and pushes are its own.
    Window,
    /// A `mailo watch`: the window fetches only when a person asks.
    Watch,
}

/// What the scheduler does with an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Give it to the link.
    Run,
    /// A timer's request: ask again one interval later, and give the link nothing now.
    Defer,
    /// The server's request: the watch heard it too.
    Drop,
}

/// What to do with `event` for `link`, given who schedules.
pub fn verdict(schedule: Schedule, link: &Link, event: &Event) -> Verdict {
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
