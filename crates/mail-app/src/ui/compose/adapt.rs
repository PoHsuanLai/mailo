//! The adapter between quire's `EditSurface` and the editor core, on Blitz (`native`): what the
//! webview's glue did in script, as pure functions.
//!
//! - [`asked`] reads one [`EditInput`] as what it asks of the page: an editor [`InputEvent`],
//!   a caret move, or a clipboard gesture. The `/` and `@` menus' keys and the Ctrl chords
//!   (bold, undo…) are read before it, by the same code as on the webview (`body::key_taken`).
//! - [`pos_of`] and [`text_position`] convert positions both ways. quire counts UTF-8 bytes into
//!   a paragraph's own text; the editor counts grapheme clusters. A paragraph's text on the
//!   surface is its runs' text as written (the surface is `white-space: pre-wrap`), so the
//!   conversion goes through `runs_text`. An object is an atom: 0 before it, 1 after it, in both.
//! - [`step`], [`word_at`], [`para_at`] and [`selected_text`] are the caret moves and the
//!   selections the browser used to make, over the document alone. Up, Down, Home and End need
//!   the laid-out lines and are the surface's (`surface.rs`).

use dioxus::prelude::{Key, Modifiers};
use ds::{
    Composition, EditInput, EditPointer, Extend, KeyInput, Pasted, PointerPhase, TextPosition,
};
use unicode_segmentation::UnicodeSegmentation;

use crate::editor::{Doc, InputEvent, Node, Pos, Range, node_len, runs_text};

/// What one input asks of the page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Asked {
    /// An editor event, handed to the page as the glue's were (`wire::hear`): through the IME
    /// rule, acting on the page's own selection.
    Edit(InputEvent),
    /// Move the caret; with [`Reach::Extend`] only the selection's focus moves.
    Move(Step, Reach),
    /// Select the whole document.
    SelectAll,
    /// Put the selection on the clipboard.
    Copy,
    /// Put the selection on the clipboard, then delete it (`deleteByCut`).
    Cut,
    /// Nothing for the editor: Tab, a key it has no use for, a chord a parent handles.
    Nothing,
}

/// Whether a move carries the selection with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Reach {
    /// The caret moves; the selection goes.
    Collapse,
    /// The selection's focus moves; its anchor stays (Shift held).
    Extend,
}

/// A caret move.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Step {
    /// One grapheme back, across a paragraph's start into the one before.
    Left,
    /// One grapheme on.
    Right,
    /// To the start of this word, or the one before.
    WordLeft,
    /// To the end of this word, or the one after.
    WordRight,
    /// A line up: the laid-out line, so the surface's.
    Up,
    /// A line down.
    Down,
    /// The start of the laid-out line.
    LineStart,
    /// The end of the laid-out line.
    LineEnd,
    /// The start of the document.
    DocStart,
    /// The end of the document.
    DocEnd,
}

/// What `input` asks of the page. `ranges` is left empty: the page supplies its own selection.
pub(super) fn asked(input: &EditInput) -> Asked {
    match input {
        EditInput::Text(text) => edit("insertText", Some(text.clone()), false),
        EditInput::Key(key) => key_asked(key),
        EditInput::Composition(Composition::Start) => edit("compositionstart", None, true),
        EditInput::Composition(Composition::Update { text, .. }) => {
            edit("insertCompositionText", Some(text.clone()), true)
        }
        EditInput::Composition(Composition::End { text }) => {
            edit("compositionend", Some(text.clone()), false)
        }
        EditInput::Paste(Pasted::Text(text)) => edit("insertFromPaste", Some(text.clone()), false),
        EditInput::Paste(Pasted::Html { html, text }) => {
            let mut event =
                InputEvent::new("insertFromPaste", Some(text.clone()), Vec::new(), false);
            event.html = Some(html.clone()).filter(|html| !html.is_empty());
            Asked::Edit(event)
        }
        EditInput::Cut => Asked::Cut,
        EditInput::Copy => Asked::Copy,
    }
}

fn edit(input_type: &str, data: Option<String>, composing: bool) -> Asked {
    Asked::Edit(InputEvent::new(input_type, data, Vec::new(), composing))
}

