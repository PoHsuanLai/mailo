//! Several conversations picked at once, in the window: what an action on them means, and the
//! bar that counts them and acts on them.
//!
//! Which conversations are picked is the shell's (`crate::selection`, pure). This is the part
//! that needs the store and the window: every action here goes through `motion::act_all`, so
//! whatever a gesture does to five conversations is one entry on the undo stack, one toast, and
//! one Ctrl Z that puts all five back.

use super::motion::{act_all, act_kind_all, motion};
use super::press::on_primary;
use crate::view::{Shell, Shortcut, mute_for_all, mute_label, op_for_selection};
use dioxus::prelude::*;
use ds::Icon;
use mail_domain::*;
use mail_store::{SqliteStore, Store};
use std::sync::Arc;

/// The ids the list drew last, in order: what a click on a row measures a range over.
pub(super) fn drawn_order() -> Vec<ThreadId> {
    motion()
        .map(|state| state.order.read().clone())
        .unwrap_or_default()
}

/// The conversations an action on `thread`'s own row means: the selection when the row is in
/// it, the row alone when it is not (`Shell::with_selection`).
pub(super) fn with_selection(shell: Signal<Shell>, thread: ThreadId) -> Vec<ThreadId> {
    shell.peek().with_selection(thread, &drawn_order())
}

/// What `action` does to the conversations it means among `listed`: the picked ones, or with
/// none picked the open one. One gesture, one undo. Returns how many it reached.
pub(super) fn act_on_picked(
    store: &SqliteStore,
    shell: Signal<Shell>,
    revision: Signal<u64>,
    action: Shortcut,
    listed: &[ThreadSummary],
) -> usize {
    let ids: Vec<ThreadId> = listed.iter().map(|summary| summary.id).collect();
    let targets = shell.peek().acted_on(&ids);
    let summaries: Vec<ThreadSummary> = listed
        .iter()
        .filter(|summary| targets.contains(&summary.id))
        .cloned()
        .collect();
    op_for_selection(action, &summaries).map_or(0, |kind| {
        act_kind_all(store, shell, revision, &targets, kind)
    })
}

/// Put `label` on each of `threads`, or take it off: off when every one already wears it, on
/// otherwise, and only on the ones it changes. One gesture, one undo.
pub(super) fn label_all(
    store: &SqliteStore,
    shell: Signal<Shell>,
    revision: Signal<u64>,
    threads: &[ThreadId],
    label: LabelId,
) -> usize {
    let wearing: Vec<(ThreadId, bool)> = threads
        .iter()
        .filter_map(|thread| {
            let loaded = store.thread(*thread).ok()?;
            Some((*thread, loaded.summary.labels.contains(&label)))
        })
        .collect();
    let wanted = if wearing.iter().all(|(_, on)| *on) {
        Membership::Out
    } else {
        Membership::In
    };
    let ops = wearing
        .into_iter()
        .filter(|(_, on)| match wanted {
            Membership::In => !*on,
            Membership::Out => *on,
        })
        .map(|(thread, _)| (thread, Op::Label(label, wanted)))
        .collect();
    act_all(store, shell, revision, ops)
}

/// Mute each of `threads`, or unmute them: unmute when every one is already muted, mute
/// otherwise, and only the ones it changes. One gesture, one undo.
pub(in crate::ui) fn mute_all(
    store: &SqliteStore,
    shell: Signal<Shell>,
    revision: Signal<u64>,
    threads: &[ThreadId],
) -> usize {
    let summaries: Vec<ThreadSummary> = threads
        .iter()
        .filter_map(|thread| store.thread(*thread).ok().map(|loaded| loaded.summary))
        .collect();
    let wanted = mute_for_all(&summaries);
    let ops = summaries
        .iter()
        .filter(|summary| summary.mute != wanted)
        .map(|summary| (summary.id, Op::SetMute(wanted)))
        .collect();
    act_all(store, shell, revision, ops)
}

/// [`mute_all`] over what `action`s mean among `listed`: the picked ones, or with none picked
/// the open one. `m`, and the pick bar's Mute.
pub(super) fn mute_picked(
    store: &SqliteStore,
    shell: Signal<Shell>,
    revision: Signal<u64>,
    listed: &[ThreadSummary],
) -> usize {
    let ids: Vec<ThreadId> = listed.iter().map(|summary| summary.id).collect();
    let targets = shell.peek().acted_on(&ids);
    mute_all(store, shell, revision, &targets)
}

/// The count and the actions over the list while anything listed is picked.
///
/// Label and Move to… are the picked rows' own menus, which act on the whole selection; this
/// bar holds the ones a button can carry.
#[component]
pub(super) fn PickBar(
    shell: Signal<Shell>,
    revision: Signal<u64>,
    threads: Memo<Vec<ThreadSummary>>,
) -> Element {
    let listed = threads();
    let ids: Vec<ThreadId> = listed.iter().map(|summary| summary.id).collect();
    let chosen = shell.read().picked.chosen(&ids);
    if chosen.is_empty() {
        return rsx! {};
    }
    let count = chosen.len();
    let summaries: Vec<ThreadSummary> = listed
        .iter()
        .filter(|summary| chosen.contains(&summary.id))
        .cloned()
        .collect();
    let offered = |action| op_for_selection(action, &summaries);
    let read = match offered(Shortcut::ToggleRead) {
        Some(OpKind::MarkUnread) => (Icon::Mail, "Mark unread"),
        _ => (Icon::MailOpen, "Mark read"),
    };
    let star = match offered(Shortcut::ToggleStar) {
        Some(OpKind::Unstar) => "Unstar",
        _ => "Star",
    };
    let mute = mute_label(&summaries);
    let buttons: Vec<(Shortcut, Icon, &'static str)> = [
        (Shortcut::Archive, Icon::Archive, "Archive"),
        (Shortcut::Trash, Icon::Trash, "Move to Trash"),
        (Shortcut::Spam, Icon::OctagonAlert, "Mark as spam"),
        (Shortcut::ToggleRead, read.0, read.1),
        (Shortcut::ToggleStar, Icon::Star, star),
    ]
    .into_iter()
    .filter(|(action, _, _)| offered(*action).is_some())
    .collect();
    rsx! {
        span { class: "status", "{count} selected" }
        for (action, icon, name) in buttons {
            ds::Button {
                key: "{name}",
                variant: ds::ButtonVariant::Mini,
                label: String::new(),
                icon,
                aria_label: format!("{name} the {count} selected"),
                title: name.to_owned(),
                onclick: on_primary(move || {
                    let store = consume_context::<Arc<SqliteStore>>();
                    act_on_picked(&store, shell, revision, action, &threads.peek());
                }),
            }
        }
        ds::Button {
            variant: ds::ButtonVariant::Mini,
            label: String::new(),
            icon: Icon::BellOff,
            aria_label: format!("{mute} the {count} selected"),
            title: format!("{mute} (m)"),
            onclick: on_primary(move || {
                let store = consume_context::<Arc<SqliteStore>>();
                mute_picked(&store, shell, revision, &threads.peek());
            }),
        }
        ds::Button {
            variant: ds::ButtonVariant::Mini,
            label: String::new(),
            icon: Icon::X,
            aria_label: "Clear the selection".to_owned(),
            title: "Clear the selection (Esc)".to_owned(),
            onclick: on_primary(move || {
                let ids: Vec<ThreadId> = threads.peek().iter().map(|summary| summary.id).collect();
                shell.write().unpick(&ids);
            }),
        }
    }
}
