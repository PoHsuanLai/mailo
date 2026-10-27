//! The paragraph and text styles the format bar and the keys set: alignment, indentation, quote
//! level, size and colour. Each is `rich_text::` edits over the selection (or the caret's paragraph),
//! committed as one undo step with the selection kept, as the marks are. A size or a colour with
//! nothing selected is what the next typed text takes.

use dioxus::prelude::*;
use ds::components::controls::button_model::{Bezel, ImagePosition};
use ds::components::controls::radio_group::Arrangement;
use ds::components::menus::pull_down_button::PullDownButton;
use ds::prelude::*;
use ds::root::common::Common;

use super::super::menu::{MenuItem, Right, menu_items};
use super::super::press::on_primary;
use super::float::commit;
use super::page::{Float, Page};
use crate::ui::actions::{Own, hint_own};
use crate::ui::copy;
use mail_core::rich_text::{
    Align, Marks, Nest, Op, Range, TextColor, TextSize, align_at, indent, marks_at_start, quote,
    step_size, tab,
};

/// The selection, or the caret as an empty range.
fn target(page: &Page) -> Range {
    let caret = page.session.caret.pos;
    page.selection.unwrap_or(Range {
        start: caret,
        end: caret,
    })
}

/// The marks the next typed text takes at the caret `range`: those already chosen for it, else
/// the text's before it.
fn typing(page: &Page, range: Range) -> Marks {
    page.session
        .pending
        .clone()
        .unwrap_or_else(|| marks_at_start(&page.session.doc, range))
}

/// `ops` as one undo step, the caret and the selection where they were.
fn keep(mut page: Signal<Page>, ops: Vec<Op>) {
    if ops.is_empty() {
        return;
    }
    let (selection, caret) = {
        let read = page.peek();
        (read.selection, read.session.caret)
    };
    let mut write = page.write();
    if commit(&mut write, ops, caret) {
        write.selection = selection;
    }
}

/// Align every paragraph the selection touches.
pub(super) fn align(page: Signal<Page>, align: Align) -> bool {
    let range = target(&page.peek());
    keep(page, vec![Op::SetAlign { range, align }]);
    true
}

/// Indent or outdent every paragraph the selection touches: ⌘] and ⌘[.
pub(super) fn indentation(page: Signal<Page>, nest: Nest) -> bool {
    let ops = {
        let read = page.peek();
        indent(&read.session.doc, target(&read), nest)
    };
    keep(page, ops);
    true
}

/// Quote every paragraph the selection touches a level more or less: ⌘' and ⌥⌘'.
pub(super) fn quote_level(page: Signal<Page>, nest: Nest) -> bool {
    let ops = {
        let read = page.peek();
        quote(&read.session.doc, target(&read), nest)
    };
    keep(page, ops);
    true
}

/// Tab and Shift-Tab: in a list item, nest it or bring it out, and the key is taken even when the
/// item can go no further; anywhere else the key is not the body's.
pub(super) fn tab_key(page: Signal<Page>, nest: Nest) -> bool {
    let ops = {
        let read = page.peek();
        tab(&read.session.doc, target(&read), nest)
    };
    match ops {
        Some(ops) => {
            keep(page, ops);
            true
        }
        None => false,
    }
}

/// Size the selection; with none, the text typed next.
pub(super) fn size(mut page: Signal<Page>, size: Option<TextSize>) -> bool {
    let range = target(&page.peek());
    if range.is_collapsed() {
        let mut marks = typing(&page.peek(), range);
        marks.size = size;
        page.write().session.pending = Some(marks);
    } else {
        keep(page, vec![Op::SetSize { range, size }]);
    }
    true
}

/// A size bigger or smaller than the one the selection starts with: ⌘> and ⌘<.
pub(super) fn step(page: Signal<Page>, nest: Nest) -> bool {
    let now = {
        let read = page.peek();
        let range = target(&read);
        if range.is_collapsed() {
            typing(&read, range).size
        } else {
            marks_at_start(&read.session.doc, range).size
        }
    };
    size(page, step_size(now, nest))
}

