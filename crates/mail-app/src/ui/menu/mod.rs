//! One menu for every list of choices.
//!
//! A title row, then items. Each item has a tile, a name, one line of help, and a shortcut or a
//! check on the right. The arrow keys, Enter and Esc live on [`MenuState`], so the table in the
//! tests never needs a document.

mod state;

use super::field::{Field, FieldKind};
use super::icon::{Glyph, Icon};
use dioxus::prelude::*;
pub(super) use state::{
    MenuEvent, MenuItem, MenuKey, MenuState, Piece, Right, Run, Shown, Tile, Tone, menu_key, pieces,
};

/// A header or an item, in the order the menu paints them.
enum Row {
    Header(String),
    Item(usize),
}

fn rows(shown: &[Shown<'_>]) -> Vec<Row> {
    let mut out = Vec::new();
    let mut last: Option<&str> = None;
    for (index, shown) in shown.iter().enumerate() {
        if let Some(group) = shown.item.group.as_deref()
            && last != Some(group)
        {
            out.push(Row::Header(group.to_owned()));
            last = Some(group);
        }
        out.push(Row::Item(index));
    }
    out
}

/// The shared menu. `active` set means the parent owns the cursor (the command menu's field
/// sits outside this element). Otherwise the menu owns its [`MenuState`].
#[component]
pub(super) fn Menu(
    title: String,
    items: Vec<MenuItem>,
    filterable: bool,
    on_pick: EventHandler<String>,
    on_close: EventHandler<()>,
    on_query: EventHandler<String>,
    slim: bool,
    active: Option<usize>,
) -> Element {
    let mut state = use_signal(|| MenuState::new(filterable));
    let mut held = use_signal(|| items.clone());
    if held.read().as_slice() != items.as_slice() {
        held.set(items.clone());
    }
    let live = held.read().clone();
    let shown = state.read().shown(&live);
    let cursor = active.unwrap_or_else(|| state.read().active().min(shown.len().saturating_sub(1)));
    let class = if slim { "fmenu slim" } else { "fmenu" };
    let owned = active.is_none();
    let painted = rows(&shown);
    let filter_value = state.read().query().to_owned();
    rsx! {
        div {
            class: "{class}",
            role: "listbox",
            tabindex: "0",
            onmousedown: move |event| {
                event.prevent_default();
                event.stop_propagation();
            },
            onkeydown: move |event| {
                if !owned {
                    return;
                }
                let Some(key) = menu_key(&event.key().to_string()) else {
                    return;
                };
                // Letters belong to the field. Handling them here as well would type each one twice.
                if matches!(key, MenuKey::Character(_) | MenuKey::Backspace) {
                    return;
                }
                event.stop_propagation();
                let current: Vec<MenuItem> = state
                    .read()
                    .shown(&held.read())
                    .into_iter()
                    .map(|shown| shown.item.clone())
                    .collect();
                match state.write().on_key(key, &current) {
                    MenuEvent::Pick(key) => on_pick.call(key),
                    MenuEvent::Close => on_close.call(()),
                    MenuEvent::Moved | MenuEvent::Typed | MenuEvent::Ignored => {}
                }
            },
            if !title.is_empty() {
                div { class: "g", "{title}" }
            }
            if filterable {
                Field {
                    kind: FieldKind::Inline,
                    value: filter_value,
                    placeholder: "Filter".to_owned(),
                    extra: None,
                    on_input: move |value: String| {
                        state.write().set_query(value.clone());
                        on_query.call(value);
                    },
                    on_focus: |_| {},
                    on_blur: |_| {},
                }
            }
            if shown.is_empty() {
                div { class: "none", "Nothing matches." }
            }
            for (n, row) in painted.into_iter().enumerate() {
                match row {
                    Row::Header(group) => rsx! { div { key: "g-{n}-{group}", class: "g", "{group}" } },
                    Row::Item(index) => {
                        let shown = &shown[index];
                        let key = shown.item.key.clone();
                        let indices = if shown.indices.is_empty() {
                            shown.item.marks.clone()
                        } else {
                            shown.indices.clone()
                        };
                        let title = if shown.item.title.is_empty() {
                            vec![Run {
                                text: shown.item.name.clone(),
                                marks: indices,
                                tone: Tone::Plain,
                            }]
                        } else {
                            shown.item.title.clone()
                        };
                        let detail = if shown.item.detail.is_empty() {
                            shown
                                .item
                                .help
                                .clone()
                                .map(|text| Run {
                                    text,
                                    marks: Vec::new(),
                                    tone: Tone::Plain,
                                })
                                .into_iter()
                                .collect()
                        } else {
                            shown.item.detail.clone()
                        };
                        let selected = index == cursor;
                        let checked = matches!(shown.item.right, Right::Check(true));
                        rsx! {
                            div {
                                key: "{key}",
                                class: "it",
                                role: "option",
                                aria_selected: if selected { "true" } else { "false" },
                                aria_checked: if checked { "true" } else { "false" },
                                onclick: move |event| {
                                    event.stop_propagation();
                                    on_pick.call(key.clone());
                                },
                                {tile(&shown.item.tile)}
                                span {
                                    b { {runs(&title)} }
                                    if !detail.is_empty() {
                                        small { class: "snip", {runs(&detail)} }
                                    }
                                }
                                span { class: "sc",
                                    match &shown.item.right {
                                        Right::Shortcut(shortcut) => rsx! { "{shortcut}" },
                                        Right::Check(true) => rsx! { Glyph { icon: Icon::Check, class: None } },
                                        Right::Check(false) | Right::None => rsx! { "" },
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn runs(parts: &[Run]) -> Element {
    rsx! {
        for (index, run) in parts.iter().enumerate() {
            span { key: "{index}", class: tone_class(run.tone),
                for (piece_index, piece) in pieces(&run.text, &run.marks).into_iter().enumerate() {
                    match piece {
                        Piece::Plain(text) => rsx! { span { key: "p{piece_index}", "{text}" } },
                        Piece::Mark(text) => rsx! { mark { key: "m{piece_index}", "{text}" } },
                    }
                }
            }
        }
    }
}

fn tone_class(tone: Tone) -> &'static str {
    match tone {
        Tone::Plain => "",
        Tone::Strong => "who",
        Tone::Faint => "addr",
    }
}

fn tile(tile: &Tile) -> Element {
    match tile {
        Tile::Icon(icon) => rsx! { span { class: "tile", Glyph { icon: *icon, class: None } } },
        Tile::Avatar { letter, color } => rsx! {
            span {
                class: "tile round",
                style: "background:{color};border-color:transparent",
                "{letter}"
            }
        },
        Tile::Glyph(ch) => rsx! { span { class: "tile", "{ch}" } },
    }
}

#[cfg(test)]
mod tests;
