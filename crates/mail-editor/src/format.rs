//! Paragraph levels and run styles over a selection: indent and outdent, quote levels, Tab in a
//! list, and the size and colour a selection starts with.
//!
//! Each returns the edits and changes nothing; the caller applies them as one undo step.
//! Alignment, size and colour need no help to be set: they are one [`Op`] each
//! ([`Op::SetAlign`], [`Op::SetSize`], [`Op::SetColor`]).

use mail_mime::{Align, TextColor, TextSize};

use crate::doc::{Doc, Layout, Marks, Node, ParaKind, Pos, Range};
use crate::op::{Op, apply_all, kind_targets};
use crate::text::{marks_at, para_len};
use crate::write::written_depths;

/// Which way a level goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nest {
    /// One level deeper: Tab, ⌘], ⌘'.
    In,
    /// One level shallower: Shift-Tab, ⌘[, ⌥⌘'.
    Out,
}

/// Whether a paragraph of `kind` is a list item.
pub fn is_item(kind: ParaKind) -> bool {
    matches!(
        kind,
        ParaKind::Bullet | ParaKind::Numbered | ParaKind::Todo(_)
    )
}

/// Indent or outdent every paragraph `range` touches.
///
/// A list item nests under the item before it, so it goes in only where there is one at its
/// depth or deeper: the first item of a list stays where it is. A paragraph or a heading is
/// indented. A quote has its own levels ([`quote`]) and code has none; both are left alone.
pub fn indent(doc: &Doc, range: Range, nest: Nest) -> Vec<Op> {
    let mut depths: Vec<Option<(ParaKind, u8)>> = doc
        .nodes
        .iter()
        .map(|node| match node {
            Node::Para { kind, layout, .. } => Some((*kind, layout.depth)),
            Node::Object(_) => None,
        })
        .collect();
    let mut ops = Vec::new();
    for index in kind_targets(doc, range.ordered()) {
        let Some((kind, depth)) = depths[index] else {
            continue;
        };
        if matches!(kind, ParaKind::Quote | ParaKind::Code) {
            continue;
        }
        let to = match nest {
            Nest::In if depth >= Layout::MAX_DEPTH => continue,
            Nest::In if is_item(kind) => {
                let under = index
                    .checked_sub(1)
                    .and_then(|before| depths[before])
                    .is_some_and(|(above, above_depth)| is_item(above) && above_depth >= depth);
                if !under {
                    continue;
                }
                depth + 1
            }
            Nest::In => depth + 1,
            Nest::Out if depth == 0 => continue,
            Nest::Out => depth - 1,
        };
        depths[index] = Some((kind, to));
        ops.push(depth_of(index, to));
    }
    ops
}

/// Quote one level more, or one less, every paragraph `range` touches.
///
/// In: a quote goes a level deeper, and any other paragraph but code becomes a quote. Out: a
/// quote below the first level comes up one, and a first-level quote becomes a paragraph.
pub fn quote(doc: &Doc, range: Range, nest: Nest) -> Vec<Op> {
    let mut ops = Vec::new();
    for index in kind_targets(doc, range.ordered()) {
        let Some(Node::Para { kind, layout, .. }) = doc.nodes.get(index) else {
            continue;
        };
        match (nest, *kind) {
            (_, ParaKind::Code) => {}
            (Nest::In, ParaKind::Quote) if layout.depth < Layout::MAX_DEPTH => {
                ops.push(depth_of(index, layout.depth + 1));
            }
            (Nest::In, ParaKind::Quote) => {}
            (Nest::In, _) => {
                ops.push(kind_of(index, ParaKind::Quote));
                if layout.depth > 0 {
                    ops.push(depth_of(index, 0));
                }
            }
            (Nest::Out, ParaKind::Quote) if layout.depth > 0 => {
                ops.push(depth_of(index, layout.depth - 1));
            }
            (Nest::Out, ParaKind::Quote) => ops.push(kind_of(index, ParaKind::Paragraph)),
            (Nest::Out, _) => {}
        }
    }
    ops
}

