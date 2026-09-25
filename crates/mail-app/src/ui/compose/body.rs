//! The message body: one `contenteditable` root, the hidden wire beside it, the `/` and `@`
//! menus at the caret, and the selection bubble.
//!
//! Every handler here calls `editor::` through [`super::wire`] and [`super::float`]. The only
//! thing this file decides is which key goes where.

use dioxus::prelude::*;

use super::super::field::{Field, FieldKind};
use super::super::menu::{Menu, MenuKey, anchor_at, menu_key, quire_entries};
use super::float::{
    Picked, commit, current_kind, mention_items, pick_mention, pick_slash, pick_turn,
    suggest_mention, turn_items,
};
use super::page::{Float, Page};
use super::render;
use super::templates::{self, TemplateFloat, page_slash_items};
use super::wire::{self, caret_attr};
use crate::editor::{InputEvent, Mark, Node, Op, Presence, Range, to_html};
use crate::view::Shell;

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
    // The caret's box, which the glue moves to the caret: quire's menus float against it.
    let mut at_caret = use_signal(|| None::<ds::MountedRef>);
    let read = page.read();
    let only_empty = matches!(read.session.doc.nodes.as_slice(),
        [Node::Para { runs, .. }] if runs.is_empty());
    let seq = read.wire.seq;
    let caret = caret_attr(&read);
    // The document as the writers see it, for the live probe. Debug builds only.
    let doc = if cfg!(debug_assertions) {
        to_html(&read.session.doc)
    } else {
        String::new()
    };
    let float = read.float.clone();
    let selected = read.selection.is_some();
    drop(read);
    let class = if only_empty { "c-body ph" } else { "c-body" };
    rsx! {
        div { class: "c-edit",
            div {
                class,
                contenteditable: "true",
                spellcheck: "true",
                role: "textbox",
                aria_multiline: "true",
                aria_label: "Message",
                "data-seq": "{seq}",
                "data-caret": "{caret}",
                "data-doc": "{doc}",
                onkeydown: move |event| keys(page, shell, on_attach, event),
                {render::body(page)}
            }
            textarea {
                class: "c-wire",
                tabindex: "-1",
                aria_hidden: "true",
                oninput: move |event| {
                    if let Some(heard) = wire::parse(&event.value()) {
                        let store = try_consume_context::<std::sync::Arc<mail_store::SqliteStore>>();
                        let mut write = page.write();
                        wire::hear(&mut write, heard, now_ms());
                        if let Some(store) = store {
                            suggest_mention(&mut write, store.as_ref());
                        }
                    }
                },
            }
            match float {
                // quire's menus, their cursor the editor's: the caret keeps the keyboard, and
                // the page's keys move `active` and pick with it. A pick is followed by quire's
                // close, which must not undo a float the pick opened (Save as template…).
                Float::Slash { active, .. } => rsx! {
                    div {
                        class: "c-float",
                        "data-anchor": "below",
                        onmounted: move |event| at_caret.set(Some(ds::MountedRef(event.data()))),
                    }
                    ds::Menu::<String> {
                        kind: ds::MenuKind::Rich,
                        anchor: anchor_at(at_caret()),
                        entries: quire_entries("", &page_slash_items(&page.read()), ds::AvatarSize::Size34, None),
                        onpick: move |key: String| pick(page, on_attach, &key),
                        onclose: move |()| close_float(page),
                        active: ds::Cursor::Controlled(Some(active)),
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
                        onmounted: move |event| at_caret.set(Some(ds::MountedRef(event.data()))),
                    }
                    ds::Menu::<String> {
                        kind: ds::MenuKind::Slim,
                        anchor: anchor_at(at_caret()),
                        entries: quire_entries(
                            "Mention, and add to Cc",
                            &mention_items(&page.read()),
                            ds::AvatarSize::Size20,
                            None,
                        ),
                        onpick: move |key: String| pick_mention(&mut page.write(), &key),
                        onclose: move |()| close_float(page),
                        active: ds::Cursor::Controlled(Some(active)),
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
                        onmounted: move |event| at_caret.set(Some(ds::MountedRef(event.data()))),
                        TemplateFloat { page, shell, at: at_caret() }
                    }
                },
                _ if selected => rsx! { Bubble { page } },
                _ => rsx! {},
            }
        }
    }
}

fn pick(mut page: Signal<Page>, on_attach: EventHandler<()>, key: &str) {
    if templates::pick(&mut page.write(), key) {
        // The name field takes the keys once it is there.
        if matches!(page.peek().float, Float::SaveTemplate(_)) {
            crate::ui::host::Host::focus_after_task(".tpl-name");
        }
        return;
    }
    let picked = pick_slash(&mut page.write(), key, &today_words());
    if picked == Picked::Attach {
        on_attach.call(());
    }
}

