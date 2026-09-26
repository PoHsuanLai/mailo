//! Drag a row onto a place.
//!
//! Pointer events, not HTML5 drag and drop, which WebKitGTK delivers unreliably. A row arms a
//! drag on `pointerdown`; the window's own `pointermove` makes it live past a few pixels and
//! moves the ghost; a place that is entered while it is live becomes the target; `pointerup`
//! drops. Esc cancels, and so does a move with no button held (the pointer was released
//! somewhere that never told us).
//!
//! What a place accepts is decided by what the place *is* (Part C #25): a mailbox you can file
//! into, a label you can apply, or a saved search, which is read-only and refuses.

use super::{Motion, act_all, motion};
use crate::view::{Place, Shell, Source, place_filter};
use dioxus::prelude::*;
use ds::{DragGhost, Point, Px};
use mail_domain::*;
use mail_store::SqliteStore;
use std::sync::Arc;

/// How far the pointer must travel before a press on a row is a drag, in pixels.
const LIVE: f64 = 8.0;

/// A drag, from the press to the drop.
#[derive(Debug, Clone, PartialEq, Default)]
pub(in crate::ui) enum Drag {
    #[default]
    Idle,
    /// Pressed on a row; not moved far enough yet.
    Armed { thread: ThreadId, from: (f64, f64) },
    /// Following the pointer.
    Live {
        thread: ThreadId,
        subject: String,
        sender: String,
        at: (f64, f64),
        /// The place under the pointer, by its index in the sidebar.
        target: Option<usize>,
    },
}

/// What a sidebar place is, as a drop target.
pub(in crate::ui) fn view_kind(place: &Place) -> ViewKind {
    match &place.source {
        Source::Mail(Filter::HasLabel(label)) => ViewKind::PlaceLabel { label: *label },
        Source::Mail(Filter::InMailbox(role)) => ViewKind::Place { mailbox: *role },
        Source::Mail(filter) if *filter == place_filter(MailboxRole::Inbox) => ViewKind::Place {
            mailbox: MailboxRole::Inbox,
        },
        Source::Mail(_) | Source::Drafts => ViewKind::Query,
    }
}

/// The op a drop onto `kind` means, if it accepts one.
pub(in crate::ui) fn drop_op(kind: &ViewKind) -> Option<Op> {
    match kind {
        ViewKind::Place { mailbox } => match mailbox {
            MailboxRole::Inbox => Some(Op::Restore),
            MailboxRole::Archive => Some(Op::Archive),
            MailboxRole::Trash => Some(Op::Trash),
            MailboxRole::Spam => Some(Op::Spam),
            // Nothing is moved into Sent or Drafts by hand.
            MailboxRole::Sent | MailboxRole::Drafts => None,
        },
        ViewKind::PlaceLabel { label } => Some(Op::Label(*label, Membership::In)),
        ViewKind::Query => None,
    }
}

/// Whether a place takes drops at all. The sidebar lights only these while a drag is live.
pub(in crate::ui) fn accepts(place: &Place) -> bool {
    drop_op(&view_kind(place)).is_some()
}

/// A press on a row.
pub(in crate::ui) fn press(thread: ThreadId, at: (f64, f64)) {
    if let Some(mut state) = motion() {
        state.drag.set(Drag::Armed { thread, from: at });
    }
}

/// The pointer moved anywhere in the window. Cheap when nothing is being dragged.
pub(in crate::ui) fn moved(at: (f64, f64), held: bool) {
    let Some(mut state) = motion() else {
        return;
    };
    let drag = state.drag.peek().clone();
    match drag {
        Drag::Idle => {}
        _ if !held => state.drag.set(Drag::Idle),
        Drag::Armed { thread, from } => {
            if (at.0 - from.0).abs() + (at.1 - from.1).abs() < LIVE {
                return;
            }
            let store = consume_context::<Arc<SqliteStore>>();
            let Ok(loaded) = mail_store::Store::thread(store.as_ref(), thread) else {
                state.drag.set(Drag::Idle);
                return;
            };
            let sender = loaded
                .summary
                .from
                .name
                .clone()
                .unwrap_or_else(|| loaded.summary.from.email.clone());
            super::super::hover::dismiss();
            state.drag.set(Drag::Live {
                thread,
                subject: loaded.summary.subject,
                sender,
                at,
                target: None,
            });
        }
        Drag::Live { .. } => {
            if let Drag::Live { at: now, .. } = &mut *state.drag.write() {
                *now = at;
            }
        }
    }
}

/// The pointer entered or left the place at `index` in the sidebar.
pub(in crate::ui) fn over(index: Option<usize>, leaving: usize) {
    let Some(mut state) = motion() else {
        return;
    };
    if !matches!(*state.drag.peek(), Drag::Live { .. }) {
        return;
    }
    if let Drag::Live { target, .. } = &mut *state.drag.write() {
        match index {
            Some(index) => *target = Some(index),
            None if *target == Some(leaving) => *target = None,
            None => {}
        }
    }
}

/// The pointer was released anywhere in the window: drop onto the target, or
/// forget an armed press.
pub(in crate::ui) fn release(shell: Signal<Shell>, revision: Signal<u64>) {
    let Some(mut state) = motion() else {
        return;
    };
    let drag = std::mem::take(&mut *state.drag.write());
    let Drag::Live {
        thread,
        target: Some(index),
        ..
    } = drag
    else {
        return;
    };
    drop_on(shell, revision, thread, index);
}

fn drop_on(shell: Signal<Shell>, revision: Signal<u64>, thread: ThreadId, index: usize) {
    let Some(place) = shell.peek().places.get(index).cloned() else {
        return;
    };
    let Some(op) = drop_op(&view_kind(&place)) else {
        return;
    };
    let store = consume_context::<Arc<SqliteStore>>();
    // A picked row carries the whole selection with it, as one gesture.
    let ops = super::super::picks::with_selection(shell, thread)
        .into_iter()
        .map(|thread| (thread, op.clone()))
        .collect();
    act_all(&store, shell, revision, ops);
}

/// Esc: drop nothing. Returns whether a drag was live or armed.
pub(in crate::ui) fn cancel(mut state: Motion) -> bool {
    let was = !matches!(*state.drag.peek(), Drag::Idle);
    if was {
        state.drag.set(Drag::Idle);
    }
    was
}

/// The row under the pointer while it is dragged: tilted, lifted, and not a hit target.
#[component]
pub(in crate::ui) fn Ghost() -> Element {
    let Some(state) = motion() else {
        return rsx! {};
    };
    let Drag::Live {
        subject,
        sender,
        at,
        ..
    } = state.drag.read().clone()
    else {
        return rsx! {};
    };
    // quire's ghost places itself up and to the left of the pointer.
    let at = Point {
        x: Px(at.0 as f32),
        y: Px(at.1 as f32),
    };
    rsx! {
        DragGhost { title: subject, sub: sender, at }
    }
}
