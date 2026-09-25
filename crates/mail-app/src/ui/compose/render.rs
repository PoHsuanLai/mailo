//! The document as markup: one element per paragraph, each carrying `data-n` (on Blitz,
//! `data-edit-node`, and each object `data-edit-kind=atom`; see [`node_attrs`]), drawn by Dioxus
//! from the `Doc`. Nothing here reads the DOM, and no markup is built from strings.
//!
//! A paragraph is its own component with value props, so an edit elsewhere does not touch it:
//! Dioxus compares the props and skips it. That is what keeps Rust's hands off a paragraph an
//! IME is composing in, since during a composition the document does not change at all.

use dioxus::prelude::*;

use super::super::menu::Floating;
use super::float::{object_items, pick_object};
use super::page::{Float, Fold, Page};
use crate::editor::{Check, Mark, Node, Object, Op, ParaKind, Pos, Range, Run};
use ds::{Glyph, Icon, MenuKind, MountedRef};

#[cfg(test)]
thread_local! {
    static PARA_RENDERS: std::cell::RefCell<std::collections::BTreeMap<usize, u32>> =
        const { std::cell::RefCell::new(std::collections::BTreeMap::new()) };
}

/// How many times the paragraph at `n` has been drawn on this thread.
#[cfg(test)]
pub(in crate::ui) fn para_renders(n: usize) -> u32 {
    PARA_RENDERS.with(|renders| renders.borrow().get(&n).copied().unwrap_or(0))
}

#[cfg(test)]
pub(in crate::ui) fn reset_para_renders() {
    PARA_RENDERS.with(|renders| renders.borrow_mut().clear());
}

/// What marks a paragraph as node `n`: `data-n`, which the webview's glue reads, or on Blitz
/// `data-edit-node`, which quire's `EditSurface` resolves positions against.
fn node_attrs(n: usize) -> Vec<Attribute> {
    #[cfg(feature = "webview")]
    let name = "data-n";
    #[cfg(not(feature = "webview"))]
    let name = ds::EDIT_NODE_ATTR;
    vec![Attribute::new(name, n.to_string(), None, false)]
}

/// A to-do item's: its node, then its class, in that order.
fn todo_attrs(n: usize, class: &'static str) -> Vec<Attribute> {
    let mut attrs = node_attrs(n);
    attrs.push(Attribute::new("class", class, None, false));
    attrs
}

/// What marks an object as node `n` and keeps the caret out of it: `contenteditable=false` on
/// the webview, an atom (`data-edit-kind=atom`) on Blitz, which the caret goes around.
fn obj_attrs(n: usize) -> Vec<Attribute> {
    #[cfg(feature = "webview")]
    let attrs = vec![
        Attribute::new("contenteditable", "false", None, false),
        Attribute::new("data-n", n.to_string(), None, false),
    ];
    #[cfg(not(feature = "webview"))]
    let attrs = vec![
        Attribute::new(ds::EDIT_NODE_ATTR, n.to_string(), None, false),
        Attribute::new(ds::EDIT_KIND_ATTR, ds::EditKind::Atom.slug(), None, false),
    ];
    attrs
}

/// Which list a run of items is drawn in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ListTag {
    Bullet,
    Numbered,
    Todo,
}

fn list_tag(kind: ParaKind) -> Option<ListTag> {
    match kind {
        ParaKind::Bullet => Some(ListTag::Bullet),
        ParaKind::Numbered => Some(ListTag::Numbered),
        ParaKind::Todo(_) => Some(ListTag::Todo),
        _ => None,
    }
}

/// A top-level piece of the body: one node, or a run of list items that share a list.
enum Group {
    One(usize),
    List(ListTag, Vec<usize>),
}

fn groups(nodes: &[Node]) -> Vec<Group> {
    let mut out: Vec<Group> = Vec::new();
    for (index, node) in nodes.iter().enumerate() {
        let tag = match node {
            Node::Para { kind, .. } => list_tag(*kind),
            Node::Object(_) => None,
        };
        match (tag, out.last_mut()) {
            (Some(tag), Some(Group::List(open, items))) if *open == tag => items.push(index),
            (Some(tag), _) => out.push(Group::List(tag, vec![index])),
            (None, _) => out.push(Group::One(index)),
        }
    }
    out
}

