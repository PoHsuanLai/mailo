//! The five edits, each applied by [`apply`] and answered with an inverse.
//!
//! Insert, delete, split, merge, and set (a paragraph's kind, alignment or depth, or an inline
//! mark, link, size or colour). A delete
//! that crosses a node boundary, and a paste, put nodes back with [`Op::Replace`] or
//! [`Op::InsertNodes`]: one insert of text cannot restore an object or a second paragraph.
//! [`Op::Seq`] is several of these applied as one inverse.

use mail_mime::{Align, SafeUrl, TextColor, TextSize};

mod delete;
mod nodes;
mod paint;

use crate::doc::{Doc, Layout, Mark, Marks, Node, ParaKind, Pos, Presence, Range, Run};
use crate::error::OpError;
use crate::text::{delete_text, grapheme_len, insert_text, node_len, normalize_runs, para_len};

/// One edit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    /// Insert `text` at `at`, carrying `marks`.
    Insert {
        /// Where the text goes.
        at: Pos,
        /// The inserted text.
        text: String,
        /// Marks on that text.
        marks: Marks,
    },
    /// Delete `range`, which may cross paragraphs and swallow objects.
    Delete {
        /// What goes.
        range: Range,
    },
    /// Break a paragraph in two at `at`. Both keep its kind and layout.
    Split {
        /// The break.
        at: Pos,
    },
    /// Join `node` onto the paragraph before it.
    Merge {
        /// The right-hand node. The survivor keeps the left node's kind and layout.
        node: usize,
    },
    /// Turn every paragraph `range` touches into `kind`. A collapsed range is that paragraph.
    SetKind {
        /// Which paragraphs.
        range: Range,
        /// The kind they become.
        kind: ParaKind,
    },
    /// Align every paragraph `range` touches. A collapsed range is that paragraph.
    SetAlign {
        /// Which paragraphs.
        range: Range,
        /// How they align.
        align: Align,
    },
    /// Set the depth of every paragraph `range` touches, at most [`Layout::MAX_DEPTH`]. A
    /// collapsed range is that paragraph.
    SetDepth {
        /// Which paragraphs.
        range: Range,
        /// Their depth.
        depth: u8,
    },
    /// Set or clear `mark` over `range`.
    SetMark {
        /// Which text.
        range: Range,
        /// Which mark.
        mark: Mark,
        /// On or off.
        on: Presence,
    },
    /// Set or clear the link over `range`. `None` clears it.
    SetLink {
        /// Which text.
        range: Range,
        /// The link, or none.
        url: Option<SafeUrl>,
    },
    /// Size the text of `range`. `None` is the body's size.
    SetSize {
        /// Which text.
        range: Range,
        /// The size, or none.
        size: Option<TextSize>,
    },
    /// Colour the text of `range`. `None` is the text's own colour.
    SetColor {
        /// Which text.
        range: Range,
        /// The colour, or none.
        color: Option<TextColor>,
    },
    /// Insert `nodes` at `at`, splitting a paragraph when `at` is in the middle of one.
    InsertNodes {
        /// Where they go.
        at: Pos,
        /// What is inserted, in order.
        nodes: Vec<Node>,
    },
    /// Replace `take` nodes at `index` with `nodes`. The inverse of a cross-node delete.
    Replace {
        /// First node touched.
        index: usize,
        /// How many current nodes to remove.
        take: usize,
        /// What replaces them.
        nodes: Vec<Node>,
    },
    /// `ops` applied in order. The inverse is those inverses, reversed.
    Seq(Vec<Op>),
}

/// Apply `op` and return the edit that restores `doc`.
///
/// On `Err` the document is unchanged.
pub fn apply(doc: &mut Doc, op: Op) -> Result<Op, OpError> {
    let mut next = doc.clone();
    let inverse = apply_mut(&mut next, op)?;
    *doc = next;
    Ok(inverse)
}

/// Apply `ops` in order. On `Err` the document is unchanged.
pub fn apply_all(doc: &mut Doc, ops: Vec<Op>) -> Result<Vec<Op>, OpError> {
    let mut next = doc.clone();
    let mut inverses = Vec::with_capacity(ops.len());
    for op in ops {
        inverses.push(apply(&mut next, op)?);
    }
    *doc = next;
    Ok(inverses)
}

fn apply_mut(doc: &mut Doc, op: Op) -> Result<Op, OpError> {
    match op {
        Op::Insert { at, text, marks } => insert(doc, at, text, marks),
        Op::Delete { range } => delete::delete(doc, range),
        Op::Split { at } => split(doc, at),
        Op::Merge { node } => merge(doc, node),
        Op::SetKind { range, kind } => set_kind(doc, range, kind),
        Op::SetAlign { range, align } => set_align(doc, range, align),
        Op::SetDepth { range, depth } => set_depth(doc, range, depth),
        Op::SetMark { range, mark, on } => paint::set_mark(doc, range, mark, on),
        Op::SetLink { range, url } => paint::set_link(doc, range, url),
        Op::SetSize { range, size } => paint::set_size(doc, range, size),
        Op::SetColor { range, color } => paint::set_color(doc, range, color),
        Op::InsertNodes { at, nodes } => nodes::insert_nodes(doc, at, nodes),
        Op::Replace { index, take, nodes } => nodes::replace(doc, index, take, nodes),
        Op::Seq(ops) => {
            let mut inverses = Vec::with_capacity(ops.len());
            for op in ops {
                inverses.push(apply_mut(doc, op)?);
            }
            inverses.reverse();
            Ok(Op::Seq(inverses))
        }
    }
}

