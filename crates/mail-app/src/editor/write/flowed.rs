//! RFC 3676 `format=flowed; delsp=no`.
//!
//! A line that ends in a space is soft: the reader joins it to the next one, and with
//! `delsp=no` the space stays in the text. Every other line is hard. So the writer:
//! - breaks a long line only after a space, and leaves that space at the end (a soft break);
//! - strips trailing spaces from every hard line, except the `-- ` signature separator;
//! - space-stuffs a line that would start with a space, `>` or `From ` (RFC 3676 §4.4);
//! - writes a quote as `>` then one space, which is itself the stuffing, then the text.
//!
//! Text with no space to break at (a long URL, a line of Chinese or Japanese) is not broken
//! at 72: breaking it would add a newline, or with a soft break a space, that the author did
//! not write. Only a line over RFC 5322's 998 octets is cut, at a grapheme boundary.

use unicode_segmentation::UnicodeSegmentation;

use super::{ListTag, block_text, list_end, todo_box};
use crate::editor::doc::{Doc, Node, Object, ParaKind};
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
            Node::Para { kind, runs } => match ListTag::of(*kind) {
                Some(tag) => {
                    let end = list_end(&doc.nodes, index, tag);
                    blocks.push(flow_list(&doc.nodes[index..end], tag));
                    index = end;
                }
                None => {
                    let text = runs_text(runs);
                    if !text.is_empty() {
                        blocks.push(flow_para(*kind, &text));
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

/// How a block's lines are marked at their start.
#[derive(Clone, Copy)]
enum Lead<'a> {
    /// Plain text, stuffed where needed.
    Plain,
    /// The first line starts with this marker (`- `, `1. `, `[ ] `); later lines are plain.
    Item(&'a str),
    /// Every line is quoted.
    Quoted,
}

fn flow_para(kind: ParaKind, text: &str) -> String {
    match kind {
        ParaKind::Quote => flow_text(text, Lead::Quoted),
        ParaKind::Code => flow_code(text),
        ParaKind::Paragraph
        | ParaKind::Heading(_)
        | ParaKind::Bullet
        | ParaKind::Numbered
        | ParaKind::Todo(_) => flow_text(text, Lead::Plain),
    }
}

fn flow_list(nodes: &[Node], tag: ListTag) -> String {
    nodes
        .iter()
        .enumerate()
        .filter_map(|(number, node)| match node {
            Node::Para { kind, runs } => Some((number, *kind, runs_text(runs))),
            Node::Object(_) => None,
        })
        .map(|(number, kind, text)| {
            let marker = match (tag, kind) {
                (ListTag::Numbered, _) => format!("{}. ", number + 1),
                (ListTag::Todo, ParaKind::Todo(check)) => todo_box(check).to_owned(),
                (ListTag::Bullet | ListTag::Todo, _) => "- ".to_owned(),
            };
            flow_text(&text, Lead::Item(&marker))
        })
        .collect::<Vec<_>>()
        .join("\n")
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
            let mut lines = vec![flow_text(&format!("On {who}, {when} wrote:"), Lead::Plain)];
            lines.extend(
                body.iter()
                    .map(block_text)
                    .filter(|text| !text.is_empty())
                    .map(|text| flow_text(&text, Lead::Quoted)),
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
            Lead::Item(_) | Lead::Plain | Lead::Quoted => "",
        };
        let quote = if matches!(lead, Lead::Quoted) { 2 } else { 0 };
        let budget = WIDTH.saturating_sub(marker.graphemes(true).count() + quote);
        let lines = wrap(segment, budget.max(1));
        let last = lines.len().saturating_sub(1);
        for (index, line) in lines.into_iter().enumerate() {
            let first = if index == 0 { marker } else { "" };
            let full = format!("{first}{line}");
            let full = if index == last { hard(&full) } else { full };
            out.push(match lead {
                Lead::Quoted if full.is_empty() => ">".to_owned(),
                Lead::Quoted => format!("> {full}"),
                Lead::Plain | Lead::Item(_) => stuff(&full),
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
