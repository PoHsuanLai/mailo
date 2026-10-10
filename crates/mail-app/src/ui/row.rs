//! One row of the list.
//!
//! A draft and a conversation are different rows: a draft opens the composer, a conversation
//! opens the reader. Split from [`super::app`] (`CONVENTIONS.md` §8).
//!
//! Each row is quire's `ThreadRow` inside mailo's `.row`, the box the row's own menus and the
//! snooze float are placed against. A click anywhere on a row only opens or picks it: the row's
//! actions are in its menu ([`menu`]), opened by a right click or by the one button the row shows
//! under the pointer, quire's `RowMore` (the ⋯), which cannot act on the conversation by itself.

pub(in crate::ui) mod act;
pub(in crate::ui) mod menu;
#[cfg(test)]
mod tests;

use super::list_search::RowHit;
use super::marked::{Piece, pieces};
use super::menus::{LabelMenu, SnoozeMenu};
use super::motion::{act_kind, drag, motion};
use super::move_to::MoveMenu;
use super::picks::drawn_order;
use super::text::{draft_state, sender};
use crate::ui::provider_chip::ProvChip;
use crate::ui::selection::Click;
use crate::ui::view::Marks;
use crate::ui::view::{Shell, hover_in};
use act::{Pressed, press};
use chordkit::Platform;
use chrono::Local;
use dioxus::prelude::*;
use ds::base::command::holds_primary;
use ds::base::press::{PointerButton, Press};
use ds::base::vocab::RowState;
use ds::components::app::row_more::RowMore;
use ds::components::app::thread_row::ThreadRow;
use ds::components::content::text_runs::{RunTone, TextRun};
use ds::components::controls::badge::{Badge, BadgeContent, BadgeTone};
use ds::components::controls::chip::{Chip, ChipVariant};
use ds::host::measure::MountedRef;
use ds::prelude::*;
use ds::root::common::Common;
use ds::style::icon::render::Glyph;
use ds::style::tokens::control_size::ControlSize;
use mail_core::provider::Provider;
use mail_domain::*;
use mail_store::{SqliteStore, Store};
use menu::{Muted, Opener, Pick, RowMenu};
use std::ops::Range;
use std::sync::Arc;

/// A draft, as a row. Clicking it opens the composer on that draft.
#[component]
pub(super) fn DraftRow(draft: Draft, shell: Signal<Shell>) -> Element {
    let id = draft.id;
    let subject = if draft.subject.is_empty() {
        "(no subject)".to_owned()
    } else {
        draft.subject.clone()
    };
    let who = mail_core::compose::addresses::join_addresses(&draft.to);
    let who = if who.is_empty() {
        "(no recipient)".to_owned()
    } else {
        who
    };
    let state = draft_state(&draft.state);
    let when = mail_core::when::listed(draft.updated, chrono::Utc::now(), &Local);
    let parts = shell.read().parts;
    let snippet = parts.snippet.shown().then(|| TextLine::from(state));
    let time = if parts.time.shown() {
        when
    } else {
        String::new()
    };
    rsx! {
        div { key: "{id}", class: "row", role: "none",
            ThreadRow {
                name: who,
                via: None,
                subject: TextLine::from(subject),
                snippet,
                time,
                tags: rsx! {},
                star: None,
                strip: None,
                more: None,
                onclick: move |_| {
                    let store = consume_context::<Arc<SqliteStore>>();
                    if let Ok(draft) = store.draft(id) {
                        shell.write().compose(&draft);
                    }
                },
            }
        }
    }
}

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

