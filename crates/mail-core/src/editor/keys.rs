//! Enter and Backspace at the edges of a paragraph, where the browser would do the wrong thing.
//!
//! Enter ends a heading. An empty list, to-do, or quote leaves that kind. Backspace at the
//! start of a styled paragraph turns it back into a paragraph before it will merge, and
//! Backspace just after an object arms the object so a second press deletes it.

use crate::editor::doc::{Check, Doc, Marks, Node, ParaKind, Pos, Range};
use crate::editor::error::OpError;
use crate::editor::op::Op;
use crate::editor::text::{node_len, para_len, runs_text};

/// The object Backspace will delete on the next press.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grip {
    /// No object is armed.
    Off,
    /// Backspace deletes this object.
    Armed(usize),
}

/// Where the caret is, and whether an object is armed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caret {
    /// The caret.
    pub pos: Pos,
    /// Armed by Backspace just after an object.
    pub grip: Grip,
}

impl Caret {
    /// A caret with nothing armed.
    pub fn at(node: usize, offset: usize) -> Self {
        Self {
            pos: Pos::new(node, offset),
            grip: Grip::Off,
        }
    }
}

/// What a key does: the edits, and where the caret is after them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Motion {
    /// Applied in order.
    pub ops: Vec<Op>,
    /// The caret after `ops`.
    pub caret: Caret,
}

/// Enter, without shift.
pub fn enter(doc: &Doc, caret: Caret) -> Result<Motion, OpError> {
    let Some(node) = doc.nodes.get(caret.pos.node) else {
        return Ok(Motion {
            ops: Vec::new(),
            caret: Caret::at(0, 0),
        });
    };
    match node {
        Node::Object(_) => Ok(Motion {
            ops: vec![insert_paragraph_at(if caret.pos.offset == 0 {
                caret.pos.node
            } else {
                caret.pos.node + 1
            })],
            caret: Caret::at(
                if caret.pos.offset == 0 {
                    caret.pos.node
                } else {
                    caret.pos.node + 1
                },
                0,
            ),
        }),
        Node::Para { kind, runs } => {
            let len = para_len(runs);
            let empty = runs_text(runs).trim().is_empty();
            if *kind == ParaKind::Code {
                return Ok(insert_newline(caret));
            }
            if empty && leaves_when_empty(*kind) {
                return Ok(Motion {
                    ops: vec![set_kind(caret.pos.node, ParaKind::Paragraph)],
                    caret: Caret::at(caret.pos.node, 0),
                });
            }
            if empty && matches!(kind, ParaKind::Heading(_)) {
                return Ok(Motion {
                    ops: vec![set_kind(caret.pos.node, ParaKind::Paragraph)],
                    caret: Caret::at(caret.pos.node, 0),
                });
            }
            let mut ops = vec![Op::Split { at: caret.pos }];
            let right = caret.pos.node + 1;
            let right_kind = right_kind_after_enter(*kind, caret.pos.offset == len);
            if let Some(kind) = right_kind {
                ops.push(set_kind(right, kind));
            }
            Ok(Motion {
                ops,
                caret: Caret::at(right, 0),
            })
        }
    }
}

/// Backspace.
///
/// `offset` counts graphemes, so one press deletes one cluster: a family emoji joined by
/// ZWJ goes in a single press.
pub fn backspace(doc: &Doc, caret: Caret) -> Result<Motion, OpError> {
    if let Grip::Armed(index) = caret.grip {
        return delete_armed(doc, index);
    }
    let Some(node) = doc.nodes.get(caret.pos.node) else {
        return Ok(still(caret));
    };
    match node {
        Node::Object(_) => {
            if caret.pos.offset >= 1 {
                Ok(arm(caret.pos.node, caret.pos))
            } else {
                boundary(doc, caret.pos.node, caret.pos)
            }
        }
        Node::Para { kind, .. } => {
            if caret.pos.offset > 0 {
                let start = one_cluster_back(caret.pos.offset);
                return Ok(Motion {
                    ops: vec![Op::Delete {
                        range: Range {
                            start: Pos::new(caret.pos.node, start),
                            end: caret.pos,
                        },
                    }],
                    caret: Caret::at(caret.pos.node, start),
                });
            }
            // A styled paragraph gives up its kind before a later press may merge it.
            if *kind != ParaKind::Paragraph {
                return Ok(Motion {
                    ops: vec![set_kind(caret.pos.node, ParaKind::Paragraph)],
                    caret: Caret::at(caret.pos.node, 0),
                });
            }
            boundary(doc, caret.pos.node, caret.pos)
        }
    }
}

/// One grapheme back from `offset`. The offset is already in grapheme clusters.
fn one_cluster_back(offset: usize) -> usize {
    offset.saturating_sub(1)
}

fn leaves_when_empty(kind: ParaKind) -> bool {
    matches!(
        kind,
        ParaKind::Bullet | ParaKind::Numbered | ParaKind::Todo(_) | ParaKind::Quote
    )
}

fn right_kind_after_enter(kind: ParaKind, at_end: bool) -> Option<ParaKind> {
    match kind {
        ParaKind::Heading(_) => Some(ParaKind::Paragraph),
        ParaKind::Quote if at_end => Some(ParaKind::Paragraph),
        ParaKind::Todo(_) => Some(ParaKind::Todo(Check::Open)),
        _ => None,
    }
}

fn boundary(doc: &Doc, node: usize, pos: Pos) -> Result<Motion, OpError> {
    if node == 0 {
        return Ok(still(Caret::at(pos.node, pos.offset)));
    }
    match &doc.nodes[node - 1] {
        Node::Object(_) => Ok(arm(node - 1, pos)),
        Node::Para { runs, .. } => {
            let join = para_len(runs);
            Ok(Motion {
                ops: vec![Op::Merge { node }],
                caret: Caret::at(node - 1, join),
            })
        }
    }
}

fn arm(index: usize, pos: Pos) -> Motion {
    Motion {
        ops: Vec::new(),
        caret: Caret {
            pos,
            grip: Grip::Armed(index),
        },
    }
}

fn delete_armed(doc: &Doc, index: usize) -> Result<Motion, OpError> {
    if index >= doc.nodes.len() || !matches!(doc.nodes[index], Node::Object(_)) {
        return Ok(still(Caret::at(index, 0)));
    }
    // Deleting the only node leaves an empty paragraph in its place (see `op::delete`).
    let caret = if index + 1 < doc.nodes.len() || index == 0 {
        Caret::at(index, 0)
    } else {
        Caret::at(index - 1, node_len(&doc.nodes[index - 1]))
    };
    let op = Op::Delete {
        range: Range {
            start: Pos::new(index, 0),
            end: Pos::new(index, 1),
        },
    };
    Ok(Motion {
        ops: vec![op],
        caret,
    })
}

fn insert_newline(caret: Caret) -> Motion {
    Motion {
        ops: vec![Op::Insert {
            at: caret.pos,
            text: "\n".to_owned(),
            marks: Marks::new(),
        }],
        caret: Caret::at(caret.pos.node, caret.pos.offset + 1),
    }
}

fn insert_paragraph_at(index: usize) -> Op {
    Op::InsertNodes {
        at: Pos::new(index, 0),
        nodes: vec![Node::plain(ParaKind::Paragraph, "")],
    }
}

fn set_kind(node: usize, kind: ParaKind) -> Op {
    Op::SetKind {
        range: Range {
            start: Pos::new(node, 0),
            end: Pos::new(node, 0),
        },
        kind,
    }
}

fn still(caret: Caret) -> Motion {
    Motion {
        ops: Vec::new(),
        caret,
    }
}