/// The body's children.
pub(in crate::ui) fn body(page: Signal<Page>) -> Element {
    let read = page.read();
    let nodes = read.session.doc.nodes.clone();
    let menu_at = match read.float {
        Float::Object(index) => Some(index),
        _ => None,
    };
    let quoted = read.quoted;
    let keys: Vec<String> = (0..nodes.len()).map(|index| read.node_key(index)).collect();
    drop(read);
    rsx! {
        for group in groups(&nodes) {
            match group {
                Group::One(n) => match nodes[n].clone() {
                    Node::Para { kind, runs } => rsx! {
                        Para { key: "{keys[n]}", n, kind, runs, page }
                    },
                    Node::Object(object) => rsx! {
                        Obj { key: "{keys[n]}", n, object, menu: menu_at == Some(n), quoted, page }
                    },
                },
                Group::List(tag, items) => {
                    let first = items.first().copied().unwrap_or(0);
                    let entries = items.iter().map(|n| (*n, keys[*n].clone(), nodes[*n].clone()));
                    let children = rsx! {
                        for (n, key, node) in entries {
                            if let Node::Para { kind, runs } = node {
                                Para { key: "{key}", n, kind, runs, page }
                            }
                        }
                    };
                    match tag {
                        ListTag::Numbered => rsx! { ol { key: "l{first}", {children} } },
                        ListTag::Bullet => rsx! { ul { key: "l{first}", {children} } },
                        ListTag::Todo => rsx! { ul { key: "l{first}", class: "todo", {children} } },
                    }
                }
            }
        }
    }
}

/// The classes a run's marks draw with.
fn mark_class(run: &Run) -> String {
    [
        (Mark::Bold, "m-b"),
        (Mark::Italic, "m-i"),
        (Mark::Underline, "m-u"),
        (Mark::Strike, "m-s"),
        (Mark::Code, "m-code"),
    ]
    .into_iter()
    .filter(|(mark, _)| run.marks.has(*mark))
    .map(|(_, class)| class)
    .collect::<Vec<_>>()
    .join(" ")
}

/// A paragraph's text. An empty one holds a `<br>` so the caret has a line to stand on, and a
/// trailing line break gets one so the empty last line shows.
fn text(runs: &[Run]) -> Element {
    let ends_open = runs.last().is_none_or(|run| run.text.ends_with('\n'));
    rsx! {
        for (index, run) in runs.iter().enumerate() {
            if let Some(url) = &run.marks.link {
                a { key: "{index}", class: "m-a {mark_class(run)}", href: "{url.as_str()}", "{run.text}" }
            } else {
                span { key: "{index}", class: "{mark_class(run)}", "{run.text}" }
            }
        }
        if ends_open {
            br {}
        }
    }
}

/// One paragraph, drawn as the element its kind is.
#[component]
fn Para(n: usize, kind: ParaKind, runs: Vec<Run>, page: Signal<Page>) -> Element {
    #[cfg(test)]
    PARA_RENDERS.with(|renders| *renders.borrow_mut().entry(n).or_insert(0) += 1);
    let inner = text(&runs);
    match kind {
        ParaKind::Paragraph => rsx! { p { ..node_attrs(n), {inner} } },
        ParaKind::Heading(level) => match level.number() {
            1 => rsx! { h2 { ..node_attrs(n), {inner} } },
            2 => rsx! { h3 { ..node_attrs(n), {inner} } },
            _ => rsx! { h4 { ..node_attrs(n), {inner} } },
        },
        ParaKind::Bullet | ParaKind::Numbered => rsx! { li { ..node_attrs(n), {inner} } },
        ParaKind::Todo(check) => {
            let (class, next) = match check {
                Check::Open => ("", Check::Done),
                Check::Done => ("done", Check::Open),
            };
            rsx! {
                li { ..todo_attrs(n, class),
                    span {
                        class: "box",
                        contenteditable: "false",
                        onmousedown: move |event| event.prevent_default(),
                        onclick: move |_| {
                            let caret = page.read().session.caret;
                            let at = Pos::new(n, 0);
                            let op = Op::SetKind { range: Range { start: at, end: at }, kind: ParaKind::Todo(next) };
                            super::float::commit(&mut page.write(), vec![op], caret);
                        },
                    }
                    {inner}
                }
            }
        }
        ParaKind::Quote => rsx! { blockquote { ..node_attrs(n), {inner} } },
        ParaKind::Code => rsx! { pre { ..node_attrs(n), {inner} } },
    }
}