/// Keys the browser would otherwise take: the menus' arrows and Enter, and the inline marks.
fn keys(
    mut page: Signal<Page>,
    shell: Signal<Shell>,
    on_attach: EventHandler<()>,
    event: KeyboardEvent,
) {
    let key = event.key().to_string();
    let modifiers = event.modifiers();
    let ctrl = modifiers.ctrl() || modifiers.meta();
    let float = page.read().float.clone();
    if templates::key(page, shell, &key) {
        event.prevent_default();
        event.stop_propagation();
        return;
    }
    if let Float::Slash { active, .. } | Float::Mention { active, .. } = float {
        let items = match float {
            Float::Slash { .. } => page_slash_items(&page.read()),
            _ => mention_items(&page.read()),
        };
        let taken = match menu_key(&key) {
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
            event.prevent_default();
            event.stop_propagation();
            return;
        }
    }
    if !ctrl {
        return;
    }
    let lower = key.to_lowercase();
    let done = match (lower.as_str(), modifiers.shift()) {
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
    };
    if done {
        event.prevent_default();
        event.stop_propagation();
    }
}

/// quire's menu closed (Esc, a press outside, or after a pick): the `/` or `@` menu goes, and
/// whatever a pick opened in its place stays.
fn close_float(mut page: Signal<Page>) {
    if matches!(
        page.peek().float,
        Float::Slash { .. } | Float::Mention { .. }
    ) {
        page.write().float = Float::Closed;
    }
}

fn set_active(mut page: Signal<Page>, to: usize) {
    if let Float::Slash { active, .. } | Float::Mention { active, .. } = &mut page.write().float {
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
                let len = crate::editor::grapheme_len(&run.text);
                let overlaps = seen < to && seen + len > from;
                seen += len;
                !overlaps || run.marks.has(mark)
            })
        })
}

/// The selection bubble: Turn into, the five marks, and a link.
#[component]
fn Bubble(page: Signal<Page>) -> Element {
    let read = page.read();
    let Some(range) = read.selection else {
        return rsx! {};
    };
    let float = read.float.clone();
    let kind = current_kind(&read);
    let turn_label = turn_items(kind)
        .into_iter()
        .find(|item| matches!(item.right, super::super::menu::Right::Check(true)))
        .map(|item| item.name)
        .unwrap_or_else(|| "Text".to_owned());
    let pressed = |mark| {
        if covered(&read, range, mark) {
            "true"
        } else {
            "false"
        }
    };
    let (bold, italic, under, strike) = (
        pressed(Mark::Bold),
        pressed(Mark::Italic),
        pressed(Mark::Underline),
        pressed(Mark::Strike),
    );
    drop(read);
    rsx! {
        div {
            class: "bubble",
            "data-anchor": "above",
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
                        Field {
                            kind: FieldKind::Inline,
                            value: typed,
                            placeholder: "Paste a link, then Enter".to_owned(),
                            extra: None,
                            on_input: move |value: String| page.write().float = Float::Link(value),
                            on_focus: |_| {},
                            on_blur: |_| {},
                        }
                    }
                },
                _ => rsx! {
                    button { class: "turn", r#type: "button",
                        onclick: move |_| {
                            let open = page.read().float == Float::Turn;
                            page.write().float = if open { Float::Closed } else { Float::Turn };
                        },
                        "{turn_label} ▾"
                    }
                    span { class: "sep" }
                    button { r#type: "button", title: "Bold (Ctrl B)", aria_pressed: bold,
                        onclick: move |_| { format(page, "formatBold"); }, b { "B" } }
                    button { r#type: "button", title: "Italic (Ctrl I)", aria_pressed: italic,
                        onclick: move |_| { format(page, "formatItalic"); }, i { class: "serif", "i" } }
                    button { r#type: "button", title: "Underline (Ctrl U)", aria_pressed: under,
                        onclick: move |_| { format(page, "formatUnderline"); }, u { "U" } }
                    button { r#type: "button", title: "Strikethrough (Ctrl Shift S)", aria_pressed: strike,
                        onclick: move |_| { format(page, "formatStrikeThrough"); }, s { "S" } }
                    button { class: "code", r#type: "button", title: "Inline code (Ctrl E)",
                        onclick: move |_| { code(page); }, "</>" }
                    span { class: "sep" }
                    button { r#type: "button", title: "Link (Ctrl K)",
                        onclick: move |_| page.write().float = Float::Link(String::new()), "Link" }
                    if float == Float::Turn {
                        div { class: "turn-menu",
                            Menu {
                                title: "Turn into".to_owned(),
                                items: turn_items(kind),
                                filterable: false,
                                on_pick: move |key: String| pick_turn(&mut page.write(), &key),
                                on_close: move |_| page.write().float = Float::Closed,
                                on_query: move |_| {},
                                slim: true,
                                active: None,
                            }
                        }
                    }
                },
            }
        }
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
