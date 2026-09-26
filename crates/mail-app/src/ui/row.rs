//! One row of the list.
//!
//! A draft and a conversation are different rows: a draft opens the composer, a conversation
//! opens the reader and carries the hover strip. Split from [`super::app`] (`CONVENTIONS.md` §8).
//!
//! Each row is quire's `ListRow` inside mailo's `.row`, the box the row's own menus and the
//! snooze float are placed against. Its strip is quire's `HoverStrip`, whose `on_press` hears a
//! press before anything is measured, so a row's archive never waits on a layout read.

use super::hover::{Hook, element, line_at, out, over, use_driver};
use super::list_search::RowHit;
use super::marked::{Piece, pieces};
use super::menus::{LabelMenu, SnoozeMenu};
use super::motion::{act_kind, act_kind_all, drag, motion};
use super::move_to::MoveMenu;
use super::ops::{composes, start_composing};
use super::picks::{drawn_order, with_selection};
use super::text::{draft_state, label, sender};
use crate::provider::Provider;
use crate::provider::icon::{ChipPlace, ProvChip};
use crate::selection::Click;
use crate::view::Marks;
use crate::view::{Shell, hover_actions};
use chrono::Local;
use dioxus::prelude::*;
use ds::{
    ActionId, Anim, Emphasis, Exit, Expanded, Glyph, Here, Icon, MountedRef, PartHooks, Presence,
    PulseKey, Run, RunTone, Selection, Shown, StaggerIndex, StripAction, Switch, Text, Titles,
};
use mail_domain::*;
use mail_store::{SqliteStore, Store};
use std::ops::Range;
use std::sync::Arc;

