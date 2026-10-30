//! Paste and the HTML round trip: blocks in, editor nodes out.
//!
//! [`mail_mime::from_html`] speaks [`Block`]. The composer speaks [`Node`]. This is the
//! lossy direction used when the clipboard is HTML and when a written document is read back.

use mail_mime::{Block, ListKind, ParaStyle, Span};

use crate::doc::{
    Check, ImageRef, Layout, Level, Mark, Marks, Node, Object, ParaKind, Presence, Run, Table,
};
use crate::text::{delete_text, grapheme_len, runs_text};

/// Paragraphs from plain clipboard text.
///
/// A blank line starts a paragraph. A single newline stays inside the paragraph.
pub fn nodes_from_plain(text: &str) -> Vec<Node> {
    if text.is_empty() {
        return Vec::new();
    }
    split_paragraphs(text)
        .into_iter()
        .map(|para| Node::plain(ParaKind::Paragraph, para))
        .collect()
}

/// Editor nodes for a block document.
///
/// A list inside a list item is its items, one level deeper; a quote inside a quote is its
/// paragraphs, one level deeper. A paragraph's alignment and indent, and a run's size and
/// colour, come along. A heading deeper than 3 becomes a level-3 heading. Underline and strike
/// do not exist on [`Span`], so a paste cannot carry them.
pub fn nodes_from_blocks(blocks: &[Block]) -> Vec<Node> {
    let mut out = Vec::new();
    push_blocks(blocks, Within::default(), &mut out);
    out
}

/// What the blocks being read sit in: the depth of the list whose item holds them, and how many
/// quotes deep they are.
#[derive(Debug, Clone, Copy, Default)]
struct Within {
    list: Option<u8>,
    quotes: u8,
}

/// A paragraph's layout from its block style: its own indent is its depth.
fn laid(style: ParaStyle) -> Layout {
    Layout {
        align: style.align,
        depth: style.indent.min(Layout::MAX_DEPTH),
    }
}

/// A paragraph of `runs`: a quote at its level when it is quoted, else a paragraph indented as
/// its style says.
fn paragraph(runs: Vec<Run>, style: ParaStyle, within: Within) -> Node {
    match within.quotes.checked_sub(1) {
        Some(level) => Node::laid(
            ParaKind::Quote,
            runs,
            Layout {
                align: style.align,
                depth: level.min(Layout::MAX_DEPTH),
            },
        ),
        None => Node::laid(ParaKind::Paragraph, runs, laid(style)),
    }
}

fn push_blocks(blocks: &[Block], within: Within, out: &mut Vec<Node>) {
    for block in blocks {
        match block {
            Block::Paragraph { spans, style, .. } => {
                out.push(paragraph(runs_from_spans(spans), *style, within));
            }
            Block::Heading {
                level,
                spans,
                style,
            } => {
                let kind = Level::from_number(*level)
                    .map(ParaKind::Heading)
                    .unwrap_or(ParaKind::Heading(Level::Three));
                out.push(Node::laid(kind, runs_from_spans(spans), laid(*style)));
            }
            Block::List { kind: list, items } => {
                let kind = match list {
                    ListKind::Numbered => ParaKind::Numbered,
                    ListKind::Bulleted => ParaKind::Bullet,
                };
                let depth = within
                    .list
                    .map_or(0, |outer| outer.saturating_add(1))
                    .min(Layout::MAX_DEPTH);
                // An item's own paragraphs are not quoted by a quote the list sits in: the item
                // is what it is, as it always was.
                let inside = Within {
                    list: Some(depth),
                    quotes: 0,
                };
                for item in items {
                    let mut nodes = Vec::new();
                    push_blocks(item, inside, &mut nodes);
                    let item_layout = Layout {
                        depth,
                        ..Layout::default()
                    };
                    if nodes.is_empty() {
                        nodes.push(Node::laid(kind, Vec::new(), item_layout));
                    } else if let Some(Node::Para {
                        kind: slot,
                        runs,
                        layout,
                    }) = nodes.first_mut()
                        && matches!(slot, ParaKind::Paragraph)
                    {
                        *slot = match (kind, take_box(runs)) {
                            (ParaKind::Bullet, Some(check)) => ParaKind::Todo(check),
                            _ => kind,
                        };
                        *layout = Layout {
                            align: layout.align,
                            depth,
                        };
                    }
                    out.extend(nodes);
                }
            }
            Block::Quote { blocks, .. } => {
                let inside = Within {
                    quotes: within.quotes.saturating_add(1),
                    ..within
                };
                push_blocks(blocks, inside, out);
            }
            Block::Code { text, .. } => out.push(Node::plain(ParaKind::Code, text.clone())),
            Block::Table { head, rows } => {
                let mut table_rows = Vec::new();
                if let Some(head) = head {
                    table_rows.push(head.iter().map(|cell| spans_text(cell)).collect());
                }
                for row in rows {
                    table_rows.push(row.iter().map(|cell| spans_text(cell)).collect());
                }
                out.push(Node::Object(Object::Table(Table { rows: table_rows })));
            }
            Block::Image { src, alt, .. } => {
                let src = match src {
                    mail_mime::ImgSrc::Remote(url) => ImageRef::new(url.as_str()),
                    // A draft's own image, reopened: named by its part, as the HTML wrote it.
                    mail_mime::ImgSrc::Inline(uri) => ImageRef::of_cid(uri.cid()),
                    mail_mime::ImgSrc::Blocked { host } => ImageRef::new(host.clone()),
                };
                out.push(Node::Object(Object::Image {
                    src,
                    alt: alt.clone(),
                }));
            }
            Block::Rule => out.push(Node::Object(Object::Divider)),
            Block::Signature(inner) => {
                out.push(Node::Object(Object::Signature));
                push_blocks(inner, within, out);
            }
            Block::Button { label, url } => {
                let mut marks = Marks::new();
                marks.link = Some(url.clone());
                out.push(paragraph(
                    vec![Run::new(label.clone(), marks)],
                    ParaStyle::default(),
                    within,
                ));
            }
            Block::Facts(pairs) => {
                for (label, value) in pairs {
                    let mut text = spans_text(label);
                    if !text.is_empty() {
                        text.push(' ');
                    }
                    text.push_str(&spans_text(value));
                    out.push(paragraph(
                        vec![Run::new(text, Marks::new())],
                        ParaStyle::default(),
                        within,
                    ));
                }
            }
        }
    }
}

