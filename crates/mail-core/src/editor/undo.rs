//! An op log of inverses, grouped by typing bursts.
//!
//! One group per word, or per pause over 600 ms. The caller passes the timestamps: this
//! module does not read a clock. A structural edit — paste, enter, a markdown shortcut —
//! is its own group.

use crate::editor::doc::Doc;
use crate::editor::error::OpError;
use crate::editor::op::{Op, apply_all};

/// Whether a keystroke leaves the typing burst open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Burst {
    /// More of the same word.
    Continues,
    /// A space or a committed composition: it joins the burst, then closes it.
    EndsWord,
}

/// How long a pause between keystrokes has to be before the next one starts a group.
pub const PAUSE_MS: u64 = 600;

/// Undo and redo stacks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Log {
    groups: Vec<Group>,
    redos: Vec<Group>,
    open: Option<Typing>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Group {
    /// Inverses, latest first.
    undo_ops: Vec<Op>,
    /// Forwards, in the order they were applied.
    redo_ops: Vec<Op>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Typing {
    undo_ops: Vec<Op>,
    redo_ops: Vec<Op>,
    last_ms: u64,
    /// [`Burst::EndsWord`] once a character ended the word: the next one starts a group.
    burst: Burst,
}

impl Log {
    /// An empty log.
    pub fn new() -> Self {
        Self {
            groups: Vec::new(),
            redos: Vec::new(),
            open: None,
        }
    }

    /// Record characters. A space closes the word after it has joined the group.
    pub fn record_typing(&mut self, redo: Vec<Op>, undo: Vec<Op>, at_ms: u64, burst: Burst) {
        self.redos.clear();
        let start_new = match &self.open {
            None => true,
            Some(open) => {
                open.burst == Burst::EndsWord || at_ms.saturating_sub(open.last_ms) > PAUSE_MS
            }
        };
        if start_new {
            self.close_typing();
            self.open = Some(Typing {
                undo_ops: Vec::new(),
                redo_ops: Vec::new(),
                last_ms: at_ms,
                burst: Burst::Continues,
            });
        }
        let Some(open) = self.open.as_mut() else {
            return;
        };
        open.redo_ops.extend(redo);
        let mut fresh = undo;
        fresh.reverse();
        fresh.append(&mut open.undo_ops);
        open.undo_ops = fresh;
        open.last_ms = at_ms;
        if burst == Burst::EndsWord {
            open.burst = Burst::EndsWord;
        }
    }

    /// Record one structural edit as its own group.
    pub fn record_structural(&mut self, redo: Vec<Op>, undo: Vec<Op>) {
        self.redos.clear();
        self.close_typing();
        let mut undo_ops = undo;
        undo_ops.reverse();
        self.groups.push(Group {
            undo_ops,
            redo_ops: redo,
        });
    }

    /// Undo the latest group. A no-op when the log is empty.
    pub fn undo(&mut self, doc: &mut Doc) -> Result<(), OpError> {
        self.close_typing();
        let Some(group) = self.groups.pop() else {
            return Ok(());
        };
        if let Err(err) = apply_all(doc, group.undo_ops.clone()) {
            self.groups.push(group);
            return Err(err);
        }
        self.redos.push(group);
        Ok(())
    }

    /// Redo the group most recently undone.
    pub fn redo(&mut self, doc: &mut Doc) -> Result<(), OpError> {
        let Some(group) = self.redos.pop() else {
            return Ok(());
        };
        if let Err(err) = apply_all(doc, group.redo_ops.clone()) {
            self.redos.push(group);
            return Err(err);
        }
        self.groups.push(group);
        Ok(())
    }

    /// Closed groups plus the burst still being typed.
    pub fn group_count(&self) -> usize {
        self.groups.len() + usize::from(self.open.is_some())
    }

    fn close_typing(&mut self) {
        let Some(open) = self.open.take() else {
            return;
        };
        if open.redo_ops.is_empty() {
            return;
        }
        self.groups.push(Group {
            undo_ops: open.undo_ops,
            redo_ops: open.redo_ops,
        });
    }
}

impl Default for Log {
    fn default() -> Self {
        Self::new()
    }
}
