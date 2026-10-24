//! The two bodies a draft sends, from the same document: minimal HTML for `Draft.html`, and
//! RFC 3676 `format=flowed` plain text for `Draft.text`.

mod flowed;
mod html;

pub use flowed::to_flowed;
pub use html::to_html;

use mail_mime::{Block, Span};

use crate::editor::doc::{Check, Node, ParaKind};

/// Which list a run of list items is. A list is written once around consecutive items.
#[derive(Clone, Copy)]
enum ListTag {
    Bullet,
    Numbered,
    Todo,
}

impl ListTag {
    /// The list a paragraph of `kind` belongs to, if it is a list item.
    fn of(kind: ParaKind) -> Option<Self> {
        match kind {
            ParaKind::Bullet => Some(Self::Bullet),
            ParaKind::Numbered => Some(Self::Numbered),
            ParaKind::Todo(_) => Some(Self::Todo),
            ParaKind::Paragraph | ParaKind::Heading(_) | ParaKind::Quote | ParaKind::Code => None,
        }
    }
}

/// The index just past the run of list items that starts at `start`: every item after it, at
/// any depth, until one that is not an item, or one back at the outer list's depth that is
/// another list (`tag` is the first item's).
fn list_end(nodes: &[Node], start: usize, tag: ListTag) -> usize {
    let goes_on = |node: &Node| match node {
        Node::Para { kind, layout, .. } => ListTag::of(*kind).is_some_and(|other| {
            layout.depth > 0 || std::mem::discriminant(&other) == std::mem::discriminant(&tag)
        }),
        Node::Object(_) => false,
    };
    start
        + 1
        + nodes[start + 1..]
            .iter()
            .take_while(|node| goes_on(node))
            .count()
}

/// The depth each item of a run is written at: its own, but at most one below the item before
/// it, and the first at the outer list's. A list nests inside an item, so it cannot skip one.
fn written_depths(items: &[Node]) -> Vec<usize> {
    let mut out: Vec<usize> = Vec::with_capacity(items.len());
    for node in items {
        let depth = match node {
            Node::Para { layout, .. } => usize::from(layout.depth),
            Node::Object(_) => 0,
        };
        let most = out.last().map_or(0, |before| before + 1);
        out.push(depth.min(most));
    }
    out
}

/// Whether two items are of the same list: both bullets, both numbered, or both to-dos.
fn same_list(one: ListTag, other: ListTag) -> bool {
    std::mem::discriminant(&one) == std::mem::discriminant(&other)
}

/// The box a to-do item is written with in plain text.
fn todo_box(check: Check) -> &'static str {
    match check {
        Check::Open => "[ ] ",
        Check::Done => "[x] ",
    }
}

/// A quoted block's words, for the folded original on a reply.
fn block_text(block: &Block) -> String {
    let joined = |blocks: &[Block], separator: &str| {
        blocks
            .iter()
            .map(block_text)
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join(separator)
    };
    match block {
        Block::Paragraph { spans, .. } | Block::Heading { spans, .. } => spans_plain(spans),
        Block::Code { text, .. } => text.clone(),
        Block::Quote { blocks, .. } | Block::Signature(blocks) => joined(blocks, "\n"),
        Block::List { items, .. } => items
            .iter()
            .map(|item| joined(item, " "))
            .collect::<Vec<_>>()
            .join("\n"),
        Block::Table { head, rows } => head
            .iter()
            .chain(rows.iter())
            .map(|row| {
                row.iter()
                    .map(|cell| spans_plain(cell))
                    .collect::<Vec<_>>()
                    .join(" | ")
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Block::Button { label, .. } => label.clone(),
        Block::Facts(pairs) => pairs
            .iter()
            .map(|(label, value)| format!("{} {}", spans_plain(label), spans_plain(value)))
            .collect::<Vec<_>>()
            .join("\n"),
        Block::Image { alt, .. } => alt.clone(),
        Block::Rule => String::new(),
    }
}

fn spans_plain(spans: &[Span]) -> String {
    let mut out = String::new();
    for span in spans {
        match span {
            Span::Text(text) | Span::Code(text) => out.push_str(text),
            Span::Break => out.push('\n'),
            Span::Strong(inner)
            | Span::Emphasis(inner)
            | Span::Link { spans: inner, .. }
            | Span::Styled { spans: inner, .. } => {
                out.push_str(&spans_plain(inner));
            }
        }
    }
    out
}