fn insert(doc: &mut Doc, at: Pos, text: String, marks: Marks) -> Result<Op, OpError> {
    if text.is_empty() {
        return Ok(Op::Delete {
            range: Range { start: at, end: at },
        });
    }
    let runs = paragraph_mut(doc, at)?;
    let len = grapheme_len(&text);
    insert_text(runs, at.offset, &text, marks)?;
    Ok(Op::Delete {
        range: Range {
            start: at,
            end: Pos::new(at.node, at.offset + len),
        },
    })
}

fn split(doc: &mut Doc, at: Pos) -> Result<Op, OpError> {
    let (kind, layout) = match doc.nodes.get(at.node) {
        Some(Node::Para { kind, runs, layout }) => {
            if at.offset > para_len(runs) {
                return Err(OpError::OutOfRange);
            }
            (*kind, *layout)
        }
        Some(Node::Object(_)) => return Err(OpError::NotText),
        None => return Err(OpError::OutOfRange),
    };
    let right = {
        let runs = paragraph_mut(doc, at)?;
        let len = para_len(runs);
        delete_text(runs, at.offset, len)?
    };
    doc.nodes
        .insert(at.node + 1, Node::laid(kind, right, layout));
    Ok(Op::Merge { node: at.node + 1 })
}

fn merge(doc: &mut Doc, node: usize) -> Result<Op, OpError> {
    if node == 0 || node >= doc.nodes.len() {
        return Err(OpError::CannotMerge);
    }
    let (left, left_len, right) = match (&doc.nodes[node - 1], &doc.nodes[node]) {
        (
            Node::Para { kind, runs, layout },
            Node::Para {
                kind: right_kind,
                layout: right_layout,
                ..
            },
        ) => (
            (*kind, *layout),
            para_len(runs),
            (*right_kind, *right_layout),
        ),
        _ => return Err(OpError::CannotMerge),
    };
    let Node::Para {
        runs: right_runs, ..
    } = doc.nodes.remove(node)
    else {
        return Err(OpError::CannotMerge);
    };
    if let Node::Para { runs, .. } = &mut doc.nodes[node - 1] {
        runs.extend(right_runs);
        normalize_runs(runs);
    }
    let split_back = Op::Split {
        at: Pos::new(node - 1, left_len),
    };
    // The split gives the right half the left's kind and layout; whatever of the right's own
    // differed is set back on it.
    let at = Range {
        start: Pos::new(node, 0),
        end: Pos::new(node, 0),
    };
    let mut ops = vec![split_back];
    // Setting the kind brings the depth along by `depth_for`, so the depth the right half has
    // then is not always the left's: it is set back after the kind, from what it is by then.
    let mut depth = left.1.depth;
    if left.0 != right.0 {
        ops.push(Op::SetKind {
            range: at,
            kind: right.0,
        });
        depth = depth_for(left.0, right.0, depth);
    }
    if left.1.align != right.1.align {
        ops.push(Op::SetAlign {
            range: at,
            align: right.1.align,
        });
    }
    if depth != right.1.depth {
        ops.push(Op::SetDepth {
            range: at,
            depth: right.1.depth,
        });
    }
    Ok(one_or_seq_kept(ops))
}

/// The depth a paragraph has after it changes from kind `from` to kind `to` at `depth`.
///
/// A depth means a level of the kind it sits in: an indent for a paragraph, heading or list
/// item, a quote level for a quote. It stays through a change within one of these and is
/// zero when the change crosses between them (a quote becoming a paragraph, any kind
/// becoming code), as [`crate::format::quote`] leaves it.
pub fn depth_for(from: ParaKind, to: ParaKind, depth: u8) -> u8 {
    fn family(kind: ParaKind) -> u8 {
        match kind {
            ParaKind::Quote => 1,
            ParaKind::Code => 2,
            _ => 0,
        }
    }
    if family(from) == family(to) && family(to) != 2 {
        depth
    } else {
        0
    }
}

