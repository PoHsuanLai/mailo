//! A browser `beforeinput` event, turned into edits.
//!
//! **The IME rule.** While `composing` is set, an event produces no edits: not at
//! `compositionstart`, not for any `insertCompositionText`. The composed text is committed at
//! `compositionend` as exactly one [`Op::Insert`]. Zhuyin, Pinyin, and kana-to-kanji all
//! wait until the composition ends, so the document never holds a half-chosen character.

use unicode_segmentation::UnicodeSegmentation;

use crate::editor::doc::{Doc, Mark, Marks, Node, Pos, Presence, Range};
use crate::editor::error::OpError;
use crate::editor::keys::{self, Caret};
use crate::editor::op::{Op, apply};
use crate::editor::paste;
use crate::editor::text::{grapheme_len, marks_at, node_len, para_len, range_has_mark, runs_text};
use crate::editor::undo::Burst;

/// One `beforeinput` event, or a `compositionstart`/`compositionend` forwarded the same way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputEvent {
    /// The `inputType`, such as `insertText` or `deleteContentBackward`, or the name of a
    /// composition event.
    pub input_type: String,
    /// The text the event carries, when it carries text.
    pub data: Option<String>,
    /// The ranges from `getTargetRanges`, already converted to grapheme positions.
    pub ranges: Vec<Range>,
    /// Set for the whole composition, `insertCompositionText` included, and cleared on the
    /// `compositionend` that commits it.
    pub composing: bool,
    /// Clipboard HTML for `insertFromPaste`. Absent when the paste is plain text.
    pub html: Option<String>,
}

impl InputEvent {
    /// An event with no clipboard HTML.
    pub fn new(
        input_type: impl Into<String>,
        data: Option<String>,
        ranges: Vec<Range>,
        composing: bool,
    ) -> Self {
        Self {
            input_type: input_type.into(),
            data,
            ranges,
            composing,
            html: None,
        }
    }
}

/// How [`interpret`] wants the event recorded in the undo log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Record {
    /// Composition in progress, or an event this editor ignores. Nothing changes.
    Skip,
    /// Characters, which join the typing burst.
    Typing(Burst),
    /// One group of its own: Enter, paste, a format, a cut, a deletion.
    Structural,
    /// Undo the latest group.
    Undo,
    /// Redo the group last undone.
    Redo,
}

/// The edits for one event, and the caret after them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    /// Applied in order. Empty for [`Record::Skip`], [`Record::Undo`] and [`Record::Redo`].
    pub ops: Vec<Op>,
    /// The caret after `ops`.
    pub caret: Caret,
    /// How to record `ops`.
    pub record: Record,
}

/// Map `event` to edits. Does not modify `doc`.
pub fn interpret(doc: &Doc, caret: Caret, event: &InputEvent) -> Result<Edit, OpError> {
    if event.composing {
        return Ok(skip(caret));
    }
    let selection = event
        .ranges
        .first()
        .copied()
        .unwrap_or(Range {
            start: caret.pos,
            end: caret.pos,
        })
        .ordered();
    let data = event.data.as_deref().unwrap_or("");
    match event.input_type.as_str() {
        "compositionstart" | "compositionupdate" | "insertCompositionText" => Ok(skip(caret)),
        // A composition is one word's worth of text: the next keystroke starts a new burst.
        "compositionend" => insert_text(doc, selection, data, Burst::EndsWord),
        "insertText" | "insertReplacementText" => {
            let burst = if data.chars().any(char::is_whitespace) {
                Burst::EndsWord
            } else {
                Burst::Continues
            };
            insert_text(doc, selection, data, burst)
        }
        "insertLineBreak" => insert_text(doc, selection, "\n", Burst::EndsWord),
        "insertParagraph" => paragraph(doc, caret, selection),
        "deleteContentBackward" => backward(doc, caret, selection),
        "deleteContentForward" => forward(doc, selection),
        "deleteWordBackward" => word_backward(doc, caret, selection),
        "deleteByCut" | "deleteByDrag" | "deleteContent" => Ok(Edit {
            ops: delete_ranges(&event.ranges),
            caret: at(selection.start),
            record: Record::Structural,
        }),
        "insertFromPaste" | "insertFromDrop" => paste::paste(doc, selection, event),
        "formatBold" => format_mark(doc, selection, Mark::Bold),
        "formatItalic" => format_mark(doc, selection, Mark::Italic),
        "formatUnderline" => format_mark(doc, selection, Mark::Underline),
        "formatStrikeThrough" => format_mark(doc, selection, Mark::Strike),
        "historyUndo" => Ok(history(caret, Record::Undo)),
        "historyRedo" => Ok(history(caret, Record::Redo)),
        _ => Ok(skip(caret)),
    }
}

