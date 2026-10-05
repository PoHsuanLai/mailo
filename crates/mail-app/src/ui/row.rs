//! One row of the list.
//!
//! A draft and a conversation are different rows: a draft opens the composer, a conversation
//! opens the reader and carries the hover strip. Split from [`super::app`] (`CONVENTIONS.md` §8).
//!
//! Each row is quire's `ThreadRow` inside mailo's `.row`, the box the row's own menus and the
//! snooze float are placed against. Its strip is quire's `HoverStrip`, whose `on_press` hears a
//! press before anything is measured, so a row's archive never waits on a layout read.

mod reveal;

use super::list_search::RowHit;
use super::marked::{Piece, pieces};
use super::menus::{LabelMenu, SnoozeMenu};
use super::motion::{act_kind, act_kind_all, drag, motion};
use super::move_to::MoveMenu;
use super::ops::{composes, start_composing};
use super::picks::{drawn_order, mute_all, with_selection};
use super::text::{draft_state, label, sender};
use crate::ui::provider_chip::{ChipPlace, ProvChip};
use crate::ui::selection::Click;
use crate::ui::view::Marks;
use crate::ui::view::{Shell, hover_in};
use chrono::Local;
use dioxus::prelude::*;
use ds::base::press::Press;
use ds::base::vocab::RowState;
use ds::components::app::hover_strip::{ActionId, HoverStrip, StripAction, Titles};
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

