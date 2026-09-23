//! Hover previews: wait for intent, stay warm, never mark read, never fetch.
//!
//! The rules are [`crate::hover::Timer`]'s; this is the window's half. A hook (a row, a sender's
//! name, a time, a pinned tile, a Today tab) reports the pointer, the timer decides, and one card
//! is drawn for whatever the timer says is open. The one piece of state the stylesheet sees is
//! `warm`, which lets a strip button's result label show at once while previews are scrubbing.
//!
//! **Nothing here writes.** A card reads what the store already holds — the thread row, the
//! messages' stored text, the sender history — and never marks anything read, never loads a
//! remote image and never downloads a body. A POP3 `RETR` marks the message read on the
//! server, which is why a body that is not here says "not downloaded" rather than going to get it.

mod cards;
mod link;
mod sender;

pub(super) use cards::{HoverLayer, Site};
pub(super) use link::{LinkPill, link_out, link_over};

use crate::hover::{At, Timer};
use crate::trust::Destination;
use dioxus::prelude::*;
use mail_domain::ThreadId;
use std::time::Instant;

/// Where a card can be asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Hook {
    /// A row: the thread card, beside the list.
    Thread(ThreadId),
    /// The sender's name in a row.
    Sender(ThreadId),
    /// The time in a row.
    Time(ThreadId),
    /// A pinned person or search in the sidebar, by position.
    Pin(usize),
    /// A Today tab.
    Today(ThreadId),
}

/// The hover state the window shares. Signals, so it is `Copy` and every hook holds it.
#[derive(Clone, Copy)]
pub(super) struct Hover {
    timer: Signal<Timer<Hook>>,
    /// The last hook entered, and where its element's top-left corner is in the window.
    anchor: Signal<Option<(Hook, (f64, f64))>>,
    /// Whether a card closed less than a moment ago. The one signal the stylesheet sees.
    pub warm: Signal<bool>,
    /// The link under the pointer in the reader, if any. Instant: no timer.
    pub link: Signal<Option<Destination>>,
    epoch: CopyValue<Instant>,
    /// When the sleeping task will next wake, so a burst of events does not start a burst of
    /// sleepers.
    sleeper: CopyValue<Option<At>>,
    /// The scope that owns all of this, where the timer's sleeper runs: a row that unmounts
    /// must not take the clock with it, and nothing outside the owner may hold its values.
    owner: ScopeId,
}

/// Make the hover state for the window. Called once, from `App`.
pub(super) fn use_hover() -> Hover {
    use_context_provider(|| Hover {
        timer: Signal::new(Timer::default()),
        anchor: Signal::new(None),
        warm: Signal::new(false),
        link: Signal::new(None),
        epoch: CopyValue::new(Instant::now()),
        sleeper: CopyValue::new(None),
        owner: dioxus::core::current_scope_id(),
    })
}

/// The window's hover state, when there is a window around the caller.
pub(super) fn hover() -> Option<Hover> {
    try_consume_context::<Hover>()
}

/// The top-left corner of the element a pointer event landed on, in window coordinates.
pub(super) fn corner(event: &Event<PointerData>) -> (f64, f64) {
    let client = event.client_coordinates();
    let offset = event.element_coordinates();
    (client.x - offset.x, client.y - offset.y)
}

impl Hover {
    fn now(&self) -> At {
        let millis = self.epoch.read().elapsed().as_millis();
        At(u64::try_from(millis).unwrap_or(u64::MAX))
    }

    /// Replace the timer with `next` when it differs, so a pointer crossing a row's children
    /// does not redraw anything.
    fn update(mut self, change: impl FnOnce(&mut Timer<Hook>, At)) {
        let now = self.now();
        let mut next = self.timer.peek().clone();
        change(&mut next, now);
        if next != *self.timer.peek() {
            self.timer.set(next);
        }
        let warm = self.timer.peek().warm(now);
        if warm != *self.warm.peek() {
            self.warm.set(warm);
        }
        self.schedule();
    }

    /// The pointer came to rest on `hook`, whose element's corner is at `at`.
    pub(super) fn enter(mut self, hook: Hook, at: (f64, f64)) {
        if *self.anchor.peek() != Some((hook, at)) {
            self.anchor.set(Some((hook, at)));
        }
        self.update(|timer, now| timer.enter(hook, now));
    }

    /// The pointer left the hook.
    pub(super) fn leave(self) {
        self.update(|timer, now| timer.leave(now));
    }

    pub(super) fn enter_card(self) {
        self.update(|timer, _| timer.enter_card());
    }

    pub(super) fn leave_card(self) {
        self.update(|timer, now| timer.leave_card(now));
    }

    /// Close whatever is showing: a click, Esc, an action.
    pub(super) fn dismiss(self) {
        self.update(|timer, now| timer.dismiss(now));
    }

    /// The hook whose card is showing, and where it was entered.
    pub(super) fn open(&self) -> Option<(Hook, (f64, f64))> {
        let open = *self.timer.read().open()?;
        let (hook, at) = (*self.anchor.read())?;
        (hook == open).then_some((hook, at))
    }

    /// Sleep until the timer's next deadline, in the owning scope.
    fn schedule(mut self) {
        let now = self.now();
        let Some(due) = self.timer.peek().deadline(now) else {
            return;
        };
        if self
            .sleeper
            .peek()
            .is_some_and(|wakes| wakes >= now && wakes <= due)
        {
            return;
        }
        self.sleeper.set(Some(due));
        let wait = std::time::Duration::from_millis(due.0.saturating_sub(now.0));
        dioxus::core::Runtime::current().spawn(self.owner, async move {
            tokio::time::sleep(wait).await;
            if *self.sleeper.peek() == Some(due) {
                self.sleeper.set(None);
            }
            self.update(|timer, now| timer.tick(now));
        });
    }
}

/// Space opens the thread whose card is showing in a centred peek; Esc closes the card.
/// Returns whether the key was the hover's.
pub(super) fn key(name: &str, mut shell: Signal<crate::view::Shell>) -> bool {
    let Some(hover) = hover() else {
        return false;
    };
    let Some((hook, _)) = hover.open() else {
        return false;
    };
    match (name, hook) {
        (" ", Hook::Thread(id)) => {
            hover.dismiss();
            let mut write = shell.write();
            write.peek = crate::view::Peek::Center;
            write.open(id);
            true
        }
        ("Escape", _) => {
            hover.dismiss();
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod render;
#[cfg(test)]
mod tests;
