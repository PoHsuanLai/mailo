//! The message body: quire's `EditSurface` (`surface.rs`), the `/`, `@` and `:` menus at the
//! caret, and the selection bubble.
//!
//! Every handler here calls `editor::` through [`super::wire`] and [`super::float`]. The only
//! thing this file decides is which key goes where.

use dioxus::prelude::*;
use ds::components::controls::button_model::{Bezel, ImagePosition};
use ds::components::controls::segmented::Tracking;
use ds::components::menus::pop_up_button::PopUpButton;
use ds::host::measure::{Anchor, MountedRef};
use ds::prelude::*;

use super::super::common::classed;
use super::super::menu::{MenuKey, anchor_at, menu_items, menu_key};
use super::super::press::on_primary;
use super::float::{
    Picked, commit, current_kind, emoji_items, mention_items, pick_mention, pick_slash, pick_turn,
    turn_items,
};
use super::page::{Float, Page};
use super::templates::{self, TemplateFloat, page_slash_items};
use crate::ui::editor::{InputEvent, Mark, Node, Op, Presence, Range};
use crate::ui::view::Shell;

/// Milliseconds on the wall clock, which is what groups typing into undo steps.
pub(in crate::ui) fn now_ms() -> u64 {
    u64::try_from(chrono::Utc::now().timestamp_millis()).unwrap_or(0)
}

/// Today, written out, for `/date`.
fn today_words() -> String {
    chrono::Local::now().format("%A %-d %B %Y").to_string()
}

#[component]
pub(in crate::ui) fn Body(
    page: Signal<Page>,
    shell: Signal<Shell>,
    on_attach: EventHandler<()>,
) -> Element {
    // The float's box: quire's menus float against it when the caret has no rect yet.
    let mut at_caret = use_signal(|| None::<MountedRef>);
    // The surface measures the caret and the selection itself.
    let marks = use_signal(super::surface::Marks::default);
    let read = page.read();
    let only_empty = matches!(read.session.doc.nodes.as_slice(),
        [Node::Para { runs, .. }] if runs.is_empty());
    let float = read.float.clone();
    let selected = read.selection.is_some();
    drop(read);
    let class = if only_empty { "c-body ph" } else { "c-body" };
    let editable = rsx! { super::surface::Surface { page, shell, on_attach, class, marks } };
    // Where the `/` and `@` menus float: the caret's own rect, and the float boxes and the bubble
    // are placed from the surface's measures.
    let (at, below, above) = (
        move || {
            marks
                .read()
                .at
                .map_or_else(|| anchor_at(at_caret()), Anchor::Rect)
        },
        marks.read().below_caret(),
        marks.read().above_selection(),
    );
    rsx! {
        div { class: "c-edit",
            {editable}
            match float {
                // quire's menus, their cursor the editor's: the caret keeps the keyboard, and
                // the page's keys move `active` and pick with it. A pick is followed by quire's
                // close, which must not undo a float the pick opened (Save as template…).
                Float::Slash { active, .. } => rsx! {
                    div {
                        class: "c-float",
                        "data-anchor": "below",
                        style: below.clone(),
                        onmounted: move |event| at_caret.set(Some(MountedRef(event.data()))),
                    }
                    Menu::<String> {
                        placement: MenuPlacement::Popup,
                        anchor: at(),
                        items: menu_items("", &page_slash_items(&page.read()), false),
                        onpick: move |key: String| pick(page, on_attach, &key),
                        onclose: move |()| close_float(page),
                        active: MenuCursor::Controlled(Some(active)),
                        on_active: move |to: Option<usize>| {
                            if let Some(to) = to {
                                set_active(page, to);
                            }
                        },
                    }
                },
                Float::Mention { active, .. } => rsx! {
                    div {
                        class: "c-float",
                        "data-anchor": "below",
                        style: below.clone(),
                        onmounted: move |event| at_caret.set(Some(MountedRef(event.data()))),
                    }
                    Menu::<String> {
                        placement: MenuPlacement::Popup,
                        anchor: at(),
                        items: menu_items("Mention, and add to Cc", &mention_items(&page.read()), false),
                        onpick: move |key: String| pick_mention(&mut page.write(), &key),
                        onclose: move |()| close_float(page),
                        active: MenuCursor::Controlled(Some(active)),
                        on_active: move |to: Option<usize>| {
                            if let Some(to) = to {
                                set_active(page, to);
                            }
                        },
                    }
                },
                Float::Emoji { active, .. } => rsx! {
                    div {
                        class: "c-float",
                        "data-anchor": "below",
                        style: below.clone(),
                        onmounted: move |event| at_caret.set(Some(MountedRef(event.data()))),
                    }
                    Menu::<String> {
                        placement: MenuPlacement::Popup,
                        anchor: at(),
                        items: menu_items("Emoji", &emoji_items(&page.read()), false),
                        onpick: move |key: String| super::emoji::pick_typed(page, &key),
                        onclose: move |()| close_float(page),
                        active: MenuCursor::Controlled(Some(active)),
                        on_active: move |to: Option<usize>| {
                            if let Some(to) = to {
                                set_active(page, to);
                            }
                        },
                    }
                },
                Float::SaveTemplate(_) | Float::Templates { .. } => rsx! {
                    div {
                        class: "c-float",
                        "data-anchor": "below",
                        style: below.clone(),
                        onmounted: move |event| at_caret.set(Some(MountedRef(event.data()))),
                        TemplateFloat { page, shell, at: at_caret() }
                    }
                },
                _ if selected => rsx! { Bubble { page, place: above } },
                _ => rsx! {},
            }
        }
    }
}

