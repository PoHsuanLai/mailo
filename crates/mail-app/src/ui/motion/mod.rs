//! Motion keyed to state.
//!
//! Every animation here starts because something the window already knows became true, never
//! because a timer stands in for it (Part E, decision 4), and every one is timed by quire's
//! motion clock (`ds::settle`, through `ds::Roster` and `ds::MotionTimer`), never by the
//! webview's `animationend` (coherence rule 4):
//!
//! - a row leaves (`data-presence="leaving"`) because an op was applied and the row no longer
//!   belongs to the list; the list's roster keeps it drawn until its exit settles;
//! - the rows below it heal once it has gone, again the roster's doing;
//! - a place `gulp`s because an op landed there, and a label `chip-land`s because it was added;
//! - the undo toast is up because an op was applied, and it names it.
//!
//! **The store write happens first and at once.** Only the row's unmount waits.

pub(super) mod drag;
mod toast;

pub(super) use drag::Ghost;
pub(super) use toast::Toast;

use super::ops::{perform, resolve, take_back};
use crate::undo::{Undo, UndoHandle};
use crate::view::Shell;
use chrono::{DateTime, Utc};
use dioxus::prelude::*;
use ds::{Emphasis, Exit, MotionTimer, Roster, ToastHub, UndoToken};
use mail_domain::*;
use mail_store::{SqliteStore, Store};

/// A row that has left the list and is still being drawn while it goes. The roster knows a row
/// by its thread; an undo while the row is still leaving takes its exit back
/// (`Roster::stay`), so the row stays in place under the same key and nothing below heals.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Leaving {
    /// Its key in the roster.
    pub key: ThreadId,
    /// The row as it was drawn before the op.
    pub summary: ThreadSummary,
}

/// The list's roster and the timers of the motion an op starts, made by the list, which is
/// inside the window's quire root and so reads its motion level. Every one belongs to the list
/// and is dropped with it.
#[derive(Clone, Copy)]
pub(super) struct Clock {
    pub roster: Roster<ThreadId>,
    /// How long the place an op landed in gulps.
    pub gulp: MotionTimer,
    /// How long a label that was just added lands.
    pub landing: MotionTimer,
    /// What each timer does once it has settled.
    pub gulped: EventHandler<()>,
    pub landed: EventHandler<()>,
}

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
    /// After blocking a sender: take the block back, which forgets the rule it made. Not an
    /// entry on the undo stack, whose entries are patches to messages; a rule is not one.
    Unblock { rule: RuleId, sender: String },
    /// Nothing to take back: an unsubscribe, once made, is the list's.
    Nothing,
}

/// quire's toast host, and what its undo does: made by a component that lives as long as the
/// window, because the handler belongs to the scope that made it and must outlive the toast.
#[derive(Clone, Copy)]
pub(super) struct Toasts {
    pub hub: ToastHub,
    pub on_undo: EventHandler<UndoToken>,
}

/// The motion state the window shares.
#[derive(Clone, Copy)]
pub(super) struct Motion {
    pub leaving: Signal<Vec<Leaving>>,
    /// The place an op just landed in.
    pub gulp: Signal<Option<String>>,
    /// A label just added to a row.
    pub landing: Signal<Option<(ThreadId, LabelId)>>,
    /// A row an undo just brought back.
    pub returning: Signal<Option<ThreadId>>,
    /// The toast mailo still draws itself: one with a follow-up that is not an undo. Every
    /// other toast is quire's, through [`Motion::toasts`].
    pub toast: Signal<Option<Said>>,
    /// The window root's toast host and the handler its undo calls, once the list has mounted
    /// under the root. Not reactive: only a toast being said reads it.
    pub toasts: CopyValue<Option<Toasts>>,
    /// The place a hovered Archive or Snooze button would send the row to.
    pub dest: Signal<Option<&'static str>>,
    pub drag: Signal<drag::Drag>,
    /// The ids the list drew last, in order. Not reactive: only an op reads it.
    pub order: CopyValue<Vec<ThreadId>>,
    /// The list's roster and timers, once the list has mounted. Not reactive: only an op reads
    /// it.
    pub clock: CopyValue<Option<Clock>>,
    /// The scope that owns all of this. The toast's timeout runs there, so a row that unmounts
    /// does not cancel it.
    owner: ScopeId,
}

