//! One row of the list: a conversation or a draft, drawn alike.
//!
//! What a row says, its menu and what a click opens are worked out in [`listed`]; [`ListRow`]
//! draws it and does what it asks. Each row is quire's `ThreadRow` inside mailo's `.row`, the box
//! the row's own menus and the snooze float are placed against. A click anywhere on a row only
//! opens or picks it, and it draws no buttons: its actions are in its menu ([`menu`]), opened by
//! a right click. Split from [`super::app`].

pub(in crate::ui) mod act;
pub(in crate::ui) mod listed;
#[cfg(test)]
mod listed_tests;
pub(in crate::ui) mod menu;
#[cfg(test)]
mod tests;

use super::marked::{Piece, pieces};
use super::menus::FurtherMenus;
use super::motion::{act_kind, drag, motion};
use super::ops::{Writes, open_composer};
use super::picks::drawn_order;
use crate::ui::compose::{cancel_scheduled, delete_stored_draft};
use crate::ui::copy;
use crate::ui::provider_chip::ProvChip;
use crate::ui::selection::Click;
use crate::ui::view::Marks;
use crate::ui::view::{Opened, Shell};
use act::press;
use chordkit::Platform;
use dioxus::prelude::*;
use ds::base::command::holds_primary;
use ds::base::press::{PointerButton, Press};
use ds::base::vocab::RowState;
use ds::components::app::thread_row::ThreadRow;
use ds::components::content::text_runs::{RunTone, TextRun};
use ds::components::controls::chip::{Chip, ChipVariant};
use ds::components::menus::item::item::{MenuImage, MenuItem as DsMenuItem};
use ds::host::measure::MountedRef;
use ds::prelude::*;
use ds::root::common::Common;
use ds::root::pass_through::{DataAttr, DataName};
use ds::style::icon::render::Glyph;
use listed::{Listed, Opens, RowModel};
use mail_core::SqliteStore;
use mail_core::provider::Provider;
use mail_domain::*;
use menu::{Pick, RowMenu};
use std::ops::Range;
use std::sync::Arc;

/// `text` as quire's line, with a search's `marks` as `Mark` runs; plain when nothing is marked.
fn runs(text: &str, marks: &[Range<usize>]) -> TextLine {
    if marks.is_empty() {
        return TextLine::Plain(text.to_owned());
    }
    TextLine::Runs(
        pieces(text, marks)
            .into_iter()
            .map(|piece| match piece {
                Piece::Plain(plain) => TextRun::new(plain, RunTone::Plain),
                Piece::Marked(inside) => TextRun::new(inside, RunTone::Mark),
            })
            .collect(),
    )
}