fn runs_from_spans(spans: &[Span]) -> Vec<Run> {
    let mut runs = Vec::new();
    walk_spans(spans, &Marks::new(), &mut runs);
    runs
}

fn walk_spans(spans: &[Span], marks: &Marks, out: &mut Vec<Run>) {
    for span in spans {
        match span {
            Span::Text(text) => push_run(out, text, marks.clone()),
            Span::Break => push_run(out, "\n", marks.clone()),
            Span::Strong(inner) => {
                let mut next = marks.clone();
                next.set(Mark::Bold, Presence::On);
                walk_spans(inner, &next, out);
            }
            Span::Emphasis(inner) => {
                let mut next = marks.clone();
                next.set(Mark::Italic, Presence::On);
                walk_spans(inner, &next, out);
            }
            Span::Code(text) => {
                // `Span::Code` is a string, so marks that wrapped it (bold inside is lost;
                // bold around it is this parent) have to be copied on by hand.
                let mut next = marks.clone();
                next.set(Mark::Code, Presence::On);
                push_run(out, text, next);
            }
            Span::Link { url, spans } => {
                let mut next = marks.clone();
                next.link = Some(url.clone());
                walk_spans(spans, &next, out);
            }
            Span::Styled { style, spans } => {
                let mut next = marks.clone();
                next.size = style.size.or(next.size);
                next.color = style.color.or(next.color);
                walk_spans(spans, &next, out);
            }
        }
    }
}

fn push_run(out: &mut Vec<Run>, text: &str, marks: Marks) {
    if text.is_empty() {
        return;
    }
    if let Some(last) = out.last_mut()
        && last.marks == marks
    {
        last.text.push_str(text);
        return;
    }
    out.push(Run::new(text, marks));
}

fn spans_text(spans: &[Span]) -> String {
    let mut out = String::new();
    for run in runs_from_spans(spans) {
        out.push_str(&run.text);
    }
    out
}

fn split_paragraphs(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut start = 0usize;
    let bytes = text.as_bytes();
    let mut index = 0usize;
    while index + 1 < bytes.len() {
        if bytes[index] == b'\n' && bytes[index + 1] == b'\n' {
            out.push(text[start..index].to_owned());
            index += 2;
            while index < bytes.len() && bytes[index] == b'\n' {
                index += 1;
            }
            start = index;
        } else {
            index += 1;
        }
    }
    out.push(text[start..].to_owned());
    out
}

/// The boxes a bulleted item may start with to be a to-do: the glyphs the HTML writer
/// uses, and the brackets people type.
const BOXES: &[(&str, Check)] = &[
    ("☐ ", Check::Open),
    ("☑ ", Check::Done),
    ("☒ ", Check::Done),
    ("[ ] ", Check::Open),
    ("[x] ", Check::Done),
    ("[X] ", Check::Done),
];

/// If `runs` start with a to-do box, remove it and say which. A box alone is an empty to-do.
fn take_box(runs: &mut Vec<Run>) -> Option<Check> {
    let text = runs_text(runs);
    if let Some((_, check)) = BOXES
        .iter()
        .find(|(prefix, _)| text.trim_end() == prefix.trim_end())
    {
        runs.clear();
        return Some(*check);
    }
    let (prefix, check) = BOXES.iter().find(|(prefix, _)| text.starts_with(prefix))?;
    delete_text(runs, 0, grapheme_len(prefix)).ok()?;
    Some(*check)
}