/// A key the surface did not turn into text.
fn key_asked(key: &KeyInput) -> Asked {
    let held = key.modifiers;
    let command = held.intersects(Modifiers::CONTROL | Modifiers::META);
    let reach = if held.contains(Modifiers::SHIFT) {
        Reach::Extend
    } else {
        Reach::Collapse
    };
    let word = command || held.contains(Modifiers::ALT);
    match &key.key {
        // Ctrl Enter is Send, the page's.
        Key::Enter if command => Asked::Nothing,
        Key::Enter if held.contains(Modifiers::SHIFT) => edit("insertLineBreak", None, false),
        Key::Enter => edit("insertParagraph", None, false),
        Key::Backspace if word => edit("deleteWordBackward", None, false),
        Key::Backspace => edit("deleteContentBackward", None, false),
        Key::Delete => edit("deleteContentForward", None, false),
        Key::ArrowLeft if word => Asked::Move(Step::WordLeft, reach),
        Key::ArrowRight if word => Asked::Move(Step::WordRight, reach),
        Key::ArrowLeft => Asked::Move(Step::Left, reach),
        Key::ArrowRight => Asked::Move(Step::Right, reach),
        Key::ArrowUp => Asked::Move(Step::Up, reach),
        Key::ArrowDown => Asked::Move(Step::Down, reach),
        Key::Home if command => Asked::Move(Step::DocStart, reach),
        Key::End if command => Asked::Move(Step::DocEnd, reach),
        Key::Home => Asked::Move(Step::LineStart, reach),
        Key::End => Asked::Move(Step::LineEnd, reach),
        Key::Character(letter) if command && letter.eq_ignore_ascii_case("a") => Asked::SelectAll,
        _ => Asked::Nothing,
    }
}

/// The editor position quire's `at` names, clamped into the document. `None` when `at` names
/// no node of it.
pub(super) fn pos_of(doc: &Doc, at: &TextPosition) -> Option<Pos> {
    let node: usize = at.node.0.parse().ok()?;
    match doc.nodes.get(node)? {
        Node::Object(_) => Some(Pos::new(node, at.offset.0.min(1))),
        Node::Para { runs, .. } => {
            let text = runs_text(runs);
            let mut bytes = at.offset.0.min(text.len());
            while !text.is_char_boundary(bytes) {
                bytes -= 1;
            }
            // A byte inside a cluster counts the cluster it is in as not yet passed.
            let whole = text
                .grapheme_indices(true)
                .take_while(|(start, grapheme)| start + grapheme.len() <= bytes)
                .count();
            Some(Pos::new(node, whole))
        }
    }
}

/// quire's name for the editor position `pos`: its node's key, and bytes into its text.
pub(super) fn text_position(doc: &Doc, pos: Pos) -> TextPosition {
    let offset = match doc.nodes.get(pos.node) {
        Some(Node::Para { runs, .. }) => byte_of(&runs_text(runs), pos.offset),
        Some(Node::Object(_)) => pos.offset.min(1),
        None => 0,
    };
    TextPosition::new(pos.node.to_string(), offset)
}

/// The byte where grapheme `grapheme` of `text` starts, or the text's length past its end.
fn byte_of(text: &str, grapheme: usize) -> usize {
    text.grapheme_indices(true)
        .nth(grapheme)
        .map_or(text.len(), |(byte, _)| byte)
}

/// Where the caret goes from `from` by a step that needs only the document.
pub(super) fn step(doc: &Doc, from: Pos, step: Step) -> Pos {
    let len = |node: usize| doc.nodes.get(node).map_or(0, node_len);
    let last = doc.nodes.len().saturating_sub(1);
    match step {
        Step::Left if from.offset > 0 => Pos::new(from.node, from.offset - 1),
        Step::Left if from.node > 0 => Pos::new(from.node - 1, len(from.node - 1)),
        Step::Right if from.offset < len(from.node) => Pos::new(from.node, from.offset + 1),
        Step::Right if from.node < last => Pos::new(from.node + 1, 0),
        Step::WordLeft => match words(doc, from.node) {
            Some(bounds) if from.offset > 0 => {
                let to = bounds
                    .iter()
                    .rev()
                    .find(|(start, _)| *start < from.offset)
                    .map_or(0, |(start, _)| *start);
                Pos::new(from.node, to)
            }
            _ => self::step(doc, from, Step::Left),
        },
        Step::WordRight => match words(doc, from.node) {
            Some(bounds) if from.offset < len(from.node) => {
                let to = bounds
                    .iter()
                    .find(|(_, end)| *end > from.offset)
                    .map_or(len(from.node), |(_, end)| *end);
                Pos::new(from.node, to)
            }
            _ => self::step(doc, from, Step::Right),
        },
        Step::DocStart => Pos::new(0, 0),
        Step::DocEnd => Pos::new(last, len(last)),
        // The rest are the surface's, or the edge of the document.
        Step::Left | Step::Right | Step::Up | Step::Down | Step::LineStart | Step::LineEnd => from,
    }
}

