//! Deleting a range: inside one paragraph, or across paragraphs and objects.

use super::{Op, check_range, paragraph_mut};
use crate::editor::doc::{Doc, Marks, Node, ParaKind, Pos, Range, Run};
use crate::editor::error::OpError;
use crate::editor::text::{delete_text, grapheme_len, normalize_runs, para_len};

pub(super) fn delete(doc: &mut Doc, range: Range) -> Result<Op, OpError> {
    let range = range.ordered();
    check_range(doc, range)?;
    if range.is_collapsed() {
        return Ok(Op::Delete { range });
    }
    if range.start.node == range.end.node {
        delete_one(doc, range)
    } else {
        delete_across(doc, range)
    }
}

fn delete_one(doc: &mut Doc, range: Range) -> Result<Op, OpError> {
    let index = range.start.node;
    match &doc.nodes[index] {
        Node::Object(_) => {
            if range.start.offset == 0 && range.end.offset >= 1 {
                let replacement = keep_one_paragraph(doc, index..=index, Vec::new());
                let take = replacement.len();
                let removed = doc.nodes.splice(index..=index, replacement).collect();
                Ok(Op::Replace {
                    index,
                    take,
                    nodes: removed,
                })
            } else {
                Err(OpError::OutOfRange)
            }
        }
        Node::Para { .. } => {
            let removed = {
                let runs = paragraph_mut(doc, range.start)?;
                delete_text(runs, range.start.offset, range.end.offset)?
            };
            Ok(inserts_of(index, range.start.offset, removed))
        }
    }
}

fn inserts_of(node: usize, at: usize, removed: Vec<Run>) -> Op {
    let mut ops = Vec::new();
    let mut cursor = at;
    for run in removed {
        let len = grapheme_len(&run.text);
        if len == 0 {
            continue;
        }
        ops.push(Op::Insert {
            at: Pos::new(node, cursor),
            text: run.text,
            marks: run.marks,
        });
        cursor += len;
    }
    if ops.is_empty() {
        Op::Insert {
            at: Pos::new(node, at),
            text: String::new(),
            marks: Marks::new(),
        }
    } else if ops.len() == 1 {
        ops.remove(0)
    } else {
        Op::Seq(ops)
    }
}

fn delete_across(doc: &mut Doc, range: Range) -> Result<Op, OpError> {
    let start_i = range.start.node;
    let end_i = range.end.node;
    let original = doc.nodes[start_i..=end_i].to_vec();
    let left = prefix_of(&doc.nodes[start_i], range.start.offset)?;
    let right = if range.end.offset == 0 {
        Some(doc.nodes[end_i].clone())
    } else {
        suffix_of(&doc.nodes[end_i], range.end.offset)?
    };
    let replacement = keep_one_paragraph(doc, start_i..=end_i, join(left, right));
    let take = replacement.len();
    doc.nodes.splice(start_i..=end_i, replacement);
    Ok(Op::Replace {
        index: start_i,
        take,
        nodes: original,
    })
}

fn prefix_of(node: &Node, offset: usize) -> Result<Option<Node>, OpError> {
    match node {
        Node::Object(_) => {
            if offset == 0 {
                Ok(None)
            } else {
                Ok(Some(node.clone()))
            }
        }
        // The paragraph the delete starts in survives, even emptied, with its kind: the
        // caret stays in it, as it does when a browser deletes a selection.
        Node::Para { kind, runs } => {
            let len = para_len(runs);
            let mut kept = runs.clone();
            delete_text(&mut kept, offset, len)?;
            Ok(Some(Node::Para {
                kind: *kind,
                runs: kept,
            }))
        }
    }
}

fn suffix_of(node: &Node, offset: usize) -> Result<Option<Node>, OpError> {
    match node {
        Node::Object(_) => {
            if offset >= 1 {
                Ok(None)
            } else {
                Ok(Some(node.clone()))
            }
        }
        Node::Para { kind, runs } => {
            let len = para_len(runs);
            if offset >= len {
                Ok(None)
            } else {
                let mut kept = runs.clone();
                delete_text(&mut kept, 0, offset)?;
                Ok(Some(Node::Para {
                    kind: *kind,
                    runs: kept,
                }))
            }
        }
    }
}

fn join(left: Option<Node>, right: Option<Node>) -> Vec<Node> {
    match (left, right) {
        (Some(Node::Para { kind, mut runs }), Some(Node::Para { runs: more, .. })) => {
            runs.extend(more);
            normalize_runs(&mut runs);
            vec![Node::Para { kind, runs }]
        }
        (Some(left), Some(right)) => vec![left, right],
        (Some(node), None) | (None, Some(node)) => vec![node],
        (None, None) => Vec::new(),
    }
}

/// `replacement`, or one empty paragraph when it would leave the document with no node at
/// all: there must always be somewhere to type.
fn keep_one_paragraph(
    doc: &Doc,
    removed: std::ops::RangeInclusive<usize>,
    replacement: Vec<Node>,
) -> Vec<Node> {
    let emptied = removed.end() + 1 - removed.start() == doc.nodes.len();
    if replacement.is_empty() && emptied {
        vec![Node::plain(ParaKind::Paragraph, "")]
    } else {
        replacement
    }
}