/// A conversation, as a row, including its ⋯ and whichever menu it has open.
#[component]
pub(super) fn MailRow(
    summary: ThreadSummary,
    shell: Signal<Shell>,
    revision: Signal<u64>,
    chips: Vec<String>,
    via: Option<Provider>,
    hit: Option<RowHit>,
    /// Picked, or open with nothing picked (`Shell::is_selected`, asked by the list, which
    /// knows what is listed).
    selection: Selection,
) -> Element {
    let id = summary.id;
    let unread = summary.read == ReadState::Unread;
    let starred = summary.star == Star::Starred;
    let muted = summary.mute == Mute::Muted;
    let follow_up = summary.follow_up;
    let no_reply = mail_core::follow_up::row_words(&follow_up);
    let who = sender(&summary);
    let when = mail_core::when::listed(summary.last_date, chrono::Utc::now(), &Local);
    let subject = summary.subject.clone();
    // While a search is active the snippet is cut around its first match, and both lines mark.
    let (subject_marks, snippet, snippet_marks) = match hit {
        Some(hit) => (hit.subject, hit.snippet, hit.snippet_marks),
        None => (Vec::new(), summary.snippet.clone(), Vec::new()),
    };
    let files = match summary.attachments {
        Attachments::Present { count } => Some(count),
        Attachments::None => None,
    };
    // The menu offers what the old hover strip did: the saved view's own actions, or the usual ones,
    // and, in Trash or Spam and only there, Delete forever, which asks first.
    let mut offered: Vec<OpKind> = hover_in(shell.read().saved_view(), &summary);
    if crate::ui::bin::offered(crate::ui::bin::bin_shown(&shell.read()), &summary) {
        offered.push(OpKind::Destroy);
    }
    let menu_rows = menu::rows(
        &menu::groups(&offered, summary.read, summary.star),
        if muted { Muted::Yes } else { Muted::No },
    );
    let filing = shell.read().filing == Some(id);
    // Where the menus a pick opens float: where the row's menu stood, else against the row.
    let keys = use_keys();
    let platform = use_platform();
    let mut snooze_at = use_signal(|| None::<Rect>);
    let mut label_at = use_signal(|| None::<Rect>);
    let mut move_at = use_signal(|| None::<Rect>);
    let mut row_box = use_signal(|| None::<MountedRef>);
    // The focus inside the row shows its ⋯, as the pointer over it does: Blitz never matches
    // `:focus-within`, so quire's button is told.
    let mut more_shown = use_signal(|| None::<Shown>);
    // The row's menu, and what opened it.
    let mut row_menu = use_signal(|| None::<Opener>);
    // "Remind me if no reply", opened from the row's menu where it stood.
    let mut reminding = use_signal(|| None::<Opener>);
    let parts = shell.read().parts;
    let snippet =
        (parts.snippet.shown() && !snippet.is_empty()).then(|| runs(&snippet, &snippet_marks));
    let time = if parts.time.shown() {
        when
    } else {
        String::new()
    };
    let via = if parts.provider.shown() {
        via.map(|via| rsx! { ViaChip { via, marks: shell.read().appearance.marks } })
    } else {
        None
    };
    let star = (
        if starred { Check::On } else { Check::Off },
        EventHandler::new(move |_: Check| {
            let store = consume_context::<Arc<SqliteStore>>();
            let kind = if starred {
                OpKind::Unstar
            } else {
                OpKind::Star
            };
            act_kind(&store, shell, revision, id, kind);
        }),
    );
    let tags = rsx! {
        if parts.chips.shown() {
            for name in chips {
                span { key: "{name}", "data-chip": "{name}",
                    Chip { variant: ChipVariant::Accent, text: name.clone() }
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
        if let Some(count) = files {
            span { class: "clip",
                Glyph { icon: Icon::Paperclip, size: IconSize::Compact }
                Badge { content: BadgeContent::Number(count), tone: BadgeTone::Quiet, size: ControlSize::Small }
            }
        }
    };
    // The ⋯ sits in the row's flow, under the time, and opens the row's menu and does nothing
    // else, so a click that lands on it by mistake changes nothing. The menu hangs from the
    // button's own rect, measured as it is pressed.
    let more = rsx! {
        RowMore {
            // Kept up while its own menu is open, so the menu hangs from something drawn.
            expanded: if matches!(row_menu(), Some(Opener::More(_))) { Shown::Visible } else { Shown::Hidden },
            shown: more_shown(),
            onclick: move |rect: Rect| row_menu.set(Some(Opener::More(rect))),
        }
    };
    let dragged = motion().is_some_and(
        |state| matches!(*state.drag.read(), drag::Drag::Live { thread, .. } if thread == id),
    );
    let state = RowState {
        selection,
        emphasis: if unread {
            Emphasis::Strong
        } else {
            Emphasis::Plain
        },
        drop: if dragged {
            DropState::Source
        } else {
            DropState::Idle
        },
        ..RowState::default()
    };
    rsx! {
        div { key: "{id}", class: "row", role: "none",
            onmounted: move |event: MountedEvent| row_box.set(Some(MountedRef(event.data()))),
            onfocusin: move |_| more_shown.set(Some(Shown::Visible)),
            onfocusout: move |_| more_shown.set(None),
            oncontextmenu: move |event: MouseEvent| {
                event.prevent_default();
                row_menu.set(Some(Opener::Pointer(point_rect(event.client_coordinates()))));
            },
            // Shift+Enter on the focused row opens it in a window of its own. Stopped here, so
            // the window's own Shift+Enter does not open the open conversation as well.
            onkeydown: move |event: KeyboardEvent| {
                let opens = crate::ui::actions::heard(keys, &shell.peek().keymap, &event, false)
                    == Some(crate::ui::actions::Heard::Own(crate::ui::actions::Own::OpenInWindow));
                if opens {
                    event.stop_propagation();
                    super::window::open_in_window(id);
                }
            },
            ThreadRow {
                state,
                name: who,
                via,
                subject: runs(&subject, &subject_marks),
                snippet,
                time,
                tags,
                star: Some(star),
                star_shortcut: crate::ui::actions::tip(
                    &shell.read().keymap,
                    crate::ui::view::Shortcut::ToggleStar,
                ),
                strip: None,
                more: Some(more),
                onclick: move |press: Press| {
                    if let Some(click) = click_of(press, platform) {
                        shell.write().click(id, click, &drawn_order());
                    }
                },
                onpointerdown: EventHandler::new(move |event: PointerEvent| {
                    let point = event.client_coordinates();
                    drag::press(id, (point.x, point.y));
                }),
                common: Common {
                    aria_label: Some(format!("Open {subject}")),
                    ..Common::default()
                },
            }
            if let Some(opener) = row_menu() {
                RowMenu {
                    opener,
                    anchor: row_box(),
                    subject: subject.clone(),
                    items: menu_rows,
                    on_pick: move |pick: Pick| {
                        row_menu.set(None);
                        match pick {
                            Pick::Window => super::window::open_in_window(id),
                            Pick::Remind => reminding.set(Some(opener)),
                            Pick::Press(pressed) => {
                                // A menu the pick opens stands where this one stood.
                                let at = Some(opener.place());
                                match pressed {
                                    Pressed::Op(OpKind::Snooze) => snooze_at.set(at),
                                    Pressed::Op(OpKind::AddLabel) => label_at.set(at),
                                    Pressed::MoveTo => move_at.set(at),
                                    Pressed::Op(_) => {}
                                }
                                press(shell, revision, id, pressed);
                            }
                        }
                    },
                    on_close: move |_| row_menu.set(None),
                }
            }
            if let Some(opener) = reminding() {
                super::follow_up::FollowUpMenu {
                    id,
                    current: follow_up,
                    shell,
                    revision,
                    anchor: row_box(),
                    placed: Some(opener.place()),
                    on_close: move |_| reminding.set(None),
                }
            }
            if shell.read().snoozing == Some(id) {
                SnoozeMenu { id, shell, revision, anchor: row_box(), placed: snooze_at() }
            }
            if shell.read().labelling == Some(id) {
                LabelMenu { id, summary, shell, revision, anchor: row_box(), placed: label_at() }
            }
            if filing {
                MoveMenu {
                    thread: id,
                    shell,
                    revision,
                    anchor: row_box(),
                    placed: move_at(),
                    on_close: move |_| shell.write().filing = None,
                }
            }
        }
    }
}

/// A point in the window as the rect a context menu is placed against.
fn point_rect(at: dioxus::html::geometry::ClientPoint) -> Rect {
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

#[component]
fn ViaChip(via: Provider, marks: Marks) -> Element {
    let short = via.short();
    let title = via.title();
    rsx! {
        Tooltip { text: title,
            span { class: "via",
                ProvChip { provider: via, marks }
                "{short}"
            }
        }
    }
}