/// Make every paragraph `range` touches `kind`, with the depth [`depth_for`] gives it, and
/// answer with the edits that set each changed paragraph back: its kind, then its depth.
fn set_kind(doc: &mut Doc, range: Range, kind: ParaKind) -> Result<Op, OpError> {
    let range = range.ordered();
    check_range(doc, range)?;
    let mut inverses = Vec::new();
    for index in kind_targets(doc, range) {
        let Node::Para {
            kind: slot, layout, ..
        } = &mut doc.nodes[index]
        else {
            continue;
        };
        let (previous, was) = (*slot, layout.depth);
        let depth = depth_for(previous, kind, was);
        *slot = kind;
        layout.depth = depth;
        let at = Range {
            start: Pos::new(index, 0),
            end: Pos::new(index, 0),
        };
        if previous != kind {
            inverses.push(Op::SetKind {
                range: at,
                kind: previous,
            });
        }
        if depth != was {
            inverses.push(Op::SetDepth {
                range: at,
                depth: was,
            });
        }
    }
    Ok(one_or_seq(inverses, Op::SetKind { range, kind }))
}

fn set_align(doc: &mut Doc, range: Range, align: Align) -> Result<Op, OpError> {
    set_para(
        doc,
        range,
        align,
        |node| match node {
            Node::Para { layout, .. } => Some(&mut layout.align),
            Node::Object(_) => None,
        },
        |range, align| Op::SetAlign { range, align },
    )
}

fn set_depth(doc: &mut Doc, range: Range, depth: u8) -> Result<Op, OpError> {
    set_para(
        doc,
        range,
        depth.min(Layout::MAX_DEPTH),
        |node| match node {
            Node::Para { layout, .. } => Some(&mut layout.depth),
            Node::Object(_) => None,
        },
        |range, depth| Op::SetDepth { range, depth },
    )
}

/// Where a paragraph keeps one of its settings: its kind, its alignment or its depth.
type Slot<T> = for<'a> fn(&'a mut Node) -> Option<&'a mut T>;

/// Set `value` in every paragraph `range` touches, through `slot`, and answer with the edits
/// that set each changed paragraph back, made by `op`.
fn set_para<T: Copy + PartialEq>(
    doc: &mut Doc,
    range: Range,
    value: T,
    slot: Slot<T>,
    op: fn(Range, T) -> Op,
) -> Result<Op, OpError> {
    let range = range.ordered();
    check_range(doc, range)?;
    let mut inverses = Vec::new();
    for index in kind_targets(doc, range) {
        let Some(slot) = slot(&mut doc.nodes[index]) else {
            continue;
        };
        if *slot != value {
            let previous = *slot;
            *slot = value;
            let at = Pos::new(index, 0);
            inverses.push(op(Range { start: at, end: at }, previous));
        }
    }
    Ok(one_or_seq(inverses, op(range, value)))
}

fn paragraph_mut(doc: &mut Doc, at: Pos) -> Result<&mut Vec<Run>, OpError> {
    match doc.nodes.get_mut(at.node) {
        Some(Node::Para { runs, .. }) => {
            if at.offset > para_len(runs) {
                Err(OpError::OutOfRange)
            } else {
                Ok(runs)
            }
        }
        Some(Node::Object(_)) => Err(OpError::NotText),
        None => Err(OpError::OutOfRange),
    }
}

fn check_range(doc: &Doc, range: Range) -> Result<(), OpError> {
    check_pos(doc, range.start)?;
    check_pos(doc, range.end)
}

fn check_pos(doc: &Doc, pos: Pos) -> Result<(), OpError> {
    match doc.nodes.get(pos.node) {
        Some(node) if pos.offset <= node_len(node) => Ok(()),
        Some(_) | None => Err(OpError::OutOfRange),
    }
}

/// The paragraphs `range` touches, by index: the caret's own when it is collapsed.
pub(crate) fn kind_targets(doc: &Doc, range: Range) -> Vec<usize> {
    if range.is_collapsed() {
        return match doc.nodes.get(range.start.node) {
            Some(Node::Para { .. }) => vec![range.start.node],
            _ => Vec::new(),
        };
    }
    let mut out = Vec::new();
    for (index, node) in doc.nodes.iter().enumerate() {
        let Node::Para { .. } = node else {
            continue;
        };
        if covers(index, node_len(node), range) {
            out.push(index);
        }
    }
    out
}

fn covers(index: usize, len: usize, range: Range) -> bool {
    if len == 0 {
        return range.start.node < index && range.end.node > index;
    }
    let starts_inside =
        range.start.node < index || (range.start.node == index && range.start.offset < len);
    let ends_inside = range.end.node > index || (range.end.node == index && range.end.offset > 0);
    starts_inside && ends_inside
}

fn normalize_nodes(nodes: Vec<Node>) -> Vec<Node> {
    nodes
        .into_iter()
        .map(|node| match node {
            Node::Para { kind, runs, layout } => Node::laid(kind, runs, layout),
            other => other,
        })
        .collect()
}

/// `ops` as one edit: the only one, or all of them in order.
fn one_or_seq_kept(mut ops: Vec<Op>) -> Op {
    if ops.len() == 1 {
        ops.remove(0)
    } else {
        Op::Seq(ops)
    }
}

fn one_or_seq(mut inverses: Vec<Op>, noop: Op) -> Op {
    if inverses.is_empty() {
        noop
    } else if inverses.len() == 1 {
        inverses.remove(0)
    } else {
        Op::Seq(inverses)
    }
}