/// A conversation, as a row, including the hover strip and whichever menu it has open.
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
    // The saved view being shown names its own strip; anywhere else it is the usual one.
    let mut actions: Vec<OpKind> = hover_in(shell.read().saved_view(), &summary)
        .into_iter()
        .filter(|kind| !matches!(kind, OpKind::Star | OpKind::Unstar))
        .collect();
    // In Trash or Spam, and only there, a row can be deleted forever: once the sheet has asked.
    if crate::ui::bin::offered(crate::ui::bin::bin_shown(&shell.read()), &summary) {
        actions.push(OpKind::Destroy);
    }
    let move_label = "Move to…".to_owned();
    let filing = shell.read().filing == Some(id);
    // The strip buttons whose menus float beside them: each hands over its rect once measured,
    // and until then the menu is placed against the row's own box.
    let mut snooze_at = use_signal(|| None::<Rect>);
    let mut label_at = use_signal(|| None::<Rect>);
    let mut move_at = use_signal(|| None::<Rect>);
    let mut row_box = use_signal(|| None::<MountedRef>);
    // The focus inside the row shows its strip, as the pointer over it does.
    let mut focused = use_signal(|| false);
    // The strip is pressable only once the pointer has dwelled on the row (`reveal`); `visit`
    // numbers each arrival so a dwell timer from an earlier one cannot arm a later one.
    let mut armed = use_signal(reveal::Reveal::default);
    let mut visit = use_signal(|| 0_u64);
    // The context menu, open at the point the row was right-clicked.
    let mut row_menu = use_signal(|| None::<Rect>);
    // "Remind me if no reply", opened from the context menu at the same point.
    let mut reminding = use_signal(|| None::<Rect>);
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
            span { class: "mute-mark", title: "Muted", "data-muted": "true",
                Glyph { icon: Icon::BellOff, size: IconSize::Micro }
            }
        }
        if let Some(words) = no_reply {
            span { class: "no-reply", "data-follow-up": "returned",
                Glyph { icon: Icon::Bell, size: IconSize::Micro }
                "{words}"
            }
        }
        if let Some(count) = files {
            span { class: "clip",
                Glyph { icon: Icon::Paperclip, size: IconSize::Micro }
                Badge { content: BadgeContent::Number(count), tone: BadgeTone::Quiet, size: ControlSize::Mini }
            }
        }
    };
    // The strip is quire's. A press acts inside the click, before anything is measured, so an
    // archive never waits on a layout read; the snooze and move buttons' rects follow, and the
    // menus they open are placed against them (against the row until they arrive).
    let mut strip_actions: Vec<StripAction> = actions
        .iter()
        .copied()
        .map(|kind| StripAction {
            id: ActionId(kebab(kind).to_owned()),
            icon: op_icon(kind),
            label: strip_label(kind, muted).to_owned(),
            fly: fly(kind, muted),
            onhover: preview(kind).map(|place| {
                EventHandler::new(move |here: Selection| {
                    if let Some(mut state) = motion() {
                        state
                            .dest
                            .set((here == Selection::Selected).then_some(place));
                    }
                })
            }),
            onclick: EventHandler::new(move |rect: Rect| {
                if kind == OpKind::Snooze {
                    snooze_at.set(Some(rect));
                }
                if kind == OpKind::AddLabel {
                    label_at.set(Some(rect));
                }
            }),
        })
        .collect();
    strip_actions.push(StripAction {
        id: ActionId(MOVE_TO.to_owned()),
        icon: Icon::FolderInput,
        label: move_label.clone(),
        fly: move_label.clone(),
        onhover: None,
        onclick: EventHandler::new(move |rect: Rect| move_at.set(Some(rect))),
    });
    let open_menus = {
        let read = shell.read();
        let state = |open: bool| if open { Shown::Visible } else { Shown::Hidden };
        vec![
            (
                ActionId(kebab(OpKind::Snooze).to_owned()),
                state(read.snoozing == Some(id)),
            ),
            (
                ActionId(kebab(OpKind::AddLabel).to_owned()),
                state(read.labelling == Some(id)),
            ),
            (ActionId(MOVE_TO.to_owned()), state(filing)),
        ]
    };
    let kinds = actions.clone();
    let strip = rsx! {
        HoverStrip {
            actions: strip_actions,
            shown: Some(reveal::shown(
                armed(),
                if focused() { reveal::Focus::Within } else { reveal::Focus::Outside },
            )),
            titles: Titles::FromLabel,
            expanded: open_menus,
            on_press: move |pressed: ActionId| {
                let which = if pressed.0 == MOVE_TO {
                    Some(Pressed::MoveTo)
                } else {
                    kinds.iter().copied().find(|kind| kebab(*kind) == pressed.0).map(Pressed::Op)
                };
                if let Some(which) = which {
                    press(shell, revision, id, which);
                }
            },
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
            onfocusin: move |_| focused.set(true),
            onfocusout: move |_| focused.set(false),
            oncontextmenu: move |event: MouseEvent| {
                event.prevent_default();
                row_menu.set(Some(point_rect(event.client_coordinates())));
            },
            // Shift+Enter on the focused row opens it in a window of its own. Stopped here, so
            // the window's own Shift+Enter does not open the open conversation as well.
            onkeydown: move |event: KeyboardEvent| {
                if event.key().to_string() == "Enter" && event.modifiers().shift() {
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
                strip,
                onclick: move |press: Press| {
                    let click = click_of(press.modifiers);
                    shell.write().click(id, click, &drawn_order());
                },
                onpointerenter: EventHandler::new(move |_: PointerEvent| {
                    armed.set(reveal::next(armed(), reveal::Pointer::Entered));
                    let here = visit() + 1;
                    visit.set(here);
                    spawn(async move {
                        ds::base::time::clock::sleep(reveal::DWELL).await;
                        if visit() == here {
                            armed.set(reveal::next(armed(), reveal::Pointer::Dwelled));
                        }
                    });
                }),
                onpointerleave: EventHandler::new(move |_: PointerEvent| {
                    visit.set(visit() + 1);
                    armed.set(reveal::next(armed(), reveal::Pointer::Left));
                }),
                onpointerdown: EventHandler::new(move |event: PointerEvent| {
                    let point = event.client_coordinates();
                    drag::press(id, (point.x, point.y));
                }),
                common: Common {
                    aria_label: Some(format!("Open {subject}")),
                    ..Common::default()
                },
            }
            if let Some(at) = row_menu() {
                super::menu::Floating {
                    anchor: row_box(),
                    placed: Some(at),
                    title: String::new(),
                    items: vec![super::window::menu_item(), remind_item()],
                    on_pick: move |key: String| {
                        let at = row_menu();
                        row_menu.set(None);
                        if key == super::window::OPEN_KEY {
                            super::window::open_in_window(id);
                        }
                        if key == REMIND_KEY {
                            reminding.set(at);
                        }
                    },
                    on_close: move |_| row_menu.set(None),
                }
            }
            if let Some(at) = reminding() {
                super::follow_up::FollowUpMenu {
                    id,
                    current: follow_up,
                    shell,
                    revision,
                    anchor: row_box(),
                    placed: Some(at),
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

/// The context menu's key for "Remind me if no reply…".
const REMIND_KEY: &str = "remind-me";

/// The context menu's row that opens the follow-up menu.
fn remind_item() -> super::menu::MenuItem {
    super::menu::MenuItem {
        key: REMIND_KEY.to_owned(),
        tile: super::menu::Tile::Icon(Icon::Bell),
        name: "Remind me if no reply…".to_owned(),
        help: None,
        right: super::menu::Right::None,
        group: None,
        marks: Vec::new(),
        title: Vec::new(),
        detail: Vec::new(),
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

/// The place a strip button would send the row to, which glows while the button is hovered.
fn preview(kind: OpKind) -> Option<&'static str> {
    match kind {
        OpKind::Archive => Some("Archive"),
        OpKind::Snooze => Some("Snoozed"),
        OpKind::Trash => Some("Trash"),
        _ => None,
    }
}

/// The strip's name for its Move to… button.
const MOVE_TO: &str = "move-to";

/// Which strip button was pressed, read back from its name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pressed {
    Op(OpKind),
    MoveTo,
}

/// A strip button pressed: the op it names, at once. Label, snooze and move open their menus
/// (a second press closes them); a reply opens the composer on its draft.
fn press(mut shell: Signal<Shell>, mut revision: Signal<u64>, id: ThreadId, pressed: Pressed) {
    let kind = match pressed {
        Pressed::Op(kind) => kind,
        Pressed::MoveTo => {
            let already = shell.peek().filing == Some(id);
            shell.write().filing = if already { None } else { Some(id) };
            return;
        }
    };
    let store = consume_context::<Arc<SqliteStore>>();
    if kind == OpKind::AddLabel {
        let already = shell.peek().labelling == Some(id);
        shell.write().labelling = if already { None } else { Some(id) };
        return;
    }
    if kind == OpKind::Snooze {
        let already = shell.peek().snoozing == Some(id);
        shell.write().snoozing = if already { None } else { Some(id) };
        return;
    }
    // Never at once: the sheet names how much and asks, over this row and every one picked.
    if kind == OpKind::Destroy {
        super::destroy::ask_chosen(&store, shell, &with_selection(shell, id));
        return;
    }
    // An op may take the row out of the list. Take the keyboard back before it can leave
    // with the row (FINDINGS F172).
    if composes(kind).is_none() {
        super::host::Host::focus_app();
    }
    // Mute takes its direction from the conversations it reaches, so a picked row mutes or
    // unmutes the whole selection as one gesture.
    if kind == OpKind::Mute {
        mute_all(&store, shell, revision, &with_selection(shell, id));
        return;
    }
    match composes(kind) {
        Some(what) => match start_composing(&store, id, what) {
            Ok(draft) => {
                shell.write().compose(&draft);
                revision += 1;
            }
            Err(why) => eprintln!("reply: {why}"),
        },
        // A press on a picked row acts on everything picked, as one gesture. An op that needs
        // more than the button (a pin's rank) stays with its own row.
        None if crate::ui::view::op_for(kind).is_some() => {
            act_kind_all(&store, shell, revision, &with_selection(shell, id), kind);
        }
        None => {
            act_kind(&store, shell, revision, id, kind);
        }
    }
}

/// What a click on a row asks for, from the keys held with it. Shift wins over Ctrl, as a
/// range is the larger thing to have asked for; Cmd is a Mac's Ctrl.
fn click_of(held: Modifiers) -> Click {
    if held.shift() {
        Click::Range
    } else if held.ctrl() || held.meta() {
        Click::Toggle
    } else {
        Click::Plain
    }
}

#[component]
fn ViaChip(via: Provider, marks: Marks) -> Element {
    let short = via.short();
    let title = via.title();
    rsx! {
        span { class: "via", title: "{title}",
            ProvChip { provider: via, marks, place: ChipPlace::Row }
            "{short}"
        }
    }
}

/// `tomorrow` in [`mail_core::snooze::snooze_until`] is 09:00 local, which is what this says.
/// The mockup's card says 08:00; the menu and the command line both mean 09:00.
fn fly(kind: OpKind, muted: bool) -> String {
    match kind {
        OpKind::Snooze => "Tomorrow 09:00".to_owned(),
        OpKind::Archive => "Archive → out of Inbox".to_owned(),
        OpKind::Mute if muted => "Unmute → replies to the inbox".to_owned(),
        OpKind::Mute => "Mute → replies skip the inbox".to_owned(),
        other => strip_label(other, muted).to_owned(),
    }
}

/// A strip button's name. Mute says what pressing it does to this row, as Read and Star do by
/// being two kinds.
fn strip_label(kind: OpKind, muted: bool) -> &'static str {
    match kind {
        OpKind::Mute if muted => "Unmute",
        other => label(other),
    }
}

fn kebab(kind: OpKind) -> &'static str {
    match kind {
        OpKind::Archive => "archive",
        OpKind::Trash => "trash",
        OpKind::Restore => "restore",
        OpKind::Spam => "spam",
        OpKind::MarkRead => "mark-read",
        OpKind::MarkUnread => "mark-unread",
        OpKind::Star => "star",
        OpKind::Unstar => "unstar",
        OpKind::AddLabel => "add-label",
        OpKind::RemoveLabel => "remove-label",
        OpKind::Snooze => "snooze",
        OpKind::Pin => "pin",
        OpKind::Mute => "mute",
        OpKind::FollowUp => "remind-me",
        OpKind::Destroy => "delete-forever",
        OpKind::Reply => "reply",
        OpKind::ReplyAll => "reply-all",
        OpKind::Forward => "forward",
    }
}

fn op_icon(kind: OpKind) -> Icon {
    match kind {
        OpKind::Archive => Icon::Archive,
        OpKind::Trash => Icon::Trash,
        OpKind::Restore => Icon::Corner,
        OpKind::Spam => Icon::OctagonAlert,
        OpKind::MarkRead => Icon::MailOpen,
        OpKind::MarkUnread => Icon::Mail,
        OpKind::Star | OpKind::Unstar => Icon::Star,
        OpKind::AddLabel | OpKind::RemoveLabel => Icon::Tag,
        OpKind::Snooze => Icon::Clock,
        OpKind::Pin => Icon::Pin,
        OpKind::Mute => Icon::BellOff,
        OpKind::FollowUp => Icon::Bell,
        OpKind::Destroy => Icon::Trash,
        OpKind::Reply => Icon::Reply,
        OpKind::ReplyAll => Icon::ReplyAll,
        OpKind::Forward => Icon::Forward,
    }
}