/// Make the motion state for the window. Called once, from `App`.
pub(super) fn use_motion() -> Motion {
    use_context_provider(|| Motion {
        leaving: Signal::new(Vec::new()),
        gulp: Signal::new(None),
        landing: Signal::new(None),
        returning: Signal::new(None),
        toast: Signal::new(None),
        toasts: CopyValue::new(None),
        dest: Signal::new(None),
        drag: Signal::new(drag::Drag::Idle),
        order: CopyValue::new(Vec::new()),
        clock: CopyValue::new(None),
        owner: dioxus::core::current_scope_id(),
    })
}

/// The window's motion state, when there is a window around the caller.
pub(super) fn motion() -> Option<Motion> {
    try_consume_context::<Motion>()
}

/// How long mailo's own toast stays. quire's holds its own.
const TOAST: std::time::Duration = std::time::Duration::from_secs(6);

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
    let before = store.thread(thread).ok().map(|loaded| loaded.summary);
    let Some(undo) = perform(store, thread, op.clone()) else {
        return false;
    };
    let said = undo.said.clone();
    let handle = shell.write().undo.push(undo);
    revision += 1;
    if let Some(motion) = motion() {
        motion.landed(store, shell, thread, &op, before);
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
    if let Some(mut motion) = motion {
        motion.toast.set(None);
        if let Some(toasts) = *motion.toasts.peek() {
            toasts.hub.hide();
        }
        if let Some(thread) = entry.thread {
            // Still leaving: its exit is taken back and it stays where it was, so the rows
            // below never heal. Once its exit has settled the roster no longer holds it, and
            // listed again it enters, as a row back from an undo.
            let was_leaving = motion.leaving.peek().iter().any(|row| row.key == thread);
            if was_leaving && let Some(clock) = *motion.clock.peek() {
                let _ = clock.roster.stay(thread);
            }
            motion.leaving.write().retain(|row| row.key != thread);
            motion.returning.set(Some(thread));
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

/// The row's exit, when `op` takes a row out of a list: snooze curls away, everything else
/// folds, as mailo's rows always have.
fn exit(op: &Op) -> Option<Exit> {
    match op {
        // Filed into a folder is out of the inbox, the way archiving is.
        Op::Archive | Op::File(_) | Op::Trash | Op::Spam | Op::Restore => Some(Exit::Fold),
        Op::SetSnooze(Snooze::Until(_)) => Some(Exit::Curl),
        _ => None,
    }
}

/// The sidebar place an op lands in, by name.
pub(super) fn destination(op: &Op, labels: &[(String, LabelId)]) -> Option<String> {
    let name = match op {
        Op::Archive => "Archive",
        Op::Trash => "Trash",
        Op::Spam => "Spam",
        Op::Restore => "Inbox",
        Op::SetSnooze(Snooze::Until(_)) => "Snoozed",
        Op::SetStar(Star::Starred) => "Starred",
        Op::SetPin(Pin::Rank(_)) => "Pinned",
        Op::Label(id, Membership::In) => {
            return labels
                .iter()
                .find(|(_, label)| label == id)
                .map(|(name, _)| name.clone());
        }
        _ => return None,
    };
    Some(name.to_owned())
}

impl Motion {
    /// What an op that has been applied looks like.
    fn landed(
        mut self,
        store: &SqliteStore,
        shell: Signal<Shell>,
        thread: ThreadId,
        op: &Op,
        before: Option<ThreadSummary>,
    ) {
        let clock = *self.clock.peek();
        self.dest.set(None);
        if let Some(place) = destination(op, &shell.peek().labels) {
            self.gulp.set(Some(place));
            if let Some(clock) = clock {
                clock.gulp.start(clock.gulped);
            }
        }
        if let Op::Label(label, Membership::In) = op {
            self.landing.set(Some((thread, *label)));
            if let Some(clock) = clock {
                clock.landing.start(clock.landed);
            }
        }
        let (Some(exit), Some(before)) = (exit(op), before) else {
            return;
        };
        if self.stays(store, shell, thread) || !self.order.read().contains(&thread) {
            return;
        }
        let key = thread;
        let emphasis = if before.read == ReadState::Unread {
            Emphasis::Strong
        } else {
            Emphasis::Plain
        };
        let mut leaving = self.leaving.write();
        // A row's summary is kept while the roster still draws it leaving, and no longer.
        let drawn = clock
            .map(|clock| clock.roster.entries())
            .unwrap_or_default();
        leaving.retain(|row| {
            row.key != thread
                && drawn.iter().any(|entry| {
                    entry.key == row.key && matches!(entry.presence, ds::Presence::Leaving(_))
                })
        });
        leaving.push(Leaving {
            key,
            summary: before,
        });
        drop(leaving);
        if let Some(clock) = clock {
            clock.roster.leave(key, exit, emphasis);
        }
    }

    /// Whether the row still belongs to the list it is in. A search is global, so a row there
    /// stays whatever happened to it; a place is its filter, asked of the row as it is now.
    fn stays(&self, store: &SqliteStore, shell: Signal<Shell>, thread: ThreadId) -> bool {
        belongs(store, &shell.peek(), thread, Utc::now())
    }

    /// Put up the toast. The next op replaces it; otherwise it leaves on its own. An undo or
    /// plain words are quire's toast; a follow-up is mailo's own, and takes quire's down.
    fn say(mut self, text: String, follow: Follow) {
        if let Some(toasts) = *self.toasts.peek() {
            match follow {
                Follow::Undo(handle) => {
                    self.toast.set(None);
                    toasts
                        .hub
                        .push_undoable(text, UndoToken(handle.0), toasts.on_undo);
                    return;
                }
                Follow::Nothing => {
                    self.toast.set(None);
                    toasts.hub.push(text, None);
                    return;
                }
                Follow::ArchiveFrom { .. } | Follow::Unblock { .. } => toasts.hub.hide(),
            }
        }
        let serial = self.toast.peek().as_ref().map_or(0, |said| said.serial) + 1;
        self.toast.set(Some(Said {
            text,
            serial,
            follow,
        }));
        dioxus::core::Runtime::current().spawn(self.owner, async move {
            tokio::time::sleep(TOAST).await;
            if self
                .toast
                .peek()
                .as_ref()
                .is_some_and(|said| said.serial == serial)
            {
                self.toast.set(None);
            }
        });
    }
}

/// Whether `thread`, as the store now holds it, is still in the list `shell` shows.
///
/// A search is global, so a row there stays whatever happened to it; a place is its filter,
/// asked of the thread with the folders the server holds its messages in, so a folder's place
/// keeps a row that was starred and lets go of one whose mail has left the folder.
pub(in crate::ui) fn belongs(
    store: &SqliteStore,
    shell: &Shell,
    thread: ThreadId,
    now: DateTime<Utc>,
) -> bool {
    if !shell.search.trim().is_empty() {
        return true;
    }
    let Ok(after) = store.thread(thread) else {
        return false;
    };
    let folders = folders_of(store, &after);
    shell.query(1).filter.fit(&MatchCtx {
        summary: &after.summary,
        corpus: None,
        folders: &folders,
        now,
    })
}

/// Every server folder a message of `thread` is addressed in, with whether it is still filed
/// there: what `Filter::InFolder` weighs (`Store::placed`).
fn folders_of(store: &SqliteStore, thread: &Thread) -> Vec<Placed> {
    thread
        .messages
        .iter()
        .filter_map(|message| store.placed(*message).ok())
        .flatten()
        .collect()
}

#[cfg(test)]
mod tests;