/// Colour the selection; with none, the text typed next.
pub(super) fn colour(mut page: Signal<Page>, color: Option<TextColor>) -> bool {
    let range = target(&page.peek());
    if range.is_collapsed() {
        let mut marks = typing(&page.peek(), range);
        marks.color = color;
        page.write().session.pending = Some(marks);
    } else {
        keep(page, vec![Op::SetColor { range, color }]);
    }
    true
}

/// The keys a size, an alignment and a pull-down row are picked by.
const SMALL: &str = "small";
const NORMAL: &str = "normal";
const LARGE: &str = "large";
const HUGE: &str = "huge";
const LEFT: &str = "align-left";
const CENTER: &str = "align-center";
const RIGHT: &str = "align-right";
const INDENT: &str = "indent";
const OUTDENT: &str = "outdent";
const QUOTE_MORE: &str = "quote-more";
const QUOTE_LESS: &str = "quote-less";

/// Each size with its key and its name, smallest first.
const SIZES: [(Option<TextSize>, &str, &str); 4] = [
    (Some(TextSize::Small), SMALL, copy::menus::SIZE_SMALL),
    (None, NORMAL, copy::menus::SIZE_NORMAL),
    (Some(TextSize::Large), LARGE, copy::menus::SIZE_LARGE),
    (Some(TextSize::Huge), HUGE, copy::menus::SIZE_HUGE),
];

/// Each colour with its name, no colour first.
const COLOURS: [(Option<TextColor>, &str); 7] = [
    (None, copy::menus::NO_TEXT_COLOUR),
    (Some(TextColor::Gray), copy::menus::GREY),
    (Some(TextColor::Red), copy::menus::RED),
    (Some(TextColor::Orange), copy::menus::ORANGE),
    (Some(TextColor::Green), copy::menus::GREEN),
    (Some(TextColor::Blue), copy::menus::BLUE),
    (Some(TextColor::Purple), copy::menus::PURPLE),
];

/// The custom property a colour's swatch is painted with: the palette's own, set on the row
/// ([`palette_vars`]); no colour is the text's own ink.
fn swatch_var(color: Option<TextColor>) -> String {
    match color {
        None => "--ink".to_owned(),
        Some(color) => format!("--tc-{}", color.css().trim_start_matches('#')),
    }
}

/// The palette as the custom properties [`swatch_var`] names, for the swatch row's `style`.
fn palette_vars() -> String {
    TextColor::ALL
        .into_iter()
        .map(|color| format!("{}:{}", swatch_var(Some(color)), color.css()))
        .collect::<Vec<_>>()
        .join(";")
}

/// The sizes, the one the selection starts with checked.
fn size_items(now: Option<TextSize>) -> Vec<MenuItem> {
    SIZES
        .iter()
        .map(|(size, key, name)| MenuItem {
            right: Right::Check(*size == now),
            ..MenuItem::plain(*key, *name)
        })
        .collect()
}

/// The Paragraph pull-down: the alignments, the current one checked, then the levels with the
/// keys that do them as `keymap` has them.
fn paragraph_items(now: Align, keymap: &chordkit::Keymap) -> Vec<MenuItem> {
    let aligns = [
        (Align::Left, LEFT, copy::menus::ALIGN_LEFT),
        (Align::Center, CENTER, copy::menus::ALIGN_CENTER),
        (Align::Right, RIGHT, copy::menus::ALIGN_RIGHT),
    ]
    .into_iter()
    .map(|(align, key, name)| MenuItem {
        right: Right::Check(align == now),
        group: Some(copy::headers::ALIGNMENT.to_owned()),
        ..MenuItem::plain(key, name)
    });
    let levels = [
        (Own::Indent, INDENT, copy::menus::INDENT),
        (Own::Outdent, OUTDENT, copy::menus::OUTDENT),
        (Own::QuoteMore, QUOTE_MORE, copy::menus::QUOTE_MORE),
        (Own::QuoteLess, QUOTE_LESS, copy::menus::QUOTE_LESS),
    ]
    .into_iter()
    .map(|(own, key, name)| MenuItem {
        right: hint_own(keymap, own).map_or(Right::None, Right::Shortcut),
        group: Some(copy::headers::LEVELS.to_owned()),
        ..MenuItem::plain(key, name)
    });
    aligns.chain(levels).collect()
}

