//! One row of the list.
//!
//! A draft and a conversation are different rows: a draft opens the composer, a conversation
//! opens the reader and carries the hover strip. Split from [`super::app`] (`CONVENTIONS.md` §8).
//!
//! Each row is quire's `ThreadRow` inside mailo's `.row`, the box the row's own menus and the
//! snooze float are placed against. Its strip is quire's `HoverStrip`, whose `on_press` hears a
//! press before anything is measured, so a row's archive never waits on a layout read.

use super::hover::{Hook, element, line_at, out, over, use_driver};
use super::list_search::RowHit;
use super::marked::{Piece, pieces};
use super::menus::{LabelMenu, SnoozeMenu};
use super::motion::{act_kind, drag, motion};
use super::move_to::MoveMenu;
use super::ops::{composes, start_composing};
use super::text::{draft_state, label, sender};
use crate::provider::Provider;
use crate::provider::icon::{ChipPlace, ProvChip};
use crate::view::Marks;
use crate::view::{Shell, hover_actions};
use chrono::Local;
use dioxus::prelude::*;
use ds::base::vocab::RowState;
use ds::components::app::hover_strip::{ActionId, HoverStrip, StripAction, Titles};
use ds::components::app::thread_row::ThreadRow;
use ds::components::app::thread_row_hooks::PartHooks;
use ds::components::content::text_runs::{RunTone, TextRun};
use ds::components::controls::badge::{Badge, BadgeContent, BadgeTone};
use ds::components::controls::chip::{Chip, ChipVariant};
use ds::components::overlays::hover_card::intent::HoverAnchor;
use ds::host::measure::MountedRef;
use ds::prelude::*;
use ds::root::common::Common;
use ds::style::icon::render::Glyph;
use ds::style::tokens::control_size::ControlSize;
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
    let who = crate::view::join_addresses(&draft.to);
    let who = if who.is_empty() {
        "(no recipient)".to_owned()
    } else {
        who
    };
    let state = draft_state(&draft.state);
    let when = crate::view::listed(draft.updated, chrono::Utc::now(), &Local);
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
) -> Element {
    let id = summary.id;
    let unread = summary.read == ReadState::Unread;
    let starred = summary.star == Star::Starred;
    let who = sender(&summary);
    let when = crate::view::listed(summary.last_date, chrono::Utc::now(), &Local);
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
    let actions: Vec<OpKind> = hover_actions(&summary)
        .into_iter()
        .filter(|kind| !matches!(kind, OpKind::Star | OpKind::Unstar))
        .collect();
    let selected = shell.read().open == Some(id);
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
    // quire's hover hub, which the row, its name and its time report the pointer to.
    let driver = use_driver();
    let enter = move |hook: Hook, anchor: HoverAnchor| over(driver, hook, anchor);
    // The name and the time open their own cards; leaving either is being back on the row.
    // The innermost hook wins: an entry that bubbles (a harness's does) stops at the part.
    let part = move |hook: Hook| PartHooks {
        onpointerenter: EventHandler::new(move |event: PointerEvent| {
            event.stop_propagation();
            enter(hook, line_at(&event));
        }),
        onpointerleave: EventHandler::new(move |event: PointerEvent| {
            event.stop_propagation();
            enter(Hook::Thread(id), element(row_box()));
        }),
    };
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
                Chip { key: "{name}", variant: ChipVariant::Accent, text: name.clone() }
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
            label: label(kind).to_owned(),
            fly: fly(kind),
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
            shown: focused().then_some(Shown::Visible),
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
        selection: if selected {
            Selection::Selected
        } else {
            Selection::Unselected
        },
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
                onclick: move |_| shell.write().open(id),
                on_sender: part(Hook::Sender(id)),
                on_time: part(Hook::Time(id)),
                onpointerenter: EventHandler::new(move |_: PointerEvent| {
                    enter(Hook::Thread(id), element(row_box()));
                }),
                onpointerleave: EventHandler::new(move |_| out(driver)),
                onpointerdown: EventHandler::new(move |event: PointerEvent| {
                    super::hover::press(driver);
                    let point = event.client_coordinates();
                    drag::press(id, (point.x, point.y));
                }),
                // The row names the hover hook it is, as every other hook's element does.
                common: Common {
                    aria_label: Some(format!("Open {subject}")),
                    ..super::sidebar::tagged("hc", format!("thread:{id}"))
                },
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
    match composes(kind) {
        Some(what) => match start_composing(&store, id, what) {
            Ok(draft) => {
                shell.write().compose(&draft);
                revision += 1;
            }
            Err(why) => eprintln!("reply: {why}"),
        },
        None => {
            act_kind(&store, shell, revision, id, kind);
        }
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

/// `tomorrow` in [`crate::view::snooze_until`] is 09:00 local, which is what this says.
/// The mockup's card says 08:00; the menu and the command line both mean 09:00.
fn fly(kind: OpKind) -> String {
    match kind {
        OpKind::Snooze => "Tomorrow 09:00".to_owned(),
        OpKind::Archive => "Archive → out of Inbox".to_owned(),
        other => label(other).to_owned(),
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
        OpKind::Reply => Icon::Reply,
        OpKind::ReplyAll => Icon::ReplyAll,
        OpKind::Forward => Icon::Forward,
    }
}
