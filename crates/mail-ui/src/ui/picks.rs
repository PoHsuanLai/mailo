//! Several conversations picked at once, in the window: what an action on them means, and the
//! bar that counts them and acts on them.
//!
//! Which conversations are picked is the shell's (`mail_ui_model::selection`, pure). This is the part
//! that needs the client and the window: every action here goes through `motion::act_all`, so
//! whatever a gesture does to five conversations is one entry on the undo stack, one toast, and
//! one Ctrl Z that puts all five back.

use crate::ui::toolbar::{Tool, ToolButton};
use dioxus::prelude::*;
use ds::components::controls::button_model::{Bezel, ImagePosition};
use ds::prelude::*;
use ds::root::common::Common;
use ds::style::tokens::control_size::ControlSize;
use mail_client::Client;
use mail_client::conversation::mute_for_all;
use mail_domain::*;
use mail_ui_kit::motion::{act_all, act_kind_all};
use mail_ui_kit::press::on_primary;
use mail_ui_kit::view::{Shell, Shortcut, mute_label, op_for_selection};
use mail_ui_model::copy::{aria, menus, status, tooltips};

/// What `action` does to the conversations it means among `listed`: the picked ones, or with
/// none picked the open one. One gesture, one undo. Returns how many it reached.
pub(super) fn act_on_picked(
    client: &Client,
    shell: Signal<Shell>,
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
    op_for_selection(action, &summaries)
        .map_or(0, |kind| act_kind_all(client, shell, &targets, kind))
}

/// Put `label` on each of `threads`, or take it off: off when every one already wears it, on
/// otherwise, and only on the ones it changes. One gesture, one undo.
pub(super) fn label_all(
    client: &Client,
    shell: Signal<Shell>,
    threads: &[ThreadId],
    label: LabelId,
) -> usize {
    let wearing: Vec<(ThreadId, bool)> = threads
        .iter()
        .filter_map(|thread| {
            let loaded = client.stored().thread(*thread).ok()?;
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
    act_all(client, shell, ops)
}

/// Mute each of `threads`, or unmute them: unmute when every one is already muted, mute
/// otherwise, and only the ones it changes. One gesture, one undo.
pub(in crate::ui) fn mute_all(
    client: &Client,
    shell: Signal<Shell>,
    threads: &[ThreadId],
) -> usize {
    let summaries: Vec<ThreadSummary> = threads
        .iter()
        .filter_map(|thread| {
            client
                .stored()
                .thread(*thread)
                .ok()
                .map(|loaded| loaded.summary)
        })
        .collect();
    let wanted = mute_for_all(&summaries);
    let ops = summaries
        .iter()
        .filter(|summary| summary.mute != wanted)
        .map(|summary| (summary.id, Op::SetMute(wanted)))
        .collect();
    act_all(client, shell, ops)
}

/// [`mute_all`] over what `action`s mean among `listed`: the picked ones, or with none picked
/// the open one. `m`, and the pick bar's Mute.
pub(super) fn mute_picked(
    client: &Client,
    shell: Signal<Shell>,
    listed: &[ThreadSummary],
) -> usize {
    let ids: Vec<ThreadId> = listed.iter().map(|summary| summary.id).collect();
    let targets = shell.peek().acted_on(&ids);
    mute_all(client, shell, &targets)
}

/// The count and the actions over the list while anything listed is picked.
///
/// Label and Move to… are the picked rows' own menus, which act on the whole selection; this
/// bar holds the ones a button can carry.
#[component]
pub(super) fn PickBar(shell: Signal<Shell>, threads: Memo<Vec<ThreadSummary>>) -> Element {
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
        let say = mail_client::conversation::says_for_all(kind, &summaries, now)?;
        Some((action, ToolButton::of(kind, menus::offered(say))))
    })
    .collect();
    let mute = ToolButton::of(OpKind::Mute, mute_label(&summaries));
    // The way back for what is archived, binned or in Spam: Move to Inbox, or Not Spam when all
    // of it is spam. Never Archive's other side drawn beside Archive for the same conversation.
    let back = mail_client::conversation::says_for_all(OpKind::Restore, &summaries, now)
        .map(|say| ToolButton::of(OpKind::Restore, menus::offered(say)));
    // In Trash or Spam, and only there, the picked mail can be deleted forever, once asked.
    let bin = mail_ui_kit::bin::bin_shown(&shell.read());
    let destroyable = summaries
        .iter()
        .any(|summary| mail_ui_kit::bin::offered(bin, summary));
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
                        let client = mail_ui_kit::live::client();
                        act_on_picked(&client, shell, action, &threads.peek());
                    }),
                }
            }
            if let Some(back) = back {
                Tool {
                    tool: back,
                    aria: Some(aria::on_selected(back.label, count)),
                    onclick: on_primary(move || {
                        let client = mail_ui_kit::live::client();
                        let ids: Vec<ThreadId> = threads.peek().iter().map(|summary| summary.id).collect();
                        let targets = shell.peek().acted_on(&ids);
                        act_kind_all(&client, shell, &targets, OpKind::Restore);
                    }),
                }
            }
            Tool {
                tool: mute,
                aria: Some(aria::on_selected(mute.label, count)),
                onclick: on_primary(move || {
                    let client = mail_ui_kit::live::client();
                    mute_picked(&client, shell, &threads.peek());
                }),
            }
            if destroyable {
                Tool {
                    tool: ToolButton::of(OpKind::Destroy, aria::DELETE_FOREVER),
                    aria: Some(aria::delete_selected_forever(count)),
                    onclick: on_primary(move || {
                        let client = mail_ui_kit::live::client();
                        let ids: Vec<ThreadId> = threads.peek().iter().map(|summary| summary.id).collect();
                        let chosen = shell.peek().picked.chosen(&ids);
                        super::destroy::ask_chosen(&client, shell, &chosen);
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
                title_shortcut: Some(mail_ui_kit::actions::escape()),
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
