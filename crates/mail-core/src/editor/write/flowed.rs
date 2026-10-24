//! RFC 3676 `format=flowed; delsp=no`.
//!
//! A line that ends in a space is soft: the reader joins it to the next one, and with
//! `delsp=no` the space stays in the text. Every other line is hard. So the writer:
//! - breaks a long line only after a space, and leaves that space at the end (a soft break);
//! - strips trailing spaces from every hard line, except the `-- ` signature separator;
//! - space-stuffs a line that would start with a space, `>` or `From ` (RFC 3676 §4.4);
//! - writes a quote as `>` then one space, which is itself the stuffing, then the text; a quote
//!   at a deeper level as one `>` for each level, then the space;
//! - writes a nested list item two spaces in for each level, and an indented paragraph four;
//!   the space-stuffing keeps them.
//!
//! Text with no space to break at (a long URL, a line of Chinese or Japanese) is not broken
//! at 72: breaking it would add a newline, or with a soft break a space, that the author did
//! not write. Only a line over RFC 5322's 998 octets is cut, at a grapheme boundary.

use unicode_segmentation::UnicodeSegmentation;

use super::{ListTag, block_text, list_end, same_list, todo_box, written_depths};
use crate::editor::doc::{Doc, Layout, Node, Object, ParaKind};
use crate::editor::text::runs_text;

/// Where a soft break is due, in grapheme clusters, the trailing space included.
const WIDTH: usize = 72;
/// RFC 5322 §2.1.1: no line may be longer than this many octets, CRLF excluded.
const LIMIT: usize = 998;
/// RFC 3676 §4.3: the signature separator, the one line that ends in a space and is not soft.
/// Only the signature object writes it; a paragraph that says `-- ` loses the space.
const SEPARATOR: &str = "-- ";

/// `format=flowed`, `delsp=no`, for `Draft.text`. Blocks are separated by a blank line.
pub fn to_flowed(doc: &Doc) -> String {
    let mut blocks = Vec::new();
    let mut index = 0;
    while index < doc.nodes.len() {
        match &doc.nodes[index] {
            Node::Para { kind, runs, layout } => match ListTag::of(*kind) {
                Some(tag) => {
                    let end = list_end(&doc.nodes, index, tag);
                    blocks.push(flow_list(&doc.nodes[index..end]));
                    index = end;
                }
                None => {
                    let text = runs_text(runs);
                    if !text.is_empty() {
                        blocks.push(flow_para(*kind, *layout, &text));
                    }
                    index += 1;
                }
            },
            Node::Object(object) => {
                if let Some(block) = flow_object(object) {
                    blocks.push(block);
                }
                index += 1;
            }
        }
    }
    if blocks.is_empty() {
        return String::new();
    }
    let mut out = blocks.join("\n\n");
    out.push('\n');
    out
}

/// Spaces before a nested list item, for each level.
const ITEM_INDENT: &str = "  ";
/// Spaces before every line of an indented paragraph, for each level.
const PARA_INDENT: &str = "    ";

