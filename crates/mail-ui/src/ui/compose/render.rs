//! The document as markup: one element per paragraph, each carrying `data-edit-node`, and each
//! object `data-edit-kind=atom` (see [`node_attrs`]), drawn by Dioxus
//! from the `Doc`. Nothing here reads the DOM, and no markup is built from strings.
//!
//! A paragraph is its own component with value props, so an edit elsewhere does not touch it:
//! Dioxus compares the props and skips it. That is what keeps Rust's hands off a paragraph an
//! IME is composing in, since during a composition the document does not change at all.

use dioxus::prelude::*;
use ds::components::content::text_runs::RunTone;
use ds::components::controls::button_model::Bezel;
use ds::host::measure::MountedRef;
use ds::host::position::{EDIT_KIND_ATTR, EDIT_NODE_ATTR, EditKind};
use ds::prelude::*;
use ds::style::icon::render::Glyph;

use super::super::menu::Floating;
use super::float::{object_items, pick_object};
use super::page::{Float, Fold, Page};
use mail_editor::{
    Align, Check, Layout, Mark, Node, Object, Op, ParaKind, Pos, Range, Run, TextSize,
};

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

/// What marks a paragraph as node `n`: `data-edit-node`, which quire's `EditSurface` resolves
/// positions against.
fn node_attrs(n: usize) -> Vec<Attribute> {
    vec![Attribute::new(EDIT_NODE_ATTR, n.to_string(), None, false)]
}

/// A to-do item's: its node, then its class, in that order.
fn todo_attrs(n: usize, class: &'static str) -> Vec<Attribute> {
    let mut attrs = node_attrs(n);
    attrs.push(Attribute::new("class", class, None, false));
    attrs
}

