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
use super::picks::{drawn_order, mute_all, with_selection};
use super::text::{draft_state, label, sender};
use crate::provider::Provider;
use crate::provider::icon::{ChipPlace, ProvChip};
use crate::selection::Click;
use crate::view::Marks;
use crate::view::{Shell, hover_in};
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
    let muted = summary.mute == Mute::Muted;
    let follow_up = summary.follow_up;
    let no_reply = crate::follow_up::row_words(&follow_up);
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
    // The saved view being shown names its own strip; anywhere else it is the usual one.
    let mut actions: Vec<OpKind> = hover_in(shell.read().saved_view(), &summary)
        .into_iter()
        .filter(|kind| !matches!(kind, OpKind::Star | OpKind::Unstar))
        .collect();
    // In Trash or Spam, and only there, a row can be deleted forever: once the sheet has asked.
    if crate::destroy::offered(crate::destroy::bin_shown(&shell.read()), &summary) {
        actions.push(OpKind::Destroy);
    }
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
    // The context menu, open at the point the row was right-clicked.
    let mut row_menu = use_signal(|| None::<ds::Rect>);
    // "Remind me if no reply", opened from the context menu at the same point.
    let mut reminding = use_signal(|| None::<ds::Rect>);
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
    // The name and the time open their own cards. Leaving either lets its card go like any
    // other target's leave: the pointer may be on its way to the card, which sits over the rows
    // below. Being back on the row is quire's to say (`onpointerback`, below).
    // The innermost hook wins: an entry that bubbles (a harness's does) stops at the part.
    let part = move |hook: Hook| PartHooks {
        onpointerenter: EventHandler::new(move |event: PointerEvent| {
            event.stop_propagation();
            enter(hook, line_at(&event));
        }),
        onpointerleave: EventHandler::new(move |event: PointerEvent| {
            event.stop_propagation();
            out(driver);
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
        if muted {
            span { class: "mute-mark", title: "Muted", "data-muted": "true",
                Glyph { icon: Icon::BellOff, size: ds::IconSize::Micro }
            }
        }
        if let Some(words) = no_reply {
            span { class: "no-reply", "data-follow-up": "returned",
                Glyph { icon: Icon::Bell, size: ds::IconSize::Micro }
                "{words}"
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
            label: strip_label(kind, muted).to_owned(),
            fly: fly(kind, muted),
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
                // The pointer came back from the name or the time and rested on the row.
                onpointerback: EventHandler::new(move |_: PointerEvent| {
                    enter(Hook::Thread(id), element(row_box()));
                }),
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
            if let Some(at) = row_menu() {
                super::menu::Floating {
                    kind: ds::MenuKind::Context,
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
                    placed: move_at_button(),
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
fn point_rect(at: dioxus::html::geometry::ClientPoint) -> ds::Rect {
    ds::Rect {
        origin: ds::Point {
            x: ds::Px(at.x as f32),
            y: ds::Px(at.y as f32),
        },
        size: ds::Size {
            width: ds::Px(0.0),
            height: ds::Px(0.0),
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
    // An op may take the row out of the list, and a pressed strip button would otherwise have
    // the keyboard inside the leaving row. quire hands the keyboard on when its element is
    // removed, but only if it saw the element focused first: under load the press's own focus
    // and the row's removal land in one frame, and the keyboard went nowhere. So the window
    // takes it back now, before the row can leave (FINDINGS F172).
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
