//! Emoji in the body: the picker `/` › Emoji… opens at the caret, and what a pick from either the
//! picker or the `:` menu does beyond the document, which is to remember the emoji. The system's
//! own picker types into the body as any text does.
//!
//! The picker is quire's popover at the caret (`body.rs`): a search field that keeps the keyboard, a tab
//! per group with the recent emoji first, and quire's `EmojiGrid`, whose glyphs are drawn in
//! colour through `.ds-emoji-text` where the system's Noto Color Emoji is the COLRv1 build (a
//! CBDT one, Ubuntu's, is not drawn at all). The arrows move through the grid from the field, Enter
//! picks, and Escape closes; a pick goes in at the body's caret as one undo step
//! ([`super::float::insert_emoji`]) and the body takes the keyboard back.

use dioxus::prelude::*;
use ds::components::controls::segmented::Tracking;
use ds::components::lists::emoji_grid::grid::EmojiCell;
use ds::components::lists::emoji_grid::nav::{GridMove, GridStep, grid_step};
use ds::prelude::{Choice, EmojiGrid, FieldFocus, FieldKind, Px, SegmentedControl, TextField};

use super::desk::Desk;
use super::float::{insert_emoji, pick_emoji};
use super::page::Page;
use crate::ui::emoji::{self, Emoji, Group, recent};
use crate::ui::host::Host;

/// Cells per row of the picker's grid: as wide as the ten tabs over it.
const COLUMNS: u8 = 10;

/// A cell's side: quire's glyph is 30 px, and this leaves it room.
const CELL: Px = Px(50.0);

/// The most cells a search shows.
const FOUND: usize = 64;

/// What the picker's grid shows when nothing is typed in its search field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Recent,
    Group(Group),
}

impl Tab {
    /// The tabs, recent first.
    fn all() -> Vec<(Tab, String)> {
        std::iter::once((Tab::Recent, "🕘".to_owned()))
            .chain(
                Group::ALL
                    .into_iter()
                    .map(|group| (Tab::Group(group), group.face().to_owned())),
            )
            .collect()
    }
}

/// `emoji` goes to the front of the recent ones, in the desk and in `emoji.json`.
fn remember(emoji: &'static Emoji) {
    let Some(mut desk) = try_consume_context::<Desk>() else {
        return;
    };
    let next = recent::remember(&desk.emoji.peek(), emoji);
    if let Some(dirs) = desk.dirs.peek().as_ref()
        && let Err(why) = recent::save(&dirs.state, &next)
    {
        eprintln!("emoji: {why}");
    }
    desk.emoji.set(next);
}

/// The `:` menu's pick: the typed name becomes the emoji, which is remembered.
pub(super) fn pick_typed(mut page: Signal<Page>, glyph: &str) {
    let picked = pick_emoji(&mut page.write(), glyph);
    if let Some(emoji) = picked {
        remember(emoji);
    }
}

/// The picker's content: the field, the tabs, the grid, and the name of the emoji selected.
#[component]
pub(super) fn Picker(page: Signal<Page>, on_done: EventHandler<()>) -> Element {
    let recent: Vec<&'static Emoji> = try_use_context::<Desk>()
        .map(|desk| desk.emoji.read().clone())
        .unwrap_or_default();
    let first_tab = if recent.is_empty() {
        Tab::Group(Group::Smileys)
    } else {
        Tab::Recent
    };
    let mut tab = use_signal(|| first_tab);
    let mut query = use_signal(String::new);
    let mut selected = use_signal(|| Some(0usize));
    let typed = query();
    let cells: Vec<&'static Emoji> = if typed.trim().is_empty() {
        match tab() {
            Tab::Recent => recent,
            Tab::Group(group) => emoji::in_group(group).collect(),
        }
    } else {
        emoji::search(&typed).into_iter().take(FOUND).collect()
    };
    let chosen = selected().and_then(|at| cells.get(at).copied());
    let pick = move |emoji: &'static Emoji| {
        let mut page = page;
        if insert_emoji(&mut page.write(), emoji) {
            remember(emoji);
        }
        on_done.call(());
    };
    let count = cells.len();
    let keys = move |event: KeyboardEvent| {
        let step = match event.key() {
            Key::ArrowUp => GridStep::Up,
            Key::ArrowDown => GridStep::Down,
            Key::ArrowLeft => GridStep::Left,
            Key::ArrowRight => GridStep::Right,
            Key::Enter => {
                event.prevent_default();
                if let Some(emoji) = chosen {
                    pick(emoji);
                }
                return;
            }
            _ => return,
        };
        event.prevent_default();
        if count == 0 {
            return;
        }
        let from = selected.peek().unwrap_or(0);
        if let GridMove::To(next) = grid_step(from, step, count, COLUMNS) {
            selected.set(Some(next));
            Host::scroll_into_view(".em-cells [*|aria-selected=true]");
        }
    };
    let grid: Vec<EmojiCell<&'static str>> = cells
        .iter()
        .map(|emoji| EmojiCell {
            value: emoji.glyph,
            glyph: emoji.glyph.to_owned(),
            name: emoji.name.to_owned(),
        })
        .collect();
    let empty = if typed.trim().is_empty() {
        "Emoji you pick appear here.".to_owned()
    } else {
        format!("No emoji called “{}”.", typed.trim())
    };
    rsx! {
        div { class: "em-picker",
            TextField {
                label: "Search emoji".to_owned(),
                value: typed.clone(),
                placeholder: "Search emoji".to_owned(),
                kind: FieldKind::Search,
                tokens: Vec::new(),
                oninput: move |value: String| {
                    query.set(value);
                    selected.set(Some(0));
                },
                onkey: keys,
                focus: FieldFocus::OnMount,
            }
            div { class: "em-tabs",
                SegmentedControl {
                    label: "Emoji groups".to_owned(),
                    choices: Tab::all()
                        .into_iter()
                        .map(|(tab, face)| Choice::new(tab, face))
                        .collect::<Vec<_>>(),
                    tracking: Tracking::SelectOne(tab()),
                    onchange: move |to: Tab| {
                        tab.set(to);
                        query.set(String::new());
                        selected.set(Some(0));
                    },
                }
            }
            if count == 0 {
                p { class: "em-none", "{empty}" }
            } else {
                div { class: "em-cells",
                    EmojiGrid::<&'static str> {
                        cells: grid,
                        onpick: move |glyph: &'static str| {
                            if let Some(emoji) = emoji::find(glyph) {
                                pick(emoji);
                            }
                        },
                        columns: COLUMNS,
                        cell: CELL,
                        selected: selected(),
                        on_select: move |at: usize| selected.set(Some(at)),
                        label: "Emoji to pick".to_owned(),
                    }
                }
            }
            p { class: "em-name", {chosen.map(|emoji| emoji.name).unwrap_or("")} }
        }
    }
}