/// What marks an object as node `n` and keeps the caret out of it: an atom
/// (`data-edit-kind=atom`), which the caret goes around.
fn obj_attrs(n: usize) -> Vec<Attribute> {
    vec![
        Attribute::new(EDIT_NODE_ATTR, n.to_string(), None, false),
        Attribute::new(EDIT_KIND_ATTR, EditKind::Atom.slug(), None, false),
    ]
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

/// A top-level piece of the body: one node, or a run of list items that share a list: one tag
/// at one depth.
enum Group {
    One(usize),
    List(ListTag, u8, Vec<usize>),
}

fn groups(nodes: &[Node]) -> Vec<Group> {
    let mut out: Vec<Group> = Vec::new();
    for (index, node) in nodes.iter().enumerate() {
        let tag = match node {
            Node::Para { kind, layout, .. } => list_tag(*kind).map(|tag| (tag, layout.depth)),
            Node::Object(_) => None,
        };
        match (tag, out.last_mut()) {
            (Some(tag), Some(Group::List(open, depth, items))) if (*open, *depth) == tag => {
                items.push(index);
            }
            (Some((tag, depth)), _) => out.push(Group::List(tag, depth, vec![index])),
            (None, _) => out.push(Group::One(index)),
        }
    }
    out
}

/// The number each numbered item shows, by node: counted per depth through a run of items, so
/// a list that goes on after a nested one goes on counting. Blitz draws no `<ol start>`, and a
/// nested list is drawn as a list of its own, so an item past the first of a list that was
/// interrupted carries its number as its marker.
fn numbers(nodes: &[Node]) -> Vec<Option<usize>> {
    let mut out = vec![None; nodes.len()];
    let mut counts: Vec<(ListTag, usize)> = Vec::new();
    for (index, node) in nodes.iter().enumerate() {
        let Some((tag, depth)) = (match node {
            Node::Para { kind, layout, .. } => list_tag(*kind).map(|tag| (tag, layout.depth)),
            Node::Object(_) => None,
        }) else {
            counts.clear();
            continue;
        };
        let depth = usize::from(depth);
        counts.truncate(depth + 1);
        while counts.len() <= depth {
            counts.push((tag, 0));
        }
        if counts[depth].0 != tag {
            counts[depth] = (tag, 0);
        }
        counts[depth].1 += 1;
        if tag == ListTag::Numbered {
            out[index] = Some(counts[depth].1);
        }
    }
    out
}

/// A nested list's own style: how far in its depth puts it, and the bullet it draws. `None` at
/// the top.
fn list_style(tag: ListTag, depth: u8) -> Option<String> {
    if depth == 0 {
        return None;
    }
    let mut style = format!("margin-left:{}em", f32::from(depth) * 1.5);
    if tag == ListTag::Bullet {
        let bullet = if depth == 1 { "circle" } else { "square" };
        style.push_str(&format!(";list-style-type:{bullet}"));
    }
    Some(style)
}

/// `parts` as a `style` attribute: `None`, so none is written, for no parts.
fn style_of(parts: Vec<String>) -> Option<String> {
    (!parts.is_empty()).then(|| parts.join(";"))
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
    let numbers = numbers(&nodes);
    rsx! {
        for group in groups(&nodes) {
            match group {
                Group::One(n) => match nodes[n].clone() {
                    Node::Para { kind, runs, layout } => rsx! {
                        Para { key: "{keys[n]}", n, kind, runs, layout, marker: None, page }
                    },
                    Node::Object(object) => rsx! {
                        Obj { key: "{keys[n]}", n, object, menu: menu_at == Some(n), quoted, page }
                    },
                },
                Group::List(tag, depth, items) => {
                    let first = items.first().copied().unwrap_or(0);
                    // A list that starts at one counts itself; one that goes on after a nested
                    // list says each number.
                    let counted = numbers[first].is_none_or(|number| number == 1);
                    let entries = items.iter().map(|n| {
                        let marker = numbers[*n].filter(|_| !counted).map(|number| format!("{number}. "));
                        (*n, keys[*n].clone(), nodes[*n].clone(), marker)
                    });
                    let children = rsx! {
                        for (n, key, node, marker) in entries {
                            if let Node::Para { kind, runs, layout } = node {
                                Para { key: "{key}", n, kind, runs, layout, marker, page }
                            }
                        }
                    };
                    let style = list_style(tag, depth);
                    match tag {
                        ListTag::Numbered => rsx! { ol { key: "l{first}", style, {children} } },
                        ListTag::Bullet => rsx! { ul { key: "l{first}", style, {children} } },
                        ListTag::Todo => rsx! { ul { key: "l{first}", class: "todo", style, {children} } },
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

/// A run's size and colour, as an inline style. The sizes are the ratios the written keywords
/// come to at the body's size, so the composer shows what the reader will see.
fn run_style(run: &Run) -> Option<String> {
    let mut style = Vec::new();
    if let Some(size) = run.marks.size {
        let em = match size {
            TextSize::Small => "0.85em",
            TextSize::Large => "1.2em",
            TextSize::Huge => "1.5em",
        };
        style.push(format!("font-size:{em}"));
    }
    if let Some(color) = run.marks.color {
        style.push(format!("color:{}", color.css()));
    }
    style_of(style)
}

/// A paragraph's own style: its alignment, how far in its depth puts it, and a numbered item's
/// own `marker`. A list item's depth is its list's, and code is neither aligned nor indented.
fn para_style(kind: ParaKind, layout: Layout, marker: Option<String>) -> Option<String> {
    let mut style = Vec::new();
    let step = match kind {
        ParaKind::Paragraph | ParaKind::Heading(_) => 2.5,
        ParaKind::Quote => 1.0,
        _ => 0.0,
    };
    if layout.depth > 0 && step > 0.0 {
        style.push(format!("margin-left:{}em", f32::from(layout.depth) * step));
    }
    if layout.align != Align::Left && kind != ParaKind::Code {
        style.push(format!("text-align:{}", layout.align.css()));
    }
    if let Some(marker) = marker {
        style.push(format!("list-style-type:\"{marker}\""));
    }
    style_of(style)
}

/// A paragraph's text. An empty one holds a `<br>` so the caret has a line to stand on, and a
/// trailing line break gets one so the empty last line shows.
fn text(runs: &[Run]) -> Element {
    let ends_open = runs.last().is_none_or(|run| run.text.ends_with('\n'));
    rsx! {
        for (index, run) in runs.iter().enumerate() {
            if let Some(url) = &run.marks.link {
                a { key: "{index}", class: "m-a {mark_class(run)}", style: run_style(run), href: "{url.as_str()}", "{run.text}" }
            } else {
                span { key: "{index}", class: "{mark_class(run)}", style: run_style(run), "{run.text}" }
            }
        }
        if ends_open {
            br {}
        }
    }
}

/// One paragraph, drawn as the element its kind is. `marker` is a numbered item's own number,
/// when its list does not count it ([`numbers`]).
#[component]
fn Para(
    n: usize,
    kind: ParaKind,
    runs: Vec<Run>,
    layout: Layout,
    marker: Option<String>,
    page: Signal<Page>,
) -> Element {
    #[cfg(test)]
    PARA_RENDERS.with(|renders| *renders.borrow_mut().entry(n).or_insert(0) += 1);
    let inner = text(&runs);
    let style = para_style(kind, layout, marker);
    match kind {
        ParaKind::Paragraph => rsx! { p { style, ..node_attrs(n), {inner} } },
        ParaKind::Heading(level) => match level.number() {
            1 => rsx! { h2 { style, ..node_attrs(n), {inner} } },
            2 => rsx! { h3 { style, ..node_attrs(n), {inner} } },
            _ => rsx! { h4 { style, ..node_attrs(n), {inner} } },
        },
        ParaKind::Bullet | ParaKind::Numbered => rsx! { li { style, ..node_attrs(n), {inner} } },
        ParaKind::Todo(check) => {
            let (class, next, checked) = match check {
                Check::Open => ("", Check::Done, "false"),
                Check::Done => ("done", Check::Open, "true"),
            };
            rsx! {
                li { style, ..todo_attrs(n, class),
                    span {
                        class: "box",
                        role: "checkbox",
                        aria_checked: checked,
                        aria_label: "Done",
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
        ParaKind::Quote => rsx! { blockquote { style, ..node_attrs(n), {inner} } },
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
        // quire's tip, not a `title`: the Mac help tag every other control in the window shows.
        Tooltip { text: "Options".to_owned(),
            span {
                class: "ograb",
                aria_label: "Options",
                onmounted: move |event: MountedEvent| handle.set(Some(MountedRef(event.data()))),
                onmousedown: move |event| event.prevent_default(),
                onclick: move |_| {
                    let next = if menu { Float::Closed } else { Float::Object(n) };
                    page.write().float = next;
                },
                "⋮⋮"
            }
        }
        if menu {
            Floating {
                placement: MenuPlacement::Popup,
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
            div { class: "obj o-sig", ..obj_attrs(n) }
        },
        Object::Image { src, alt } => {
            // The draft's own picture is drawn from the page's copy; any other source as written.
            // Peeked: a picture is in `images` before an image names it, and never leaves, so a
            // keystroke elsewhere need not draw the picture again.
            let drawn = match src.cid() {
                Some(cid) => page.peek().images.get(cid).unwrap_or("").to_owned(),
                None => src.as_str().to_owned(),
            };
            rsx! {
                figure { class: "obj o-img", ..obj_attrs(n),
                    {grip(n, menu, page, handle)}
                    if drawn.is_empty() {
                        div { class: "pick",
                            Glyph { icon: Icon::Paperclip, size: IconSize::Base }
                            span { "Add an image" }
                        }
                    } else {
                        div { class: "imgbox", img { src: "{drawn}", alt: "{alt}" } }
                        if !alt.is_empty() {
                            figcaption { "{alt}" }
                        }
                    }
                }
            }
        }
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
                    Button {
                        bezel: Bezel::Inline,
                        icon: Icon::Corner,
                        // Who in the strong tone, when and the hint quieter.
                        label: TextLine::Runs(vec![
                            TextRun::new(who.clone(), RunTone::Strong),
                            TextRun::new(format!(", {when}"), RunTone::Faint),
                            TextRun::new(
                                if open { " — Hide Quoted Text" } else { " — Show Quoted Text" },
                                RunTone::Faint,
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
                Span::Strong(inner)
                | Span::Emphasis(inner)
                | Span::Link { spans: inner, .. }
                | Span::Styled { spans: inner, .. } => {
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