/// A draft, as a row. Clicking it opens the composer on that draft.
#[component]
pub(super) fn DraftRow(draft: Draft, shell: Signal<Shell>, index: usize) -> Element {
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
    let snippet = parts.snippet.shown().then(|| Text::from(state));
    let time = if parts.time.shown() {
        when
    } else {
        String::new()
    };
    rsx! {
        div { key: "{id}", class: "row", role: "none",
            // A draft is not on the roster: it rises with the list whenever the list is shown.
            ds::ListRow {
                selection: Selection::Unselected,
                emphasis: Emphasis::Plain,
                index: StaggerIndex::new(index.min(8)),
                presence: Presence::Entering,
                name: who,
                via: None,
                subject,
                snippet,
                time,
                tags: rsx! {},
                star: None,
                star_pulse: PulseKey::rest(Anim::StarPop),
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

/// What a row is doing besides being there, as the list's roster has it. Each is a fact the
/// window already has: the list was just shown, an op took it out of the list, a row above it
/// has gone, an undo brought it back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum Moving {
    #[default]
    Still,
    /// Arrived, with its entrance stagger. It rises only while the list is being shown.
    Entering(u8),
    /// Drawn after the store dropped it, until its exit settles.
    Going(Exit),
    /// Closing the gap a row above left, with its heal step.
    Healing(u8),
    /// Back from an undo once its exit had settled: it arrives again, as quire's row does.
    Returning,
}

impl Moving {
    /// quire's presence for it, and the stagger it rises by. `delay` is its place in the list,
    /// `dy` how far a healing row travels.
    fn presence(self, delay: usize, dy: u32) -> (Presence, StaggerIndex) {
        match self {
            Moving::Still => (Presence::Present, StaggerIndex::new(delay)),
            Moving::Entering(stagger) => (Presence::Entering, StaggerIndex::new(stagger.into())),
            Moving::Returning => (Presence::Entering, StaggerIndex::new(delay)),
            Moving::Going(exit) => (Presence::Leaving(exit), StaggerIndex::new(delay)),
            Moving::Healing(step) => (
                Presence::Healing {
                    dy: ds::Px(dy as f32),
                    d: StaggerIndex::new(step.into()),
                },
                StaggerIndex::new(delay),
            ),
        }
    }
}

/// `text` as quire's runs, with a search's `marks` as `Mark` runs; plain when nothing is marked.
fn runs(text: &str, marks: &[Range<usize>]) -> Text {
    if marks.is_empty() {
        return Text::Plain(text.to_owned());
    }
    Text::Runs(
        pieces(text, marks)
            .into_iter()
            .map(|piece| match piece {
                Piece::Plain(plain) => Run::new(plain, RunTone::Plain),
                Piece::Marked(inside) => Run::new(inside, RunTone::Mark),
            })
            .collect(),
    )
}

/// A conversation, as a row, including the hover strip and whichever menu it has open.
#[component]
pub(super) fn Row(
    summary: ThreadSummary,
    shell: Signal<Shell>,
    revision: Signal<u64>,
    index: usize,
    chips: Vec<String>,
    via: Option<Provider>,
    hit: Option<RowHit>,
    moving: Moving,
    /// The chip that has just been added, which lands.
    landing: Option<String>,
    /// Whether the row is drawn selected: picked, or open with nothing picked
    /// (`Shell::is_selected`, asked by the list, which knows what is listed).
    selection: Selection,
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
    let delay = index.min(8);
    let move_label = "Move to…".to_owned();
    let filing = shell.read().filing == Some(id);
    // A press on the star replays its pop, and its sparks when it stars: quire's pulse.
    let pop = ds::use_pulse(Anim::StarPop);
    // The strip buttons whose menus float beside them: each hands over its rect once measured,
    // and until then the menu is placed against the row's own box.
    let mut snooze_at = use_signal(|| None::<ds::Rect>);
    let mut move_at_button = use_signal(|| None::<ds::Rect>);
    let mut label_at = use_signal(|| None::<ds::Rect>);
    let mut row_box = use_signal(|| None::<MountedRef>);
    // The focus inside the row shows its strip, as the pointer over it does.
    let mut focused = use_signal(|| false);
    // quire's hover hub, which the row, its name and its time report the pointer to.
    let driver = use_driver();
    let going = matches!(moving, Moving::Going(_));
    let (presence, stagger) = moving.presence(delay, gap(&shell.read()));
    let snoozing = moving == Moving::Going(Exit::Curl);
    let enter = move |hook: Hook, anchor: ds::HoverAnchor| {
        if !going {
            over(driver, hook, anchor);
        }
    };
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
        if starred { Switch::On } else { Switch::Off },
        EventHandler::new(move |_: Switch| {
            pop.fire();
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
                span {
                    key: "{name}",
                    class: if landing.as_deref() == Some(name.as_str()) { "chip is-landing" } else { "chip" },
                    "data-chip": "{name}",
                    "{name}"
                }
            }
        }
        if let Some(count) = files {
            span { class: "clip",
                Glyph { icon: Icon::Paperclip, size: ds::IconSize::Micro }
                "{count}"
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
                EventHandler::new(move |here: Here| {
                    if let Some(mut state) = motion() {
                        state.dest.set((here == Here::Current).then_some(place));
                    }
                })
            }),
            onclick: EventHandler::new(move |rect: ds::Rect| {
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
        onclick: EventHandler::new(move |rect: ds::Rect| move_at_button.set(Some(rect))),
    });
    let open_menus = {
        let read = shell.read();
        let state = |open: bool| {
            if open {
                Expanded::Open
            } else {
                Expanded::Closed
            }
        };
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
        ds::HoverStrip {
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
    rsx! {
        // The box names the hover hook its row is, as every other hook's element does.
        div { key: "{id}", class: "row", role: "none", "data-hc": "thread:{id}",
            onmounted: move |event: MountedEvent| row_box.set(Some(MountedRef(event.data()))),
            onfocusin: move |_| focused.set(true),
            onfocusout: move |_| focused.set(false),
            ds::ListRow {
                selection,
                emphasis: if unread { Emphasis::Strong } else { Emphasis::Plain },
                index: stagger,
                presence,
                name: who,
                via,
                subject: runs(&subject, &subject_marks),
                snippet,
                time,
                tags,
                star: Some(star),
                star_pulse: pop.key(),
                strip,
                onclick: move |click: MouseData| {
                    let click = click_of(click.modifiers());
                    shell.write().click(id, click, &drawn_order());
                },
                on_sender: part(Hook::Sender(id)),
                on_time: part(Hook::Time(id)),
                onpointerenter: EventHandler::new(move |_: PointerEvent| {
                    enter(Hook::Thread(id), element(row_box()));
                }),
                onpointerleave: EventHandler::new(move |_| out(driver)),
                onpointerdown: EventHandler::new(move |event: PointerEvent| {
                    super::hover::press(driver);
                    if !going {
                        let point = event.client_coordinates();
                        drag::press(id, (point.x, point.y));
                    }
                }),
                aria_label: "Open {subject}",
            }
            if snoozing {
                span { class: "floater", aria_hidden: "true", "zZ" }
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
                    placed: move_at_button(),
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
        // A press on a picked row acts on everything picked, as one gesture. An op that needs
        // more than the button (a pin's rank) stays with its own row.
        None if crate::view::op_for(kind).is_some() => {
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

/// How far the rows under a gap travel as they heal: one row, with its margin.
pub(super) fn gap(shell: &Shell) -> u32 {
    if shell.parts.snippet.shown() { 72 } else { 54 }
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