fn pick(mut page: Signal<Page>, on_attach: EventHandler<()>, key: &str) {
    if templates::pick(&mut page.write(), key) {
        // The name field takes the keys once it is there.
        if matches!(page.peek().float, Float::SaveTemplate(_)) {
            crate::ui::host::Host::focus_after_task(".tpl-name input");
        }
        return;
    }
    let picked = pick_slash(&mut page.write(), key, &today_words());
    if picked == Picked::Attach {
        on_attach.call(());
    }
}

/// Whether `key` with `modifiers` is the menus' or a chord's, and if it is, do it: an open
/// menu's arrows, Enter and Escape, then Ctrl B, I, U, Shift S, E, K, Z, Shift Z and Y. The
/// surface (`surface.rs`) asks this first; a taken key goes no further.
pub(super) fn key_taken(
    mut page: Signal<Page>,
    shell: Signal<Shell>,
    on_attach: EventHandler<()>,
    key: &str,
    modifiers: Modifiers,
) -> bool {
    let ctrl = ds::prelude::is_command(modifiers);
    let float = page.read().float.clone();
    if templates::key(page, shell, key) {
        return true;
    }
    if let Float::Slash { active, .. }
    | Float::Mention { active, .. }
    | Float::Emoji { active, .. } = float
    {
        let items = match float {
            Float::Slash { .. } => page_slash_items(&page.read()),
            Float::Emoji { .. } => emoji_items(&page.read()),
            _ => mention_items(&page.read()),
        };
        let taken = match menu_key(key) {
            Some(MenuKey::Down) => {
                set_active(page, (active + 1) % items.len().max(1));
                true
            }
            Some(MenuKey::Up) => {
                set_active(page, (active + items.len().max(1) - 1) % items.len().max(1));
                true
            }
            Some(MenuKey::Enter) => {
                if let Some(item) = items.get(active) {
                    match float {
                        Float::Slash { .. } => pick(page, on_attach, &item.key),
                        Float::Emoji { .. } => super::emoji::pick_typed(page, &item.key),
                        _ => pick_mention(&mut page.write(), &item.key),
                    }
                }
                true
            }
            Some(MenuKey::Escape) => {
                page.write().float = Float::Closed;
                true
            }
            _ => false,
        };
        if taken {
            return true;
        }
    }
    if !ctrl {
        return false;
    }
    let lower = key.to_lowercase();
    match (lower.as_str(), modifiers.shift()) {
        ("b", false) => format(page, "formatBold"),
        ("i", false) => format(page, "formatItalic"),
        ("u", false) => format(page, "formatUnderline"),
        ("s", true) => format(page, "formatStrikeThrough"),
        ("e", false) => code(page),
        ("k", false) => {
            let has = page.read().selection.is_some();
            if has {
                page.write().float = Float::Link(String::new());
            }
            has
        }
        ("z", false) => format(page, "historyUndo"),
        ("z", true) | ("y", false) => format(page, "historyRedo"),
        _ => false,
    }
}