/// A pick from the size pop-up or the Paragraph pull-down.
fn picked(page: Signal<Page>, key: &str) {
    if let Some((to, ..)) = SIZES.iter().find(|(_, one, _)| *one == key) {
        size(page, *to);
        return;
    }
    match key {
        LEFT => align(page, Align::Left),
        CENTER => align(page, Align::Center),
        RIGHT => align(page, Align::Right),
        INDENT => indentation(page, Nest::In),
        OUTDENT => indentation(page, Nest::Out),
        QUOTE_MORE => quote_level(page, Nest::In),
        QUOTE_LESS => quote_level(page, Nest::Out),
        _ => false,
    };
}

/// The format bar's own controls after the marks: the text size, the colour (which opens the
/// swatch row in the bar's place) and the Paragraph pull-down.
#[component]
pub(super) fn Controls(page: Signal<Page>) -> Element {
    let (now_size, now_align) = {
        let read = page.read();
        let range = target(&read);
        (
            marks_at_start(&read.session.doc, range).size,
            align_at(&read.session.doc, range),
        )
    };
    let keys = use_keys();
    let paragraph = keys.with_keymap(|keymap| paragraph_items(now_align, keymap));
    rsx! {
        PullDownButton::<String> {
            items: menu_items("", &size_items(now_size), false),
            icon: Icon::Type,
            common: Common {
                aria_label: Some(copy::aria::TEXT_SIZE.to_owned()),
                ..Common::default()
            },
            onpick: move |key: String| picked(page, &key),
        }
        Button {
            bezel: Bezel::Toolbar,
            label: copy::aria::TEXT_COLOUR,
            title: copy::aria::TEXT_COLOUR.to_owned(),
            icon: Icon::Circle,
            image: ImagePosition::Only,
            onclick: on_primary(move || page.write().float = Float::Colour),
        }
        PullDownButton::<String> {
            items: menu_items("", &paragraph, false),
            icon: Icon::ListTree,
            common: Common {
                aria_label: Some(copy::aria::PARAGRAPH.to_owned()),
                ..Common::default()
            },
            onpick: move |key: String| picked(page, &key),
        }
    }
}

/// The colours, in the bar's place: quire's swatches, the selection's colour checked. A pick
/// colours the selection and gives the bar back; Escape gives it back as it was.
#[component]
pub(super) fn Swatches(page: Signal<Page>) -> Element {
    let now = {
        let read = page.read();
        marks_at_start(&read.session.doc, target(&read)).color
    };
    let choices: Vec<Choice<Option<TextColor>>> = COLOURS
        .iter()
        .map(|(color, name)| Choice::new(*color, *name).with_swatch(swatch_var(*color)))
        .collect();
    rsx! {
        div {
            class: "c-swatches",
            style: palette_vars(),
            onkeydown: move |event: KeyboardEvent| {
                if event.key() == Key::Escape {
                    event.prevent_default();
                    back_to_bar(page);
                }
            },
            RadioGroup::<Option<TextColor>> {
                label: copy::aria::TEXT_COLOUR.to_owned(),
                choices,
                value: now,
                arrangement: Arrangement::Swatches,
                onchange: move |color: Option<TextColor>| {
                    colour(page, color);
                    back_to_bar(page);
                },
            }
        }
    }
}

/// The swatch row closes: the bar is back, and the body has the keyboard.
fn back_to_bar(mut page: Signal<Page>) {
    if page.peek().float == Float::Colour {
        page.write().float = Float::Closed;
    }
    crate::ui::host::Host::focus_next_frame(".c-body");
}
