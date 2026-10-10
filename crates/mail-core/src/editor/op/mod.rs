//! The five edits, each applied by [`apply`] and answered with an inverse.
//!
//! Insert, delete, split, merge, and set (a paragraph's kind or an inline mark). A delete
//! that crosses a node boundary, and a paste, put nodes back with [`Op::Replace`] or
//! [`Op::InsertNodes`]: one insert of text cannot restore an object or a second paragraph.
//! [`Op::Seq`] is several of these applied as one inverse.

use mail_mime::SafeUrl;

mod delete;
mod nodes;
mod paint;

use crate::editor::doc::{Doc, Mark, Marks, Node, ParaKind, Pos, Presence, Range, Run};
use crate::editor::error::OpError;
use crate::editor::text::{
    delete_text, grapheme_len, insert_text, node_len, normalize_runs, para_len,
};

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
    /// Break a paragraph in two at `at`. Both keep its kind.
    Split {
        /// The break.
        at: Pos,
    },
    /// Join `node` onto the paragraph before it.
    Merge {
        /// The right-hand node. The survivor keeps the left node's kind.
        node: usize,
    },
    /// Turn every paragraph `range` touches into `kind`. A collapsed range is that paragraph.
    SetKind {
        /// Which paragraphs.
        range: Range,
        /// The kind they become.
        kind: ParaKind,
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
        Op::SetMark { range, mark, on } => paint::set_mark(doc, range, mark, on),
        Op::SetLink { range, url } => paint::set_link(doc, range, url),
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
    let kind = match doc.nodes.get(at.node) {
        Some(Node::Para { kind, runs }) => {
            if at.offset > para_len(runs) {
                return Err(OpError::OutOfRange);
            }
            *kind
        }
        Some(Node::Object(_)) => return Err(OpError::NotText),
        None => return Err(OpError::OutOfRange),
    };
    let right = {
        let runs = paragraph_mut(doc, at)?;
        let len = para_len(runs);
        delete_text(runs, at.offset, len)?
    };
    doc.nodes.insert(at.node + 1, Node::para(kind, right));
    Ok(Op::Merge { node: at.node + 1 })
}

fn merge(doc: &mut Doc, node: usize) -> Result<Op, OpError> {
    if node == 0 || node >= doc.nodes.len() {
        return Err(OpError::CannotMerge);
    }
    let (left_kind, left_len, right_kind) = match (&doc.nodes[node - 1], &doc.nodes[node]) {
        (Node::Para { kind: left, runs }, Node::Para { kind: right, .. }) => {
            (*left, para_len(runs), *right)
        }
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
    if left_kind == right_kind {
        Ok(split_back)
    } else {
        Ok(Op::Seq(vec![
            split_back,
            Op::SetKind {
                range: Range {
                    start: Pos::new(node, 0),
                    end: Pos::new(node, 0),
                },
                kind: right_kind,
            },
        ]))
    }
}

fn set_kind(doc: &mut Doc, range: Range, kind: ParaKind) -> Result<Op, OpError> {
    let range = range.ordered();
    check_range(doc, range)?;
    let mut inverses = Vec::new();
    for index in kind_targets(doc, range) {
        let Node::Para { kind: slot, .. } = &mut doc.nodes[index] else {
            continue;
        };
        if *slot != kind {
            let previous = *slot;
            *slot = kind;
            inverses.push(Op::SetKind {
                range: Range {
                    start: Pos::new(index, 0),
                    end: Pos::new(index, 0),
                },
                kind: previous,
            });
        }
    }
    Ok(one_or_seq(inverses, Op::SetKind { range, kind }))
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

fn kind_targets(doc: &Doc, range: Range) -> Vec<usize> {
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
            Node::Para { kind, runs } => Node::para(kind, runs),
            other => other,
        })
        .collect()
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