/// The words of paragraph `node`, as grapheme `(start, end)` pairs. `None` for an object.
fn words(doc: &Doc, node: usize) -> Option<Vec<(usize, usize)>> {
    let Some(Node::Para { runs, .. }) = doc.nodes.get(node) else {
        return None;
    };
    let text = runs_text(runs);
    let mut out = Vec::new();
    let mut at = 0usize;
    for piece in text.split_word_bounds() {
        let len = piece.graphemes(true).count();
        if piece.chars().any(char::is_alphanumeric) {
            out.push((at, at + len));
        }
        at += len;
    }
    Some(out)
}

/// The word under `pos`, for a double click: the word it is in or touches, else the run of
/// spaces or the one mark it is on. An object selects itself.
pub(super) fn word_at(doc: &Doc, pos: Pos) -> Range {
    let Some(Node::Para { runs, .. }) = doc.nodes.get(pos.node) else {
        return para_at(doc, pos);
    };
    let text = runs_text(runs);
    let mut at = 0usize;
    let mut touching = None;
    for piece in text.split_word_bounds() {
        let len = piece.graphemes(true).count();
        let (start, end) = (at, at + len);
        at = end;
        let wordy = piece.chars().any(char::is_alphanumeric);
        if start <= pos.offset && pos.offset < end {
            // Inside this piece: it, unless it is not a word and one ends right here.
            return match touching {
                Some(word) if !wordy && start == pos.offset => word,
                _ => Range {
                    start: Pos::new(pos.node, start),
                    end: Pos::new(pos.node, end),
                },
            };
        }
        touching = wordy.then_some(Range {
            start: Pos::new(pos.node, start),
            end: Pos::new(pos.node, end),
        });
    }
    touching.unwrap_or(Range {
        start: pos,
        end: pos,
    })
}

/// The whole paragraph (or object) at `pos`, for a triple click.
pub(super) fn para_at(doc: &Doc, pos: Pos) -> Range {
    Range {
        start: Pos::new(pos.node, 0),
        end: Pos::new(pos.node, doc.nodes.get(pos.node).map_or(0, node_len)),
    }
}

/// The selection a press, drag or release asks for, as `(anchor, focus)`, given the current
/// selection's `anchor`: a press puts the caret where it lands; Shift, or a drag after a single
/// press, moves the focus from the anchor; a double press takes the word, a triple the
/// paragraph. `None` for a release, a drag after a double or triple press, or a point over
/// nothing addressable, which all leave the selection as it is.
pub(super) fn pointer_selection(
    doc: &Doc,
    anchor: Pos,
    pointer: &EditPointer,
) -> Option<(Pos, Pos)> {
    let at = pointer
        .position
        .as_ref()
        .and_then(|position| pos_of(doc, position))?;
    let range = |range: Range| Some((range.start, range.end));
    match (pointer.phase, pointer.clicks.0, pointer.extend) {
        (PointerPhase::Press, 2, _) => range(word_at(doc, at)),
        (PointerPhase::Press, 3, _) => range(para_at(doc, at)),
        (PointerPhase::Press, _, Extend::FromAnchor) | (PointerPhase::Drag, 1, _) => {
            Some((anchor, at))
        }
        (PointerPhase::Press, _, Extend::Fresh) => Some((at, at)),
        _ => None,
    }
}

/// The plain text `range` covers: paragraphs joined by line breaks, objects left out.
pub(super) fn selected_text(doc: &Doc, range: Range) -> String {
    let range = range.ordered();
    let mut lines = Vec::new();
    for (index, node) in doc
        .nodes
        .iter()
        .enumerate()
        .take(range.end.node + 1)
        .skip(range.start.node)
    {
        let Node::Para { runs, .. } = node else {
            continue;
        };
        let text = runs_text(runs);
        let from = if index == range.start.node {
            range.start.offset
        } else {
            0
        };
        let to = if index == range.end.node {
            range.end.offset
        } else {
            usize::MAX
        };
        let line: String = text
            .graphemes(true)
            .skip(from)
            .take(to.saturating_sub(from))
            .collect();
        lines.push(line);
    }
    lines.join("\n")
}

#[cfg(test)]
#[path = "adapt_tests.rs"]
mod tests;
