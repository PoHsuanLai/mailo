//! Paste and the HTML round trip: blocks in, editor nodes out.
//!
//! [`mail_mime::from_html`] speaks [`Block`]. The composer speaks [`Node`]. This is the
//! lossy direction used when the clipboard is HTML and when a written document is read back.

use mail_mime::{Block, Span};

use crate::editor::doc::{
    Check, ImageRef, Level, Mark, Marks, Node, Object, ParaKind, Presence, Run, Table,
};
use crate::editor::text::{delete_text, grapheme_len, runs_text};

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
/// Nested lists become sibling items. A heading deeper than 3 becomes a level-3 heading.
/// Underline and strike do not exist on [`Span`], so a paste cannot carry them.
pub fn nodes_from_blocks(blocks: &[Block]) -> Vec<Node> {
    let mut out = Vec::new();
    push_blocks(blocks, &mut out);
    out
}

fn push_blocks(blocks: &[Block], out: &mut Vec<Node>) {
    for block in blocks {
        match block {
            Block::Paragraph { spans, .. } => {
                out.push(Node::para(ParaKind::Paragraph, runs_from_spans(spans)));
            }
            Block::Heading { level, spans } => {
                let kind = Level::from_number(*level)
                    .map(ParaKind::Heading)
                    .unwrap_or(ParaKind::Heading(Level::Three));
                out.push(Node::para(kind, runs_from_spans(spans)));
            }
            Block::List { ordered, items } => {
                let kind = if *ordered {
                    ParaKind::Numbered
                } else {
                    ParaKind::Bullet
                };
                for item in items {
                    let mut nodes = Vec::new();
                    push_blocks(item, &mut nodes);
                    if nodes.is_empty() {
                        nodes.push(Node::plain(kind, ""));
                    } else if let Some(Node::Para { kind: slot, runs }) = nodes.first_mut()
                        && matches!(slot, ParaKind::Paragraph)
                    {
                        *slot = match (kind, take_box(runs)) {
                            (ParaKind::Bullet, Some(check)) => ParaKind::Todo(check),
                            _ => kind,
                        };
                    }
                    out.extend(nodes);
                }
            }
            Block::Quote { blocks, .. } => {
                let mut nodes = Vec::new();
                push_blocks(blocks, &mut nodes);
                for node in &mut nodes {
                    if let Node::Para { kind, .. } = node
                        && matches!(kind, ParaKind::Paragraph)
                    {
                        *kind = ParaKind::Quote;
                    }
                }
                out.extend(nodes);
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
                let id = match src {
                    mail_mime::ImgSrc::Remote(url) => url.as_str().to_owned(),
                    mail_mime::ImgSrc::Inline(uri) => uri.as_str().to_owned(),
                    mail_mime::ImgSrc::Blocked { host } => host.clone(),
                };
                out.push(Node::Object(Object::Image {
                    src: ImageRef::new(id),
                    alt: alt.clone(),
                }));
            }
            Block::Rule => out.push(Node::Object(Object::Divider)),
            Block::Signature(inner) => {
                out.push(Node::Object(Object::Signature));
                push_blocks(inner, out);
            }
            Block::Button { label, url } => {
                let mut marks = Marks::new();
                marks.link = Some(url.clone());
                out.push(Node::para(
                    ParaKind::Paragraph,
                    vec![Run::new(label.clone(), marks)],
                ));
            }
            Block::Facts(pairs) => {
                for (label, value) in pairs {
                    let mut text = spans_text(label);
                    if !text.is_empty() {
                        text.push(' ');
                    }
                    text.push_str(&spans_text(value));
                    out.push(Node::plain(ParaKind::Paragraph, text));
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