/// quire's menu closed (Esc, a press outside, or after a pick): the `/`, `@` or `:` menu goes,
/// and whatever a pick opened in its place stays.
fn close_float(mut page: Signal<Page>) {
    if matches!(
        page.peek().float,
        Float::Slash { .. } | Float::Mention { .. } | Float::Emoji { .. }
    ) {
        page.write().float = Float::Closed;
    }
}

fn set_active(mut page: Signal<Page>, to: usize) {
    if let Float::Slash { active, .. }
    | Float::Mention { active, .. }
    | Float::Emoji { active, .. } = &mut page.write().float
    {
        *active = to;
    }
}

/// Bold, italic, underline, strike, undo and redo go through the editor as the input events a
/// browser would have sent, so they are recorded and undone exactly like typed ones.
pub(in crate::ui) fn format(mut page: Signal<Page>, input_type: &str) -> bool {
    let mut write = page.write();
    let ranges: Vec<Range> = write.selection.into_iter().collect();
    let event = InputEvent::new(input_type, None, ranges, false);
    let before = write.session.doc.clone();
    if write.session.handle(&event, now_ms()).is_ok() && write.session.doc != before {
        write.touch();
    }
    true
}

/// Inline code over the selection, or off when it is all code already.
pub(in crate::ui) fn code(mut page: Signal<Page>) -> bool {
    let Some(range) = page.read().selection else {
        return false;
    };
    let on = if covered(&page.read(), range, Mark::Code) {
        Presence::Off
    } else {
        Presence::On
    };
    let caret = page.read().session.caret;
    let mut write = page.write();
    commit(
        &mut write,
        vec![Op::SetMark {
            range,
            mark: Mark::Code,
            on,
        }],
        caret,
    );
    write.selection = Some(range);
    true
}

/// Whether every paragraph `range` touches carries `mark` wherever it touches. Read-only.
pub(in crate::ui) fn covered(page: &Page, range: Range, mark: Mark) -> bool {
    let range = range.ordered();
    page.session
        .doc
        .nodes
        .iter()
        .enumerate()
        .take(range.end.node + 1)
        .skip(range.start.node)
        .all(|(index, node)| {
            let Node::Para { runs, .. } = node else {
                return true;
            };
            let from = if index == range.start.node {
                range.start.offset
            } else {
                0
            };
            let to = if index == range.end.node {
                range.end.offset
            } else {
                usize::MAX
            };
            let mut seen = 0usize;
            runs.iter().all(|run| {
                let len = crate::ui::editor::grapheme_len(&run.text);
                let overlaps = seen < to && seen + len > from;
                seen += len;
                !overlaps || run.marks.has(mark)
            })
        })
}

