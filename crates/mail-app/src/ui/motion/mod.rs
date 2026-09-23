//! Motion keyed to state.
//!
//! Every animation here starts because something the window already knows became true, never
//! because a timer stands in for it (Part E, decision 4):
//!
//! - a row is `going[data-op]` because an op was applied and the row no longer belongs to the
//!   list — it stays drawn until its own `animationend` (a 900 ms fallback is the safety net);
//! - the rows below it are `healing` because that row has gone;
//! - a place `gulp`s because an op landed there, and a label `chip-land`s because it was added;
//! - the undo toast is up because an op was applied, and it names it.
//!
//! **The store write happens first and at once.** Only the row's unmount waits.

pub(super) mod drag;
mod toast;

pub(super) use drag::Ghost;
pub(super) use toast::Toast;

use super::ops::{perform, resolve, take_back};
use crate::view::Shell;
use chrono::Utc;
use dioxus::prelude::*;
use mail_domain::*;
use mail_store::{SqliteStore, Store};

/// A row that has left the list and is still being drawn while it goes.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Leaving {
    /// The row as it was drawn before the op.
    pub summary: ThreadSummary,
    /// Where it stood in the list.
    pub index: usize,
    /// `data-op`: which exit it plays.
    pub op: &'static str,
    /// The rows under it, which heal into the gap once it has gone.
    pub below: Vec<ThreadId>,
}

/// What the toast says, and which op it belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Said {
    pub text: String,
    pub serial: u64,
}

/// The motion state the window shares.
#[derive(Clone, Copy)]
pub(super) struct Motion {
    pub leaving: Signal<Vec<Leaving>>,
    /// Rows healing into a gap, with their stagger.
    pub healing: Signal<Vec<(ThreadId, usize)>>,
    /// The place an op just landed in.
    pub gulp: Signal<Option<String>>,
    /// A label just added to a row.
    pub landing: Signal<Option<(ThreadId, LabelId)>>,
    /// A row an undo just brought back.
    pub returning: Signal<Option<ThreadId>>,
    pub toast: Signal<Option<Said>>,
    /// The place a hovered Archive or Snooze button would send the row to.
    pub dest: Signal<Option<&'static str>>,
    pub drag: Signal<drag::Drag>,
    pub pull: Signal<Option<toast::Pull>>,
    /// The ids the list drew last, in order. Not reactive: only an op reads it.
    pub order: CopyValue<Vec<ThreadId>>,
    /// The scope that owns all of this. The fallback and the toast's timeout run there, so a
    /// row that unmounts does not cancel them.
    owner: ScopeId,
}

/// Make the motion state for the window. Called once, from `App`.
pub(super) fn use_motion() -> Motion {
    use_context_provider(|| Motion {
        leaving: Signal::new(Vec::new()),
        healing: Signal::new(Vec::new()),
        gulp: Signal::new(None),
        landing: Signal::new(None),
        returning: Signal::new(None),
        toast: Signal::new(None),
        dest: Signal::new(None),
        drag: Signal::new(drag::Drag::Idle),
        pull: Signal::new(None),
        order: CopyValue::new(Vec::new()),
        owner: dioxus::core::current_scope_id(),
    })
}

/// The window's motion state, when there is a window around the caller.
pub(super) fn motion() -> Option<Motion> {
    try_consume_context::<Motion>()
}

/// How long a row may take to leave before it is removed anyway.
const FALLBACK: std::time::Duration = std::time::Duration::from_millis(900);
/// How long the toast stays.
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
    shell.write().undo.push(undo);
    revision += 1;
    if let Some(motion) = motion() {
        motion.landed(store, shell, thread, &op, before);
        motion.say(said);
    }
    true
}

/// Take back the newest op. The toast's tab, and Ctrl Z.
pub(super) fn undo_last(
    store: &SqliteStore,
    mut shell: Signal<Shell>,
    mut revision: Signal<u64>,
) -> bool {
    let Some(entry) = shell.write().undo.pop() else {
        return false;
    };
    if !take_back(store, &entry) {
        shell.write().undo.push(entry);
        return false;
    }
    revision += 1;
    if let Some(mut motion) = motion() {
        motion.toast.set(None);
        motion.pull.set(None);
        motion
            .leaving
            .write()
            .retain(|row| row.summary.id != entry.thread);
        motion.returning.set(Some(entry.thread));
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

/// The row's exit, when `op` takes a row out of a list.
fn exit(op: &Op) -> Option<&'static str> {
    match op {
        Op::Archive => Some("archive"),
        Op::Trash => Some("trash"),
        Op::Spam => Some("spam"),
        Op::Restore => Some("restore"),
        Op::SetSnooze(Snooze::Until(_)) => Some("snooze"),
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
        self.dest.set(None);
        if let Some(place) = destination(op, &shell.peek().labels) {
            self.gulp.set(Some(place));
        }
        if let Op::Label(label, Membership::In) = op {
            self.landing.set(Some((thread, *label)));
        }
        let (Some(slug), Some(before)) = (exit(op), before) else {
            return;
        };
        if self.stays(store, shell, thread) {
            return;
        }
        let order = self.order.read().clone();
        let Some(index) = order.iter().position(|id| *id == thread) else {
            return;
        };
        let below = order[index + 1..].to_vec();
        let mut leaving = self.leaving.write();
        leaving.retain(|row| row.summary.id != thread);
        leaving.push(Leaving {
            summary: before,
            index,
            op: slug,
            below,
        });
        drop(leaving);
        dioxus::core::Runtime::current().spawn(self.owner, async move {
            tokio::time::sleep(FALLBACK).await;
            finish(self, thread);
        });
    }

    /// Whether the row still belongs to the list it is in. A search is global, so a row there
    /// stays whatever happened to it; a place is its filter, asked of the row as it is now.
    fn stays(&self, store: &SqliteStore, shell: Signal<Shell>, thread: ThreadId) -> bool {
        let current = shell.peek();
        if !current.search.trim().is_empty() {
            return true;
        }
        let Ok(after) = store.thread(thread) else {
            return false;
        };
        let filter = current.query(1).filter;
        filter.fit(&MatchCtx {
            summary: &after.summary,
            corpus: None,
            now: Utc::now(),
        })
    }

    /// Put up the toast. The next op replaces it; otherwise it leaves on its own.
    fn say(mut self, text: String) {
        let serial = self.toast.peek().as_ref().map_or(0, |said| said.serial) + 1;
        self.toast.set(Some(Said { text, serial }));
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

/// A leaving row has finished: take it off the page, and let the rows under it heal.
pub(super) fn finish(mut motion: Motion, thread: ThreadId) {
    let Some(at) = motion
        .leaving
        .peek()
        .iter()
        .position(|row| row.summary.id == thread)
    else {
        return;
    };
    let gone = motion.leaving.write().remove(at);
    motion.healing.set(
        gone.below
            .into_iter()
            .take(12)
            .enumerate()
            .map(|(stagger, id)| (id, stagger))
            .collect(),
    );
}

/// The animation a row just finished, and what it means.
pub(super) fn row_animation_ended(thread: ThreadId, name: &str) {
    let Some(mut motion) = motion() else {
        return;
    };
    match name {
        "fold" | "curl" => finish(motion, thread),
        "heal" => motion.healing.write().retain(|(id, _)| *id != thread),
        "rise" if *motion.returning.peek() == Some(thread) => motion.returning.set(None),
        "chip-land" if motion.landing.peek().is_some_and(|(id, _)| id == thread) => {
            motion.landing.set(None);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests;
