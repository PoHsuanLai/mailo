//! Motion keyed to state.
//!
//! Every animation here starts because something the window already knows became true, never
//! because a timer stands in for it, and every one is quire's, timed by quire's motion clock
//! (`ds::prelude::settle`), never by an `animationend` event (coherence rule 4):
//!
//! - a row leaves because an op was applied and the row no longer belongs to the list: quire's
//!   `List` plays the exit for a key that stops being listed and closes the gap;
//! - the undo toast is up because an op was applied, and it names it (quire's `ToastHost`).
//!
//! **The store write happens first and at once.** Only the row's unmount waits.

pub(super) mod drag;
mod toast;

pub(super) use drag::Ghost;
pub(super) use toast::Toast;

use super::ops::{perform, resolve, take_back};
use crate::undo::{Undo, UndoHandle};
use crate::view::Shell;
use dioxus::prelude::*;
use ds::stack::toast_hub::{ToastAction, ToastHub, UndoToken};
use mail_domain::*;
use mail_store::SqliteStore;
#[cfg(test)]
use mail_store::Store;

/// What the toast says, and which op it belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Said {
    pub text: String,
    pub serial: u64,
    pub follow: Follow,
}

/// What the toast offers beside its words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) enum Follow {
    /// The tab that takes back the op the stack holds under this handle.
    Undo(UndoHandle),
    /// After leaving a list: archive what it already sent, by its sender's address.
    ArchiveFrom { sender: String, list: String },
    /// Nothing to take back: an unsubscribe, once made, is the list's.
    Nothing,
}

/// quire's toast host, and what its undo does: made by a component that lives as long as the
/// window, because the handler belongs to the scope that made it and must outlive the toast.
#[derive(Clone, Copy)]
pub(super) struct Toasts {
    pub hub: ToastHub,
    pub on_undo: EventHandler<UndoToken>,
    /// What the "Archive All" button of a leave-a-list toast does: it reads the list it offered
    /// from [`Motion::toast`] when pressed.
    pub on_archive: EventHandler<()>,
}

/// The motion state the window shares.
#[derive(Clone, Copy)]
pub(super) struct Motion {
    /// The follow-up a toast on screen offers that is not an undo (archive what a list already
    /// sent), kept here for its button's handler to read. The toast itself is quire's, through
    /// [`Motion::toasts`].
    pub toast: Signal<Option<Said>>,
    /// The window root's toast host and the handler its undo calls, once the list has mounted
    /// under the root. Not reactive: only a toast being said reads it.
    pub toasts: CopyValue<Option<Toasts>>,
    /// The place a hovered Archive or Snooze button would send the row to.
    pub dest: Signal<Option<&'static str>>,
    pub drag: Signal<drag::Drag>,
    /// The ids the list drew last, in order. Not reactive: only an op reads it.
    pub order: CopyValue<Vec<ThreadId>>,
}

/// Make the motion state for the window. Called once, from `App`.
pub(super) fn use_motion() -> Motion {
    use_context_provider(|| Motion {
        toast: Signal::new(None),
        toasts: CopyValue::new(None),
        dest: Signal::new(None),
        drag: Signal::new(drag::Drag::Idle),
        order: CopyValue::new(Vec::new()),
    })
}

/// The window's motion state, when there is a window around the caller.
pub(super) fn motion() -> Option<Motion> {
    try_consume_context::<Motion>()
}

/// Apply what a button means, and let the window show it.
pub(super) fn act_kind(
    store: &SqliteStore,
    shell: Signal<Shell>,
    revision: Signal<u64>,
    thread: ThreadId,
    kind: OpKind,
) -> bool {
    resolve(store, thread, kind).is_some_and(|op| act(store, shell, revision, thread, op))
}

/// Apply `op` to `thread` now, keep its undo, and start whatever motion it means.
pub(super) fn act(
    store: &SqliteStore,
    mut shell: Signal<Shell>,
    mut revision: Signal<u64>,
    thread: ThreadId,
    op: Op,
) -> bool {
    let Some(undo) = perform(store, thread, op.clone()) else {
        return false;
    };
    let said = undo.said.clone();
    let handle = shell.write().undo.push(undo);
    revision += 1;
    if let Some(mut motion) = motion() {
        motion.dest.set(None);
        motion.say(said, Follow::Undo(handle));
    }
    true
}

/// Put up the toast for something that is not an op on one row.
pub(in crate::ui) fn tell(text: String, follow: Follow) {
    if let Some(motion) = motion() {
        motion.say(text, follow);
    }
}

/// [`tell`], through motion state looked up earlier: for a task that outlives the component
/// that started it, where the lookup would no longer find the window's.
pub(in crate::ui) fn tell_through(motion: Option<Motion>, text: String) {
    if let Some(motion) = motion {
        motion.say(text, Follow::Nothing);
    }
}