/// A row of the list, drawn as `model` says, and whichever menu it has open.
#[component]
pub(super) fn ListRow(
    listed: Listed,
    model: RowModel,
    shell: Signal<Shell>,
    mut revision: Signal<u64>,
    /// A conversation's labels, drawn while the list shows them.
    chips: Vec<String>,
    /// Whose conversation it is, drawn only while the list shows more than one account.
    via: Option<AccountMark>,
    /// Picked, or open with nothing picked, asked by the list, which knows what is listed.
    selection: Selection,
) -> Element {
    let platform = use_platform();
    let keys = use_keys();
    // Where the further menu a pick opens floats: where the row's menu stood, else against the
    // row.
    let mut further_at = use_signal(|| None::<Rect>);
    let mut row_box = use_signal(|| None::<MountedRef>);
    // The row's menu, and the point it was asked for at.
    let mut row_menu = use_signal(|| None::<Rect>);
    let thread = match &listed {
        Listed::Thread(summary) | Listed::Message(summary, _) => Some(summary.clone()),
        Listed::Draft(_) => None,
    };
    let draft = match &listed {
        Listed::Draft(draft) => Some(draft.id),
        Listed::Thread(_) | Listed::Message(..) => None,
    };
    let subject = listed.subject();
    // A conversation's sender, and whether they are a VIP: the mark beside the name, and the
    // menu's last row, which marks them or takes the mark off.
    let sender = thread
        .as_ref()
        .map(|summary| (summary.from.email.clone(), summary.from.name.clone()));
    let vip = sender
        .as_ref()
        .is_some_and(|(email, _)| shell.read().vips.includes(email));
    let mut menu_rows = listed.menu(chrono::Utc::now());
    if let Some((email, _)) = &sender {
        menu_rows.push(DsMenuItem::Separator);
        menu_rows.push(
            DsMenuItem::new(
                Pick::Vip,
                crate::ui::vip::menu_words(&shell.read().vips, email),
            )
            .with_image(MenuImage::Icon(Icon::Star)),
        );
    }
    let dragged = thread.as_ref().is_some_and(|summary| {
        motion().is_some_and(|state| {
            matches!(*state.drag.read(), drag::Drag::Live { thread, .. } if thread == summary.id)
        })
    });
    let state = RowState {
        selection,
        emphasis: model.emphasis,
        drop: if dragged {
            DropState::Source
        } else {
            DropState::Idle
        },
        ..RowState::default()
    };
    // Where a test, and a hover card, find a message held for its time: by its subject.
    let common = Common {
        aria_label: Some(model.label.clone()),
        data: listed
            .held()
            .and_then(|_| DataName::parse("scheduled").ok())
            .map(|name| vec![DataAttr::new(name, subject.clone())])
            .unwrap_or_default(),
        ..Common::default()
    };
    let via = via.map(|via| rsx! { ViaChip { via, marks: shell.read().appearance.marks } });
    let via = match (vip, via) {
        (false, via) => via,
        (true, via) => Some(rsx! {
            crate::ui::vip::VipMark {}
            {via}
        }),
    };
    // A star is drawn only on a starred conversation, as its mark: starring is in the row's menu
    // and on its key, and nothing appears under the pointer. Pressing the mark unstars.
    let star = thread
        .as_ref()
        .filter(|summary| summary.star == Star::Starred)
        .map(|summary| {
            let id = summary.id;
            (
                Check::On,
                EventHandler::new(move |_: Check| {
                    let store = consume_context::<Arc<SqliteStore>>();
                    act_kind(&store, shell, revision, id, OpKind::Unstar);
                }),
            )
        });
    let tags = match &thread {
        Some(summary) => {
            rsx! { Tags { summary: summary.clone(), chips, shown: shell.read().parts.chips.shown() } }
        }
        None => rsx! {},
    };
    let open_on = listed.clone();
    let id = thread.as_ref().map(|summary| summary.id);
    rsx! {
        div { key: "{listed.key()}", class: "row", role: "none",
            onmounted: move |event: MountedEvent| row_box.set(Some(MountedRef(event.data()))),
            oncontextmenu: move |event: MouseEvent| {
                event.prevent_default();
                row_menu.set(Some(point_rect(event.client_coordinates())));
            },
            // Shift+Enter on a focused conversation opens it in a window of its own. Stopped
            // here, so the window's own Shift+Enter does not open the open conversation as well.
            onkeydown: move |event: KeyboardEvent| {
                let opens = crate::ui::actions::heard(keys, &event, false)
                    == Some(crate::ui::actions::Heard::Own(crate::ui::actions::Own::OpenInWindow));
                if let (true, Some(id)) = (opens, id) {
                    event.stop_propagation();
                    super::window::open_in_window(id);
                }
            },
            ThreadRow {
                state,
                name: model.name,
                via,
                subject: model.subject,
                snippet: model.snippet,
                time: model.time,
                tags,
                star,
                star_shortcut: crate::ui::actions::tip(crate::ui::view::Shortcut::ToggleStar),
                strip: None,
                more: None,
                onclick: move |press: Press| {
                    let Some(click) = click_of(press, platform) else {
                        return;
                    };
                    match open_on.open(click) {
                        Opens::Thread(id, click) => shell.write().click(id, click, &drawn_order()),
                        Opens::Message(id, message) => {
                            let at = super::picks::drawn_messages()
                                .iter()
                                .position(|shown| *shown == message)
                                .unwrap_or_default();
                            shell.write().open_message(id, message, at);
                        }
                        Opens::Range(message) => {
                            if let Some(at) = super::picks::drawn_messages()
                                .iter()
                                .position(|shown| *shown == message)
                            {
                                shell.write().click_row(at, &drawn_order());
                            }
                        }
                        Opens::Draft(id) => {
                            let store = consume_context::<Arc<SqliteStore>>();
                            let _ = open_composer(&store, shell, Writes::Draft(id));
                        }
                    }
                },
                onpointerdown: id.map(|id| EventHandler::new(move |event: PointerEvent| {
                    let point = event.client_coordinates();
                    drag::press(id, (point.x, point.y));
                })),
                common,
            }
            if let Some(at) = row_menu() {
                RowMenu {
                    at,
                    anchor: row_box(),
                    subject: subject.clone(),
                    items: menu_rows,
                    on_pick: move |pick: Pick| {
                        row_menu.set(None);
                        match (pick, id, draft) {
                            (Pick::Press(pressed), Some(id), _) => {
                                // A menu the pick opens stands where this one stood.
                                if act::further(pressed).is_some() {
                                    further_at.set(Some(at));
                                }
                                press(shell, revision, id, pressed, Opened::Row);
                            }
                            (Pick::DeleteDraft, _, Some(draft)) => delete_stored_draft(shell, revision, draft),
                            (Pick::Vip, Some(_), _) => {
                                if let Some((email, name)) = &sender {
                                    let mark = mail_core::vip::toggled(&shell.peek().vips, email);
                                    let marking = crate::ui::vip::Marking { shell, revision: Some(revision) };
                                    crate::ui::vip::set(marking, email, name.as_deref(), mark);
                                }
                            }
                            // A refusal is said in the toast; the message stays as it was.
                            (Pick::CancelSending, _, Some(draft))
                                if cancel_scheduled(shell, draft).is_ok() =>
                            {
                                revision += 1;
                            }
                            // Not offered here: the reader's ⋯ holds them.
                            _ => {}
                        }
                    },
                    on_close: move |_| row_menu.set(None),
                }
            }
            if let Some(summary) = thread {
                FurtherMenus { summary, opened: Opened::Row, shell, revision, anchor: row_box(), placed: further_at() }
            }
        }
    }
}