/// The format bar over a selection: Turn into (a pop-up button), the four marks (a quire
/// segmented control that holds any number down), inline code and a link.
#[component]
fn Bubble(page: Signal<Page>, place: Option<String>) -> Element {
    let read = page.read();
    let Some(range) = read.selection else {
        return rsx! {};
    };
    let float = read.float.clone();
    let kind = current_kind(&read);
    let held: Vec<Mark> = [
        Mark::Bold,
        Mark::Italic,
        Mark::Underline,
        Mark::Strike,
        Mark::Code,
    ]
    .into_iter()
    .filter(|mark| covered(&read, range, *mark))
    .collect();
    drop(read);
    let turn = turn_items(kind);
    let now = turn
        .iter()
        .find(|item| matches!(item.right, super::super::menu::Right::Check(true)))
        .map(|item| item.key.clone());
    let marks = vec![
        // Named, since a segment that is a picture alone says nothing else. gap(quire): a named
        // image-only segment takes no tip yet.
        Choice::new(Mark::Bold, "")
            .with_icon(Icon::Bold)
            .with_name("Bold"),
        Choice::new(Mark::Italic, "")
            .with_icon(Icon::Italic)
            .with_name("Italic"),
        Choice::new(Mark::Underline, "")
            .with_icon(Icon::Underline)
            .with_name("Underline"),
        Choice::new(Mark::Strike, "")
            .with_icon(Icon::Strike)
            .with_name("Strikethrough"),
        Choice::new(Mark::Code, "")
            .with_icon(Icon::Code)
            .with_name("Code"),
    ];
    rsx! {
        div {
            class: "bubble",
            "data-anchor": "above",
            style: place,
            onmousedown: move |event| event.prevent_default(),
            match float {
                Float::Link(typed) => rsx! {
                    div {
                        class: "link-in",
                        onkeydown: move |event: KeyboardEvent| {
                            let key = event.key().to_string();
                            if key == "Enter" {
                                event.prevent_default();
                                link(page);
                            } else if key == "Escape" {
                                page.write().float = Float::Closed;
                            }
                        },
                        TextField {
                            label: "Paste a link, then Enter".to_owned(),
                            bezel: FieldBezel::Plain,
                            placeholder: "Paste a link, then Enter".to_owned(),
                            value: typed,
                            common: classed("link-field"),
                            oninput: move |value: String| page.write().float = Float::Link(value),
                        }
                    }
                },
                // The bar acts on a press and leaves the keyboard in the body, as a Mac format
                // bar does: Bold, then Ctrl-I, italicises the same words.
                _ => rsx! { FocusOnPressScope { focus: FocusOnPress::Refuses,
                    PopUpButton::<String> {
                        items: menu_items("", &turn, false),
                        value: now,
                        title: "Text".to_owned(),
                        onpick: move |key: String| pick_turn(&mut page.write(), &key),
                    }
                    SegmentedControl::<Mark> {
                        label: "Text style".to_owned(),
                        choices: marks,
                        tracking: Tracking::SelectAny(held),
                        onchange: move |mark: Mark| match mark {
                            Mark::Code => {
                                code(page);
                            }
                            Mark::Bold | Mark::Italic | Mark::Underline | Mark::Strike => {
                                format(page, mark_command(mark));
                            }
                        },
                    }
                    Button {
                        bezel: Bezel::Toolbar,
                        label: "Link",
                        title: "Link (\u{2318}K)".to_owned(),
                        icon: Icon::Link,
                        image: ImagePosition::Only,
                        onclick: on_primary(move || page.write().float = Float::Link(String::new())),
                    }
                } },
            }
        }
    }
}

/// The editor command a mark's segment runs.
fn mark_command(mark: Mark) -> &'static str {
    match mark {
        Mark::Bold => "formatBold",
        Mark::Italic => "formatItalic",
        Mark::Underline => "formatUnderline",
        Mark::Strike => "formatStrikeThrough",
        Mark::Code => "formatCode",
    }
}

/// The typed link over the selection. Only `http`, `https` and `mailto` are links.
fn link(mut page: Signal<Page>) {
    let (range, typed) = {
        let read = page.read();
        match (&read.selection, &read.float) {
            (Some(range), Float::Link(typed)) => (*range, typed.trim().to_owned()),
            _ => return,
        }
    };
    let Some(url) = mail_mime::SafeUrl::parse(&typed) else {
        page.write().notice = Some(format!("“{typed}” is not a web or mail link"));
        return;
    };
    let caret = page.read().session.caret;
    let mut write = page.write();
    commit(
        &mut write,
        vec![Op::SetLink {
            range,
            url: Some(url),
        }],
        caret,
    );
    write.float = Float::Closed;
}