/// `ops` and, after them, the edits that bring every list item's depth to the depth it is sent
/// at: at most one below the item before it ([`written_depths`]). An edit that takes an item
/// out, lowers one, or turns one into a paragraph can leave the items after it deeper than the
/// list can nest; this is what keeps what the composer draws what the sender receives. `ops`
/// that do not apply are returned as they are.
pub fn settled(doc: &Doc, mut ops: Vec<Op>) -> Vec<Op> {
    let mut after = doc.clone();
    if apply_all(&mut after, ops.clone()).is_err() {
        return ops;
    }
    let mut start = 0;
    while start < after.nodes.len() {
        let item = |node: &Node| matches!(node, Node::Para { kind, .. } if is_item(*kind));
        if !item(&after.nodes[start]) {
            start += 1;
            continue;
        }
        let end = start
            + after.nodes[start..]
                .iter()
                .take_while(|node| item(node))
                .count();
        let written = written_depths(&after.nodes[start..end]);
        for (index, depth) in (start..end).zip(written) {
            if let Node::Para { layout, .. } = &after.nodes[index]
                && usize::from(layout.depth) != depth
            {
                let depth = u8::try_from(depth).unwrap_or(Layout::MAX_DEPTH);
                ops.push(depth_of(index, depth));
            }
        }
        start = end;
    }
    ops
}

/// Tab and Shift-Tab: in a list item they nest it or bring it out, and the edits are returned
/// (none, when it can go no further). `None` when the caret is not in a list item, where Tab is
/// not the editor's.
pub fn tab(doc: &Doc, range: Range, nest: Nest) -> Option<Vec<Op>> {
    let in_list = matches!(
        doc.nodes.get(range.ordered().start.node),
        Some(Node::Para { kind, .. }) if is_item(*kind)
    );
    in_list.then(|| indent(doc, range, nest))
}

/// The alignment of the paragraph `range` starts in.
pub fn align_at(doc: &Doc, range: Range) -> Align {
    match doc.nodes.get(range.ordered().start.node) {
        Some(Node::Para { layout, .. }) => layout.align,
        _ => Align::Left,
    }
}

/// The marks of the text `range` starts with: those a typed character would take, when it is
/// collapsed.
pub fn marks_at_start(doc: &Doc, range: Range) -> Marks {
    let range = range.ordered();
    let Some(Node::Para { runs, .. }) = doc.nodes.get(range.start.node) else {
        return Marks::new();
    };
    let offset = if range.is_collapsed() {
        range.start.offset
    } else {
        (range.start.offset + 1).min(para_len(runs))
    };
    marks_at(runs, offset)
}

/// The size of the text `range` starts with.
pub fn size_at(doc: &Doc, range: Range) -> Option<TextSize> {
    marks_at_start(doc, range).size
}

/// The colour of the text `range` starts with.
pub fn color_at(doc: &Doc, range: Range) -> Option<TextColor> {
    marks_at_start(doc, range).color
}

/// The size a step from `size` goes to: small, normal, large, huge, held at either end.
pub fn step_size(size: Option<TextSize>, nest: Nest) -> Option<TextSize> {
    const LADDER: [Option<TextSize>; 4] = [
        Some(TextSize::Small),
        None,
        Some(TextSize::Large),
        Some(TextSize::Huge),
    ];
    let at = LADDER.iter().position(|rung| *rung == size).unwrap_or(1);
    let to = match nest {
        Nest::In => (at + 1).min(LADDER.len() - 1),
        Nest::Out => at.saturating_sub(1),
    };
    LADDER[to]
}

fn at(index: usize) -> Range {
    let at = Pos::new(index, 0);
    Range { start: at, end: at }
}

fn depth_of(index: usize, depth: u8) -> Op {
    Op::SetDepth {
        range: at(index),
        depth,
    }
}

fn kind_of(index: usize, kind: ParaKind) -> Op {
    Op::SetKind {
        range: at(index),
        kind,
    }
}