/// A conversation's marks at the end of its row: its labels, muted, a reply that never came,
/// files.
#[component]
fn Tags(summary: ThreadSummary, chips: Vec<String>, shown: bool) -> Element {
    let no_reply = mail_core::follow_up::returned_words(&summary.follow_up);
    let muted = summary.mute == Mute::Muted;
    let files = matches!(summary.attachments, Attachments::Present { .. });
    rsx! {
        if shown {
            for name in chips {
                span { key: "{name}", "data-chip": "{name}",
                    Chip { variant: ChipVariant::Neutral, text: name.clone() }
                }
            }
        }
        if muted {
            Tooltip { text: "Muted".to_owned(),
                span { class: "mute-mark", aria_label: "Muted", "data-muted": "true",
                    Glyph { icon: Icon::BellOff, size: IconSize::Compact }
                }
            }
        }
        if let Some(words) = no_reply {
            span { class: "no-reply", "data-follow-up": "returned",
                Glyph { icon: Icon::Bell, size: IconSize::Compact }
                "{words}"
            }
        }
        if files {
            span { class: "clip", aria_label: copy::aria::HAS_ATTACHMENTS,
                Glyph { icon: Icon::Paperclip, size: IconSize::Compact }
            }
        }
    }
}

/// A point in the window as the rect a context menu is placed against.
pub(in crate::ui) fn point_rect(at: dioxus::html::geometry::ClientPoint) -> Rect {
    Rect {
        origin: Point {
            x: Px(at.x as f32),
            y: Px(at.y as f32),
        },
        size: Size {
            width: Px(0.0),
            height: Px(0.0),
        },
    }
}

/// What a click on a row asks for, from its button and the keys held with it. Only the primary
/// button opens or picks: a right click is the row's menu, and opens nothing by itself. Shift
/// wins over the platform's primary key (Command, or Ctrl), as a range is the larger thing to have
/// asked for.
fn click_of(press: Press, platform: Platform) -> Option<Click> {
    if press.button != PointerButton::Primary {
        return None;
    }
    let held = press.modifiers;
    Some(if held.shift() {
        Click::Range
    } else if holds_primary(platform, held) {
        Click::Toggle
    } else {
        Click::Plain
    })
}

/// Which account a conversation is on: its provider's mark, and the account's name in its tip.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct AccountMark {
    pub provider: Provider,
    /// The account as the window names it: its address, or the name it was given.
    pub name: String,
}

#[component]
fn ViaChip(via: AccountMark, marks: Marks) -> Element {
    rsx! {
        Tooltip { text: via.name.clone(),
            span { class: "via", aria_label: via.name.clone(),
                ProvChip { provider: via.provider, marks }
            }
        }
    }
}