fn skip(caret: Caret) -> Edit {
    history(caret, Record::Skip)
}

fn history(caret: Caret, record: Record) -> Edit {
    Edit {
        ops: Vec::new(),
        caret,
        record,
    }
}

pub(crate) fn at(pos: Pos) -> Caret {
    Caret::at(pos.node, pos.offset)
}

/// Replace the selection with `text`, which takes the marks of the text before it.
fn insert_text(doc: &Doc, selection: Range, text: &str, burst: Burst) -> Result<Edit, OpError> {
    let mut ops = Vec::new();
    if !selection.is_collapsed() {
        ops.push(Op::Delete { range: selection });
    }
    let start = selection.start;
    if !text.is_empty() {
        let marks = match doc.nodes.get(start.node) {
            Some(Node::Para { runs, .. }) => marks_at(runs, start.offset.min(para_len(runs))),
            _ => Marks::new(),
        };
        ops.push(Op::Insert {
            at: start,
            text: text.to_owned(),
            marks,
        });
    }
    let record = if ops.is_empty() {
        Record::Skip
    } else if selection.is_collapsed() {
        Record::Typing(burst)
    } else {
        Record::Structural
    };
    Ok(Edit {
        ops,
        caret: Caret::at(start.node, start.offset + grapheme_len(text)),
        record,
    })
}

/// Enter: the selection goes, then [`keys::enter`] decides on what is left.
fn paragraph(doc: &Doc, caret: Caret, selection: Range) -> Result<Edit, OpError> {
    let (mut ops, after) = without_selection(doc, selection)?;
    let motion = keys::enter(
        &after,
        Caret {
            pos: selection.start,
            grip: caret.grip,
        },
    )?;
    ops.extend(motion.ops);
    Ok(Edit {
        ops,
        caret: motion.caret,
        record: Record::Structural,
    })
}

/// The delete of a selection, and the document as it will be after it.
pub(crate) fn without_selection(doc: &Doc, selection: Range) -> Result<(Vec<Op>, Doc), OpError> {
    let mut after = doc.clone();
    if selection.is_collapsed() {
        return Ok((Vec::new(), after));
    }
    let op = Op::Delete { range: selection };
    apply(&mut after, op.clone())?;
    Ok((vec![op], after))
}

fn delete_selection(selection: Range) -> Edit {
    Edit {
        ops: vec![Op::Delete { range: selection }],
        caret: at(selection.start),
        record: Record::Structural,
    }
}

fn backward(doc: &Doc, caret: Caret, selection: Range) -> Result<Edit, OpError> {
    if !selection.is_collapsed() {
        return Ok(delete_selection(selection));
    }
    let motion = keys::backspace(
        doc,
        Caret {
            pos: selection.start,
            grip: caret.grip,
        },
    )?;
    Ok(Edit {
        ops: motion.ops,
        caret: motion.caret,
        record: Record::Structural,
    })
}

