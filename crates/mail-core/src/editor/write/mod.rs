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

/// The index just past the run of items of the list `tag` that starts at `start`.
fn list_end(nodes: &[Node], start: usize, tag: ListTag) -> usize {
    let same = |node: &Node| match node {
        Node::Para { kind, .. } => ListTag::of(*kind)
            .is_some_and(|other| std::mem::discriminant(&other) == std::mem::discriminant(&tag)),
        Node::Object(_) => false,
    };
    start + nodes[start..].iter().take_while(|node| same(node)).count()
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
            Span::Strong(inner) | Span::Emphasis(inner) | Span::Link { spans: inner, .. } => {
                out.push_str(&spans_plain(inner));
            }
        }
    }
    out
}