/// How a block's lines are marked at their start.
#[derive(Clone, Copy)]
enum Lead<'a> {
    /// Plain text, stuffed where needed.
    Plain,
    /// The first line starts with this marker (`- `, `1. `, `[ ] `, after the spaces of its
    /// level); later lines are plain.
    Item(&'a str),
    /// Every line is quoted, as many levels deep as this.
    Quoted(usize),
    /// Every line starts with these spaces, then is stuffed.
    Indented(&'a str),
}

fn flow_para(kind: ParaKind, layout: Layout, text: &str) -> String {
    match kind {
        ParaKind::Quote => flow_text(text, Lead::Quoted(usize::from(layout.depth) + 1)),
        ParaKind::Code => flow_code(text),
        ParaKind::Paragraph | ParaKind::Heading(_) if layout.depth > 0 => flow_text(
            text,
            Lead::Indented(&PARA_INDENT.repeat(usize::from(layout.depth))),
        ),
        ParaKind::Paragraph
        | ParaKind::Heading(_)
        | ParaKind::Bullet
        | ParaKind::Numbered
        | ParaKind::Todo(_) => flow_text(text, Lead::Plain),
    }
}

/// A run of list items, each on its line, a nested one indented under the one before. A
/// numbered item counts among the numbered items of its own level since the last item that
/// was shallower.
fn flow_list(nodes: &[Node]) -> String {
    let mut counts: Vec<(ListTag, usize)> = Vec::new();
    let mut lines = Vec::new();
    for (node, depth) in nodes.iter().zip(written_depths(nodes)) {
        let Node::Para { kind, runs, .. } = node else {
            continue;
        };
        let Some(tag) = ListTag::of(*kind) else {
            continue;
        };
        counts.truncate(depth + 1);
        let number = match counts.get_mut(depth) {
            Some((last, count)) if same_list(*last, tag) => {
                *count += 1;
                *count
            }
            Some(slot) => {
                *slot = (tag, 1);
                1
            }
            None => {
                counts.push((tag, 1));
                1
            }
        };
        let marker = match (tag, kind) {
            (ListTag::Numbered, _) => format!("{number}. "),
            (ListTag::Todo, ParaKind::Todo(check)) => todo_box(*check).to_owned(),
            (ListTag::Bullet | ListTag::Todo, _) => "- ".to_owned(),
        };
        let marker = format!("{}{marker}", ITEM_INDENT.repeat(depth));
        lines.push(flow_text(&runs_text(runs), Lead::Item(&marker)));
    }
    lines.join("\n")
}

fn flow_object(object: &Object) -> Option<String> {
    match object {
        Object::Divider => Some("----".to_owned()),
        Object::Signature => Some(SEPARATOR.to_owned()),
        Object::Image { alt, .. } if alt.is_empty() => Some("[image]".to_owned()),
        Object::Image { alt, .. } => Some(flow_text(&format!("[image: {alt}]"), Lead::Plain)),
        Object::Attachment(file) => Some(flow_text(
            &format!("[attachment: {}]", file.name()),
            Lead::Plain,
        )),
        Object::Table(table) => Some(
            table
                .rows
                .iter()
                .map(|row| stuff(&hard(&row.join(" | "))))
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        Object::QuotedMessage { who, when, body } => {
            let mut lines = vec![flow_text(&format!("On {when}, {who} wrote:"), Lead::Plain)];
            lines.extend(
                body.iter()
                    .map(block_text)
                    .filter(|text| !text.is_empty())
                    .map(|text| flow_text(&text, Lead::Quoted(1))),
            );
            Some(lines.join("\n"))
        }
    }
}

/// Code is sent as the author wrote it: never soft-wrapped, only stuffed and cut at 998.
fn flow_code(text: &str) -> String {
    text.split('\n')
        .flat_map(|line| cut_long(line.trim_end_matches(' ')))
        .map(|line| stuff(&line))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Text with its own line breaks (Shift+Enter), each hard, each soft-wrapped.
fn flow_text(text: &str, lead: Lead<'_>) -> String {
    let mut out = Vec::new();
    for (hard_index, segment) in text.split('\n').enumerate() {
        let marker = match lead {
            Lead::Item(marker) if hard_index == 0 => marker,
            Lead::Indented(spaces) => spaces,
            Lead::Item(_) | Lead::Plain | Lead::Quoted(_) => "",
        };
        let quote = match lead {
            Lead::Quoted(levels) => levels + 1,
            Lead::Plain | Lead::Item(_) | Lead::Indented(_) => 0,
        };
        let budget = WIDTH.saturating_sub(marker.graphemes(true).count() + quote);
        let lines = wrap(segment, budget.max(1));
        let last = lines.len().saturating_sub(1);
        for (index, line) in lines.into_iter().enumerate() {
            let first = match lead {
                Lead::Indented(spaces) => spaces,
                _ if index == 0 => marker,
                _ => "",
            };
            let full = format!("{first}{line}");
            let full = if index == last { hard(&full) } else { full };
            out.push(match lead {
                Lead::Quoted(levels) if full.is_empty() => ">".repeat(levels),
                Lead::Quoted(levels) => format!("{} {full}", ">".repeat(levels)),
                Lead::Plain | Lead::Item(_) | Lead::Indented(_) => stuff(&full),
            });
        }
    }
    out.join("\n")
}

/// Greedy soft wrap. Each line but the last ends in the space it broke after.
///
/// A word is its letters and the spaces after it; a word wider than `width` gets a line to
/// itself rather than being cut. Widths count grapheme clusters.
fn wrap(segment: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut line_width = 0;
    for word in words(segment) {
        let word_width = word.graphemes(true).count();
        if line_width > 0 && line_width + word_width > width {
            lines.push(std::mem::take(&mut line));
            line_width = 0;
        }
        line.push_str(word);
        line_width += word_width;
    }
    lines.push(line);
    lines.into_iter().flat_map(|line| cut_long(&line)).collect()
}

/// Split after every run of spaces: "a b  c" is "a ", "b  ", "c".
fn words(segment: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut in_space = false;
    for (index, ch) in segment.char_indices() {
        if ch == ' ' {
            in_space = true;
        } else if in_space {
            out.push(&segment[start..index]);
            start = index;
            in_space = false;
        }
    }
    if start < segment.len() {
        out.push(&segment[start..]);
    }
    out
}

/// A line over 998 octets, cut at grapheme boundaries. Such a cut is a hard break: there is
/// no way to break text with no space in it that `delsp=no` undoes.
fn cut_long(line: &str) -> Vec<String> {
    let budget = LIMIT - 2; // room for a stuffing space, or `> `.
    if line.len() <= budget {
        return vec![line.to_owned()];
    }
    let mut out = Vec::new();
    let mut piece = String::new();
    for grapheme in line.graphemes(true) {
        if piece.len() + grapheme.len() > budget {
            out.push(std::mem::take(&mut piece));
        }
        piece.push_str(grapheme);
    }
    out.push(piece);
    out
}

/// A hard line: no trailing space, which would make the reader join it to the next line.
fn hard(line: &str) -> String {
    line.trim_end_matches(' ').to_owned()
}

/// RFC 3676 §4.4: a leading space keeps a line that starts with a space, `>` or `From `
/// from being read as stuffed, quoted, or an mbox separator.
fn stuff(line: &str) -> String {
    if line.starts_with(' ') || line.starts_with('>') || line.starts_with("From ") {
        format!(" {line}")
    } else {
        line.to_owned()
    }
}
