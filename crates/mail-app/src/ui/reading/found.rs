//! Where a highlight lands in a thread's parsed bodies.
//!
//! The blocks are walked in reading order, one text leaf at a time, and each leaf is named by
//! the same key the renderer gives it: the block path `blocks.rs` already keys elements with,
//! then `/` and the span's index. The renderer looks its marks up by that key, so the order it
//! happens to draw in cannot renumber a match, and Ctrl F's "3 of 12" counts exactly the marks
//! on the page.
//!
//! Only blocks. The Original frame is the sender's document in a sandbox; nothing here reads
//! it, and nothing here can mark it.

use super::super::marked::Numbering;
use crate::search::{Find, Highlight};
use mail_mime::{Block, Document, Shape, Span};
use std::collections::BTreeMap;
use std::ops::Range;

/// The key of a machine body's primary action, which the receipt draws before its blocks.
pub(super) const PRIMARY: &str = "primary";

/// One message's marks, by leaf key.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct Found {
    /// Each leaf's marks, and the thread-wide number of its first one.
    leaves: BTreeMap<String, (usize, Vec<Range<usize>>)>,
    current: Option<usize>,
}

impl Found {
    /// The marks for the leaf at `key`, and how to number them. No marks is an empty slice.
    pub(super) fn at(&self, key: &str) -> (&[Range<usize>], Numbering) {
        match self.leaves.get(key) {
            Some((first, ranges)) => (
                ranges.as_slice(),
                Numbering {
                    first: *first,
                    current: self.current,
                },
            ),
            None => (&[], Numbering::default()),
        }
    }

    /// Whether any leaf under the block path `prefix` has a mark. A folded quote with a match
    /// inside opens, so every match counted is one that can be seen.
    pub(super) fn inside(&self, prefix: &str) -> bool {
        let dotted = format!("{prefix}.");
        self.leaves
            .range(dotted.clone()..)
            .next()
            .is_some_and(|(key, _)| key.starts_with(&dotted))
    }
}

/// Mark `highlight` in each document, numbering across all of them.
///
/// `find`, when Ctrl F is open, picks the current match. The second value is the total, which
/// is what the find bar counts against.
pub(super) fn find_in(
    documents: &[Option<&Document>],
    highlight: &Highlight,
    find: Option<&Find>,
) -> (Vec<Found>, usize) {
    let mut total = 0;
    let mut out = Vec::with_capacity(documents.len());
    for document in documents {
        let mut found = Found::default();
        if let Some(document) = document
            && !highlight.is_empty()
        {
            for (key, text) in leaves(document) {
                let ranges = highlight.ranges(text);
                if ranges.is_empty() {
                    continue;
                }
                let count = ranges.len();
                found.leaves.insert(key, (total, ranges));
                total += count;
            }
        }
        out.push(found);
    }
    let current = find.and_then(|find| find.at(total));
    for found in &mut out {
        found.current = current;
    }
    (out, total)
}

/// Every text leaf in `document`, in reading order, with the key the renderer draws it under.
pub(super) fn leaves(document: &Document) -> Vec<(String, &str)> {
    let mut out = Vec::new();
    if document.shape == Shape::Machine
        && let Some(action) = &document.primary
    {
        out.push((PRIMARY.to_owned(), action.label.as_str()));
    }
    walk_blocks(&document.blocks, "0", &mut out);
    out
}

fn walk_blocks<'a>(blocks: &'a [Block], path: &str, out: &mut Vec<(String, &'a str)>) {
    for (index, block) in blocks.iter().enumerate() {
        walk_block(block, &format!("{path}.{index}"), out);
    }
}

fn walk_block<'a>(block: &'a Block, path: &str, out: &mut Vec<(String, &'a str)>) {
    match block {
        Block::Heading { spans, .. } | Block::Paragraph { spans, .. } => {
            walk_spans(spans, path, out);
        }
        Block::List { items, .. } => {
            for (index, item) in items.iter().enumerate() {
                walk_blocks(item, &format!("{path}.{index}"), out);
            }
        }
        Block::Quote {
            attribution,
            blocks,
        } => {
            if let Some(who) = attribution {
                walk_spans(who, &format!("{path}/who"), out);
            }
            walk_blocks(blocks, &format!("{path}.q"), out);
        }
        Block::Code { text, .. } => out.push((format!("{path}/code"), text.as_str())),
        Block::Table { head, rows } => {
            for (index, cell) in head.iter().flatten().enumerate() {
                walk_spans(cell, &format!("{path}/h{index}"), out);
            }
            for (row_index, row) in rows.iter().enumerate() {
                for (index, cell) in row.iter().enumerate() {
                    walk_spans(cell, &format!("{path}/r{row_index}c{index}"), out);
                }
            }
        }
        Block::Facts(pairs) => {
            for (index, (label, value)) in pairs.iter().enumerate() {
                walk_spans(label, &format!("{path}/k{index}"), out);
                walk_spans(value, &format!("{path}/v{index}"), out);
            }
        }
        Block::Button { label, .. } => out.push((format!("{path}/btn"), label.as_str())),
        Block::Signature(blocks) => walk_blocks(blocks, &format!("{path}.s"), out),
        Block::Image { .. } | Block::Rule => {}
    }
}

fn walk_spans<'a>(items: &'a [Span], key: &str, out: &mut Vec<(String, &'a str)>) {
    for (index, span) in items.iter().enumerate() {
        let here = format!("{key}/{index}");
        match span {
            Span::Text(text) | Span::Code(text) => out.push((here, text.as_str())),
            Span::Strong(inner) | Span::Emphasis(inner) | Span::Link { spans: inner, .. } => {
                walk_spans(inner, &here, out);
            }
            Span::Break => {}
        }
    }
}
