//! Inserting whole nodes (a paste), and replacing a stretch of nodes (an inverse).

use super::{Op, normalize_nodes, paragraph_mut};
use crate::editor::doc::{Doc, Node, Pos, Range};
use crate::editor::error::OpError;
use crate::editor::text::{delete_text, node_len, para_len};

pub(super) fn insert_nodes(doc: &mut Doc, at: Pos, nodes: Vec<Node>) -> Result<Op, OpError> {
    let nodes = normalize_nodes(nodes);
    if nodes.is_empty() {
        return Ok(Op::InsertNodes { at, nodes });
    }
    if doc.nodes.is_empty() {
        if at != Pos::new(0, 0) {
            return Err(OpError::OutOfRange);
        }
        let take = nodes.len();
        doc.nodes = nodes;
        return Ok(Op::Replace {
            index: 0,
            take,
            nodes: Vec::new(),
        });
    }
    if at.node == doc.nodes.len() && at.offset == 0 {
        let index = doc.nodes.len();
        let take = nodes.len();
        doc.nodes.extend(nodes);
        return Ok(Op::Replace {
            index,
            take,
            nodes: Vec::new(),
        });
    }
    let len = node_len(doc.nodes.get(at.node).ok_or(OpError::OutOfRange)?);
    if at.offset > len {
        return Err(OpError::OutOfRange);
    }
    if at.offset == 0 && len == 0 && matches!(doc.nodes[at.node], Node::Para { .. }) {
        let old = doc.nodes.remove(at.node);
        let take = nodes.len();
        doc.nodes.splice(at.node..at.node, nodes);
        return Ok(Op::Replace {
            index: at.node,
            take,
            nodes: vec![old],
        });
    }
    match &doc.nodes[at.node] {
        Node::Object(_) => {
            let index = if at.offset == 0 { at.node } else { at.node + 1 };
            let take = nodes.len();
            doc.nodes.splice(index..index, nodes);
            Ok(Op::Replace {
                index,
                take,
                nodes: Vec::new(),
            })
        }
        Node::Para { .. } => {
            if at.offset == 0 || at.offset == len {
                let index = if at.offset == 0 { at.node } else { at.node + 1 };
                let take = nodes.len();
                doc.nodes.splice(index..index, nodes);
                Ok(Op::Replace {
                    index,
                    take,
                    nodes: Vec::new(),
                })
            } else {
                let kind = match &doc.nodes[at.node] {
                    Node::Para { kind, .. } => *kind,
                    Node::Object(_) => return Err(OpError::NotText),
                };
                let right = {
                    let runs = paragraph_mut(doc, at)?;
                    let len = para_len(runs);
                    delete_text(runs, at.offset, len)?
                };
                let count = nodes.len();
                doc.nodes.insert(at.node + 1, Node::para(kind, right));
                doc.nodes.splice(at.node + 1..at.node + 1, nodes);
                Ok(Op::Delete {
                    range: Range {
                        start: Pos::new(at.node, at.offset),
                        end: Pos::new(at.node + count + 1, 0),
                    },
                })
            }
        }
    }
}

pub(super) fn replace(
    doc: &mut Doc,
    index: usize,
    take: usize,
    nodes: Vec<Node>,
) -> Result<Op, OpError> {
    if index > doc.nodes.len() || take > doc.nodes.len() - index {
        return Err(OpError::OutOfRange);
    }
    let nodes = normalize_nodes(nodes);
    let removed: Vec<Node> = doc
        .nodes
        .splice(index..index + take, nodes.clone())
        .collect();
    Ok(Op::Replace {
        index,
        take: nodes.len(),
        nodes: removed,
    })
}