/// The ⋮⋮ handle every object carries, and the object menu, floating beside it, when it is open.
fn grip(
    n: usize,
    menu: bool,
    mut page: Signal<Page>,
    mut handle: Signal<Option<MountedRef>>,
) -> Element {
    rsx! {
        span {
            class: "ograb",
            title: "Options",
            onmounted: move |event: MountedEvent| handle.set(Some(MountedRef(event.data()))),
            onmousedown: move |event| event.prevent_default(),
            onclick: move |_| {
                let next = if menu { Float::Closed } else { Float::Object(n) };
                page.write().float = next;
            },
            "⋮⋮"
        }
        if menu {
            Floating {
                kind: MenuKind::Slim,
                anchor: handle(),
                title: "This object".to_owned(),
                items: object_items(),
                on_pick: move |key: String| pick_object(&mut page.write(), n, &key),
                on_close: move |_| {
                    if page.peek().float == Float::Object(n) {
                        page.write().float = Float::Closed;
                    }
                },
            }
        }
    }
}

/// An object: atomic, not editable, and the only thing with a handle.
#[component]
fn Obj(n: usize, object: Object, menu: bool, quoted: Fold, page: Signal<Page>) -> Element {
    let handle = use_signal(|| None::<MountedRef>);
    match object {
        Object::Divider => rsx! {
            div { class: "obj o-hr", ..obj_attrs(n),
                {grip(n, menu, page, handle)}
                hr {}
            }
        },
        Object::Signature => rsx! {
            div { class: "obj o-sig", ..obj_attrs(n),
                span { class: "mono", "-- " }
            }
        },
        Object::Image { src, alt } => rsx! {
            figure { class: "obj o-img", ..obj_attrs(n),
                {grip(n, menu, page, handle)}
                if src.as_str().is_empty() {
                    div { class: "pick",
                        Glyph { icon: Icon::Paperclip, size: ds::IconSize::Large }
                        span { "Add an image. It is embedded in the message, never fetched." }
                    }
                } else {
                    div { class: "imgbox", img { src: "{src.as_str()}", alt: "{alt}" } }
                    if !alt.is_empty() {
                        figcaption { "{alt}" }
                    }
                }
            }
        },
        Object::Table(table) => rsx! {
            div { class: "obj o-table", ..obj_attrs(n),
                {grip(n, menu, page, handle)}
                div { class: "tbl",
                    table {
                        tbody {
                            for (r, row) in table.rows.iter().enumerate() {
                                tr { key: "{r}",
                                    for (c, cell) in row.iter().enumerate() {
                                        td { key: "{c}", "{cell}" }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        },
        Object::Attachment(file) => rsx! {
            div { class: "obj o-att", ..obj_attrs(n),
                {grip(n, menu, page, handle)}
                Glyph { icon: Icon::Paperclip }
                span { "{file.name()}" }
            }
        },
        Object::QuotedMessage { who, when, body } => {
            let open = quoted == Fold::Open;
            let lines: Vec<String> = body
                .iter()
                .map(words)
                .filter(|line| !line.is_empty())
                .collect();
            rsx! {
                div { class: "obj o-rq", ..obj_attrs(n),
                    {grip(n, menu, page, handle)}
                    ds::Button {
                        variant: ds::ButtonVariant::Quiet,
                        icon: Icon::Corner,
                        // Who in the strong tone, when and the hint quieter.
                        label: ds::Text::Runs(vec![
                            ds::Run::new(who.clone(), ds::RunTone::Strong),
                            ds::Run::new(format!(", {when}"), ds::RunTone::Faint),
                            ds::Run::new(
                                if open { "  hide quoted text" } else { "  show quoted text" },
                                ds::RunTone::Faint,
                            ),
                        ]),
                        onclick: move |_| {
                            let mut write = page.write();
                            write.quoted = if open { Fold::Folded } else { Fold::Open };
                        },
                    }
                    if open {
                        div { class: "rq-body",
                            for (index, line) in lines.iter().enumerate() {
                                p { key: "{index}", "{line}" }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// A quoted block's words, for the unfolded original.
fn words(block: &mail_mime::Block) -> String {
    use mail_mime::{Block, Span};
    fn spans(parts: &[Span], out: &mut String) {
        for part in parts {
            match part {
                Span::Text(text) | Span::Code(text) => out.push_str(text),
                Span::Break => out.push('\n'),
                Span::Strong(inner) | Span::Emphasis(inner) | Span::Link { spans: inner, .. } => {
                    spans(inner, out);
                }
            }
        }
    }
    let mut out = String::new();
    match block {
        Block::Paragraph { spans: parts, .. } | Block::Heading { spans: parts, .. } => {
            spans(parts, &mut out);
        }
        Block::Code { text, .. } => out.push_str(text),
        Block::Quote { blocks, .. } | Block::Signature(blocks) => {
            let inner: Vec<String> = blocks.iter().map(words).collect();
            out.push_str(&inner.join("\n"));
        }
        _ => {}
    }
    out
}