/// Take back the newest op: Ctrl Z.
pub(super) fn undo_last(
    store: &SqliteStore,
    mut shell: Signal<Shell>,
    revision: Signal<u64>,
) -> bool {
    let Some(entry) = shell.write().undo.pop() else {
        return false;
    };
    restore(store, shell, revision, motion(), entry)
}

/// Take back the op the toast named, by the handle its undo carried: the toast's tab.
pub(super) fn undo_by(
    store: &SqliteStore,
    mut shell: Signal<Shell>,
    revision: Signal<u64>,
    motion: Option<Motion>,
    handle: UndoHandle,
) -> bool {
    let Some(entry) = shell.write().undo.take(handle) else {
        return false;
    };
    restore(store, shell, revision, motion, entry)
}

/// Put `entry` back, and take down the toast that offered it. Refused, it goes back on the
/// stack.
fn restore(
    store: &SqliteStore,
    mut shell: Signal<Shell>,
    mut revision: Signal<u64>,
    motion: Option<Motion>,
    entry: Undo,
) -> bool {
    if !take_back(store, &entry) {
        shell.write().undo.push(entry);
        return false;
    }
    revision += 1;
    // A row listed again while it leaves stays where it was (quire's `List`), and one whose
    // exit has settled enters again: the list is told nothing.
    if let Some(mut motion) = motion {
        motion.toast.set(None);
        if let Some(toasts) = *motion.toasts.peek() {
            toasts.hub.hide();
        }
    }
    true
}

/// The keys motion owns: Esc drops a drag, Ctrl Z undoes, and the hover card takes Space and
/// Esc. Returns whether the key was handled. Never while typing: Ctrl Z in a field is the
/// field's own.
pub(super) fn key(
    name: &str,
    ctrl: bool,
    typing: bool,
    shell: Signal<Shell>,
    revision: Signal<u64>,
) -> bool {
    if name == "Escape" && motion().is_some_and(drag::cancel) {
        return true;
    }
    if typing {
        return false;
    }
    if ctrl && (name == "z" || name == "Z") {
        let store = consume_context::<std::sync::Arc<SqliteStore>>();
        undo_last(&store, shell, revision);
        return true;
    }
    super::hover::key(name, shell)
}

impl Motion {
    /// Put up the toast. The next op replaces it; otherwise it leaves on its own. All of them are
    /// quire's: an undo, plain words, or a button of the follow-up's own ("Archive All").
    fn say(mut self, text: String, follow: Follow) {
        let toasts = *self.toasts.peek();
        match (follow, toasts) {
            (Follow::Undo(handle), Some(toasts)) => {
                self.toast.set(None);
                toasts
                    .hub
                    .push_undoable(text, UndoToken(handle.0), toasts.on_undo);
            }
            (Follow::Nothing, Some(toasts)) => {
                self.toast.set(None);
                toasts.hub.push(text, None);
            }
            (Follow::ArchiveFrom { sender, list }, toasts) => {
                let serial = self.toast.peek().as_ref().map_or(0, |said| said.serial) + 1;
                self.toast.set(Some(Said {
                    text: text.clone(),
                    serial,
                    follow: Follow::ArchiveFrom { sender, list },
                }));
                if let Some(toasts) = toasts {
                    toasts
                        .hub
                        .push_action(text, ToastAction::new("Archive All"), toasts.on_archive);
                }
            }
            // No host yet (a window still mounting): the words are kept, as before.
            (follow @ (Follow::Undo(_) | Follow::Nothing), None) => {
                let serial = self.toast.peek().as_ref().map_or(0, |said| said.serial) + 1;
                self.toast.set(Some(Said {
                    text,
                    serial,
                    follow,
                }));
            }
        }
    }
}

/// Whether `thread`, as the store now holds it, is still in the list `shell` shows.
///
/// A search is global, so a row there stays whatever happened to it; a place is its filter,
/// asked of the thread with the folders the server holds its messages in, so a folder's place
/// keeps a row that was starred and lets go of one whose mail has left the folder. What the
/// list draws is the store's own answer to that query; this asks it of one thread, for the
/// tests that pin the two to each other.
#[cfg(test)]
pub(in crate::ui) fn belongs(
    store: &SqliteStore,
    shell: &Shell,
    thread: ThreadId,
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    if !shell.search.trim().is_empty() {
        return true;
    }
    let Ok(after) = store.thread(thread) else {
        return false;
    };
    let folders: Vec<Placed> = after
        .messages
        .iter()
        .filter_map(|message| store.placed(*message).ok())
        .flatten()
        .collect();
    shell.query(1).filter.fit(&MatchCtx {
        summary: &after.summary,
        corpus: None,
        folders: &folders,
        now,
    })
}

#[cfg(test)]
mod tests;
