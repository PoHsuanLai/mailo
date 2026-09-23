//! The document, its undo log and the caret, driven one event at a time.
//!
//! This is what the composer holds. It owns no clock: every event arrives with the time the
//! caller read, which is what groups typing into undo steps.

use crate::editor::doc::{Doc, Marks, Node, Presence};
use crate::editor::error::OpError;
use crate::editor::input::{InputEvent, Record, interpret};
use crate::editor::keys::Caret;
use crate::editor::markdown::{self, Shortcut};
use crate::editor::op::{Op, apply_all};
use crate::editor::text::{marks_at, node_len};
use crate::editor::undo::Log;

/// The body being written, with its history and caret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    /// The body.
    pub doc: Doc,
    /// Undo and redo.
    pub log: Log,
    /// The caret.
    pub caret: Caret,
    /// Marks the next typed text takes instead of the marks before the caret. Set when an
    /// inline shortcut closes a mark: after `**hi**` the next word is not bold.
    pub pending: Option<Marks>,
}

impl Session {
    /// A blank paragraph.
    pub fn new() -> Self {
        Self::with(Doc::blank())
    }

    /// Editing `doc`, caret at its start.
    pub fn with(doc: Doc) -> Self {
        Self {
            doc,
            log: Log::new(),
            caret: Caret::at(0, 0),
            pending: None,
        }
    }

    /// Apply `event`, which happened at `at_ms` milliseconds. A Markdown shortcut that the
    /// event completes is applied after it, as an undo step of its own.
    pub fn handle(&mut self, event: &InputEvent, at_ms: u64) -> Result<(), OpError> {
        let edit = interpret(&self.doc, self.caret, event)?;
        match edit.record {
            Record::Skip => {
                self.caret = edit.caret;
                Ok(())
            }
            Record::Undo => {
                self.pending = None;
                self.log.undo(&mut self.doc)?;
                self.caret = clamp(&self.doc, self.caret);
                Ok(())
            }
            Record::Redo => {
                self.pending = None;
                self.log.redo(&mut self.doc)?;
                self.caret = clamp(&self.doc, self.caret);
                Ok(())
            }
            Record::Typing(burst) => {
                let ops = with_marks(edit.ops, self.pending.take());
                let inverses = apply_all(&mut self.doc, ops.clone())?;
                self.log.record_typing(ops, inverses, at_ms, burst);
                self.caret = edit.caret;
                if let Some(shortcut) = markdown::shortcuts(&self.doc, self.caret.pos) {
                    self.commit_shortcut(shortcut)?;
                }
                Ok(())
            }
            Record::Structural => {
                self.pending = None;
                let inverses = apply_all(&mut self.doc, edit.ops.clone())?;
                self.log.record_structural(edit.ops, inverses);
                self.caret = edit.caret;
                Ok(())
            }
        }
    }

    fn commit_shortcut(&mut self, shortcut: Shortcut) -> Result<(), OpError> {
        let inverses = apply_all(&mut self.doc, shortcut.ops.clone())?;
        self.log.record_structural(shortcut.ops, inverses);
        self.caret = Caret::at(shortcut.caret.node, shortcut.caret.offset);
        if let Some(mark) = shortcut.closes
            && let Some(Node::Para { runs, .. }) = self.doc.nodes.get(self.caret.pos.node)
        {
            let mut marks = marks_at(runs, self.caret.pos.offset);
            marks.set(mark, Presence::Off);
            self.pending = Some(marks);
        }
        Ok(())
    }
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

/// `ops` with every insert carrying `marks`, when there are marks to carry.
fn with_marks(ops: Vec<Op>, marks: Option<Marks>) -> Vec<Op> {
    let Some(marks) = marks else {
        return ops;
    };
    ops.into_iter()
        .map(|op| match op {
            Op::Insert { at, text, .. } => Op::Insert {
                at,
                text,
                marks: marks.clone(),
            },
            other => other,
        })
        .collect()
}

/// The nearest position that exists in `doc`. Undo does not say where the caret belongs.
fn clamp(doc: &Doc, caret: Caret) -> Caret {
    let Some(last) = doc.nodes.len().checked_sub(1) else {
        return Caret::at(0, 0);
    };
    let node = caret.pos.node.min(last);
    Caret::at(node, caret.pos.offset.min(node_len(&doc.nodes[node])))
}