/// Delete: one cluster forward, else join the next paragraph, else take the next object.
fn forward(doc: &Doc, selection: Range) -> Result<Edit, OpError> {
    if !selection.is_collapsed() {
        return Ok(delete_selection(selection));
    }
    let pos = selection.start;
    let Some(node) = doc.nodes.get(pos.node) else {
        return Ok(skip(at(pos)));
    };
    let range = if pos.offset < node_len(node) {
        Some(Range {
            start: pos,
            end: Pos::new(pos.node, pos.offset + 1),
        })
    } else {
        None
    };
    let ops = match (range, node, doc.nodes.get(pos.node + 1)) {
        (Some(range), _, _) => vec![Op::Delete { range }],
        (None, Node::Para { .. }, Some(Node::Para { .. })) => {
            vec![Op::Merge { node: pos.node + 1 }]
        }
        (None, _, Some(Node::Object(_))) => vec![Op::Delete {
            range: Range {
                start: Pos::new(pos.node + 1, 0),
                end: Pos::new(pos.node + 1, 1),
            },
        }],
        (None, _, _) => Vec::new(),
    };
    Ok(Edit {
        record: if ops.is_empty() {
            Record::Skip
        } else {
            Record::Structural
        },
        ops,
        caret: at(pos),
    })
}

/// Back to the start of the word before the caret. At a paragraph's start it is Backspace.
fn word_backward(doc: &Doc, caret: Caret, selection: Range) -> Result<Edit, OpError> {
    if !selection.is_collapsed() {
        return Ok(delete_selection(selection));
    }
    let pos = selection.start;
    let start = match doc.nodes.get(pos.node) {
        Some(Node::Para { runs, .. }) => word_start(&runs_text(runs), pos.offset),
        _ => pos.offset,
    };
    if start == pos.offset {
        return backward(doc, caret, selection);
    }
    Ok(delete_selection(Range {
        start: Pos::new(pos.node, start),
        end: pos,
    }))
}

fn word_start(text: &str, offset: usize) -> usize {
    let graphemes: Vec<&str> = text.graphemes(true).take(offset).collect();
    let is_space = |grapheme: &str| grapheme.chars().all(char::is_whitespace);
    let spaces = graphemes.iter().rev().take_while(|g| is_space(g)).count();
    let letters = graphemes[..graphemes.len() - spaces]
        .iter()
        .rev()
        .take_while(|g| !is_space(g))
        .count();
    graphemes.len() - spaces - letters
}

/// Each range deleted, the last first, so an earlier range's positions stay true.
fn delete_ranges(ranges: &[Range]) -> Vec<Op> {
    let mut ranges: Vec<Range> = ranges
        .iter()
        .copied()
        .map(Range::ordered)
        .filter(|range| !range.is_collapsed())
        .collect();
    ranges.sort_by_key(|range| std::cmp::Reverse((range.start.node, range.start.offset)));
    ranges
        .into_iter()
        .map(|range| Op::Delete { range })
        .collect()
}

/// Bold and the rest: on over the selection, unless it is all on already.
fn format_mark(doc: &Doc, selection: Range, mark: Mark) -> Result<Edit, OpError> {
    if selection.is_collapsed() {
        return Ok(skip(at(selection.start)));
    }
    let on = if selection_has_mark(doc, selection, mark) {
        Presence::Off
    } else {
        Presence::On
    };
    Ok(Edit {
        ops: vec![Op::SetMark {
            range: selection,
            mark,
            on,
        }],
        caret: at(selection.end),
        record: Record::Structural,
    })
}

/// Every paragraph the selection touches carries `mark` over the part it touches.
fn selection_has_mark(doc: &Doc, selection: Range, mark: Mark) -> bool {
    let (first, last) = (selection.start.node, selection.end.node);
    doc.nodes
        .iter()
        .enumerate()
        .take(last + 1)
        .skip(first)
        .all(|(index, node)| match node {
            Node::Para { runs, .. } => {
                let from = if index == first {
                    selection.start.offset
                } else {
                    0
                };
                let to = if index == last {
                    selection.end.offset
                } else {
                    para_len(runs)
                };
                range_has_mark(runs, from, to, mark)
            }
            Node::Object(_) => true,
        })
}
