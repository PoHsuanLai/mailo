//! Several conversations picked at once, in the window: what an action on them means, and the
//! bar that counts them and acts on them.
//!
//! Which conversations are picked is the shell's (`crate::ui::selection`, pure). This is the part
//! that needs the store and the window: every action here goes through `motion::act_all`, so
//! whatever a gesture does to five conversations is one entry on the undo stack, one toast, and
//! one Ctrl Z that puts all five back.

use super::motion::{act_all, act_kind_all, motion};
use super::press::on_primary;
use crate::ui::copy::{aria, menus, status, tooltips};
use crate::ui::toolbar::{Tool, ToolButton};
use crate::ui::view::{Shell, Shortcut, mute_label, op_for_selection};
use dioxus::prelude::*;
use ds::components::controls::button_model::{Bezel, ImagePosition};
use ds::prelude::*;
use ds::root::common::Common;
use ds::style::tokens::control_size::ControlSize;
use mail_core::view::mute_for_all;
use mail_core::{SqliteStore, Store};
use mail_domain::*;
use std::sync::Arc;

/// The ids the list drew last, in order: what a click on a row measures a range over.
pub(super) fn drawn_order() -> Vec<ThreadId> {
    motion()
        .map(|state| state.order.read().clone())
        .unwrap_or_default()
}

/// The messages the list drew last, one for each of the rows [`drawn_order`] gives, while it
/// lists a message to a row; empty while it lists conversations.
pub(super) fn drawn_messages() -> Vec<MessageId> {
    motion()
        .map(|state| state.messages.read().clone())
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
    let now = chrono::Utc::now();
    // Each button is the action's own menu row: the glyph and the side of it the picked mail is
    // offered, so Mark as Unread and Unstar read as the row's menu would.
    let buttons: Vec<(Shortcut, ToolButton)> = [
        Shortcut::Archive,
        Shortcut::Trash,
        Shortcut::Spam,
        Shortcut::ToggleRead,
        Shortcut::ToggleStar,
    ]
    .into_iter()
    .filter_map(|action| {
        let kind = offered(action)?;
        let say = mail_core::view::says_for_all(kind, &summaries, now)?;
        Some((action, ToolButton::of(kind, menus::offered(say))))
    })
    .collect();
    let mute = ToolButton::of(OpKind::Mute, mute_label(&summaries));
    // The way back for what is archived, binned or in Spam: Move to Inbox, or Not Spam when all
    // of it is spam. Never Archive's other side drawn beside Archive for the same conversation.
    let back = mail_core::view::says_for_all(OpKind::Restore, &summaries, now)
        .map(|say| ToolButton::of(OpKind::Restore, menus::offered(say)));
    // In Trash or Spam, and only there, the picked mail can be deleted forever, once asked.
    let bin = crate::ui::bin::bin_shown(&shell.read());
    let destroyable = summaries
        .iter()
        .any(|summary| crate::ui::bin::offered(bin, summary));
    rsx! {
        span { class: "status", {status::selected(count)} }
        // The actions wrap within the list column when it is too narrow for one line.
        div { class: "pick-tools",
            for (action, tool) in buttons {
                Tool {
                    key: "{tool.label}",
                    tool,
                    aria: Some(aria::on_selected(tool.label, count)),
                    onclick: on_primary(move || {
                        let store = consume_context::<Arc<SqliteStore>>();
                        act_on_picked(&store, shell, revision, action, &threads.peek());
                    }),
                }
            }
            if let Some(back) = back {
                Tool {
                    tool: back,
                    aria: Some(aria::on_selected(back.label, count)),
                    onclick: on_primary(move || {
                        let store = consume_context::<Arc<SqliteStore>>();
                        let ids: Vec<ThreadId> = threads.peek().iter().map(|summary| summary.id).collect();
                        let targets = shell.peek().acted_on(&ids);
                        act_kind_all(&store, shell, revision, &targets, OpKind::Restore);
                    }),
                }
            }
            Tool {
                tool: mute,
                aria: Some(aria::on_selected(mute.label, count)),
                onclick: on_primary(move || {
                    let store = consume_context::<Arc<SqliteStore>>();
                    mute_picked(&store, shell, revision, &threads.peek());
                }),
            }
            if destroyable {
                Tool {
                    tool: ToolButton::of(OpKind::Destroy, aria::DELETE_FOREVER),
                    aria: Some(aria::delete_selected_forever(count)),
                    onclick: on_primary(move || {
                        let store = consume_context::<Arc<SqliteStore>>();
                        let ids: Vec<ThreadId> = threads.peek().iter().map(|summary| summary.id).collect();
                        let chosen = shell.peek().picked.chosen(&ids);
                        super::destroy::ask_chosen(&store, shell, &chosen);
                    }),
                }
            }
            Button {
                bezel: Bezel::Toolbar,
                image: ImagePosition::Only,
                size: ControlSize::Large,
                label: aria::CLEAR_THE_SELECTION.to_owned(),
                icon: Icon::X,
                title: Some(tooltips::CLEAR_SELECTION.to_owned()),
                title_shortcut: Some(crate::ui::actions::escape()),
                common: Common {
                    aria_label: Some(aria::CLEAR_THE_SELECTION.to_owned()),
                    ..Common::default()
                },
                onclick: on_primary(move || {
                    let ids: Vec<ThreadId> = threads.peek().iter().map(|summary| summary.id).collect();
                    shell.write().unpick(&ids);
                }),
            }
        }
    }
}
