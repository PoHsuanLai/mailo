//! Plain text to the same blocks HTML lowers to.
//!
//! RFC 3676 for `format=flowed`: one leading space is stuffing and comes off,
//! a trailing space joins the next line, and `delsp` decides whether that
//! space was a marker or a real space. Getting `delsp` backwards inserts
//! spaces into CJK, which has none. A change of quote depth ends a paragraph
//! even mid-flow. `-- ` is not flowed, trailing space or not, and the
//! signature is the last such line at depth 0 — a quoted chain has earlier ones.
//!
//! A run is [`Block::Code`] on column alignment, rule characters, or source
//! shape. Prose is a veto, and a flowed line is never code: the sender said
//! it may be rewrapped, which is a fact rather than a guess.

use super::kind::{Block, Flowed, Span, first_strong, strip_overrides};
use super::limits::{Limits, Reached};
use super::quote::is_attribution;

pub(crate) struct TextOut {
    pub blocks: Vec<Block>,
    pub reached: Reached,
}

pub(crate) fn parse(text: &str, flowed: Flowed) -> TextOut {
    let mut reached = Reached::Nothing;
    let decoded = decode_lines(text, flowed);
    let split = last_signature(&decoded);
    let (body, signature) = match split {
        Some(index) => (&decoded[..index], Some(&decoded[index + 1..])),
        None => (decoded.as_slice(), None),
    };
    let mut blocks = pieces_to_blocks(&group(body), &mut reached);
    if let Some(signature) = signature {
        let inner = pieces_to_blocks(&group(signature), &mut reached);
        blocks.push(Block::Signature(inner));
    }
    if blocks.len() > Limits::MAX_BLOCKS {
        blocks.truncate(Limits::MAX_BLOCKS);
        note(&mut reached, Reached::Blocks);
    }
    TextOut { blocks, reached }
}

struct Line {
    depth: usize,
    text: String,
    /// Joins with the following line. Never set for `-- `.
    flowed: bool,
}

fn decode_lines(text: &str, mode: Flowed) -> Vec<Line> {
    let mut lines = Vec::new();
    for raw in text.split('\n') {
        let raw = raw.strip_suffix('\r').unwrap_or(raw);
        lines.push(decode_line(raw, mode));
        if lines.len() >= Limits::MAX_BLOCKS.saturating_mul(4) {
            break;
        }
    }
    lines
}

fn decode_line(mut raw: &str, mode: Flowed) -> Line {
    if matches!(mode, Flowed::Flowed { .. }) && raw.starts_with(' ') {
        raw = &raw[1..];
    }
    let mut depth = 0usize;
    while raw.starts_with('>') {
        depth += 1;
        raw = &raw[1..];
        // `> > >` is the spaced form some clients send. The space belongs to
        // the marker only when another `>` follows; otherwise it is the one
        // stuffing space after the run, removed below.
        if raw.starts_with(' ') && raw.as_bytes().get(1) == Some(&b'>') {
            raw = &raw[1..];
        }
        if depth > Limits::MAX_DEPTH.saturating_mul(2) {
            break;
        }
    }
    if depth > 0 && raw.starts_with(' ') {
        raw = &raw[1..];
    }
    let signature = depth == 0 && raw == "-- ";
    let flowed = matches!(mode, Flowed::Flowed { .. }) && !signature && raw.ends_with(' ');
    if flowed && matches!(mode, Flowed::Flowed { delsp: true }) {
        raw = &raw[..raw.len() - 1];
    }
    Line {
        depth,
        text: strip_overrides(raw),
        flowed,
    }
}

/// The last `-- ` at depth 0. Earlier ones, including quoted ones, stay put.
fn last_signature(lines: &[Line]) -> Option<usize> {
    lines
        .iter()
        .rposition(|line| line.depth == 0 && line.text == "-- ")
}

struct Piece {
    depth: usize,
    block: Block,
}

fn group(lines: &[Line]) -> Vec<Piece> {
    let mut pieces = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        if lines[index].text.is_empty() {
            index += 1;
            continue;
        }
        let depth = lines[index].depth;
        if lines[index].flowed {
            let start = index;
            index += 1;
            while index < lines.len()
                && lines[index].depth == depth
                && !lines[index].text.is_empty()
                && lines[index - 1].flowed
            {
                index += 1;
            }
            // A flowed line is one the sender said may be rewrapped.
            pieces.push(paragraph(depth, join_flowed(&lines[start..index])));
            continue;
        }
        let start = index;
        index += 1;
        while index < lines.len()
            && !lines[index].flowed
            && !lines[index].text.is_empty()
            && lines[index].depth == depth
        {
            index += 1;
        }
        let run = &lines[start..index];
        if is_code(run) {
            let text = run
                .iter()
                .map(|line| line.text.trim_end())
                .collect::<Vec<_>>()
                .join("\n");
            pieces.push(Piece {
                depth,
                block: Block::Code {
                    lang: code_lang(&text),
                    text,
                },
            });
        } else {
            pieces.push(paragraph(depth, join_fixed(run)));
        }
    }
    pieces
}

fn paragraph(depth: usize, text: String) -> Piece {
    let dir = first_strong(&text);
    Piece {
        depth,
        block: Block::Paragraph {
            spans: vec![Span::Text(text)],
            dir,
        },
    }
}

fn join_flowed(lines: &[Line]) -> String {
    let mut out = String::new();
    for line in lines {
        out.push_str(&line.text);
    }
    out.trim().to_owned()
}

fn join_fixed(lines: &[Line]) -> String {
    lines
        .iter()
        .map(|line| line.text.trim())
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn pieces_to_blocks(pieces: &[Piece], reached: &mut Reached) -> Vec<Block> {
    let mut stack: Vec<(usize, Vec<Block>)> = vec![(0, Vec::new())];
    for piece in pieces {
        let depth = piece.depth.min(Limits::MAX_DEPTH);
        if piece.depth > Limits::MAX_DEPTH {
            note(reached, Reached::Depth);
        }
        while stack.last().is_some_and(|top| top.0 > depth) {
            close_quote(&mut stack);
        }
        while stack.last().is_some_and(|top| top.0 < depth) {
            let next = stack.last().map(|top| top.0 + 1).unwrap_or(depth);
            if next > Limits::MAX_DEPTH {
                note(reached, Reached::Depth);
                break;
            }
            stack.push((next, Vec::new()));
        }
        if let Some(top) = stack.last_mut() {
            if top.1.len() >= Limits::MAX_BLOCKS {
                note(reached, Reached::Blocks);
            } else {
                top.1.push(piece.block.clone());
            }
        }
    }
    while stack.len() > 1 {
        close_quote(&mut stack);
    }
    stack.pop().map(|(_, blocks)| blocks).unwrap_or_default()
}

fn close_quote(stack: &mut Vec<(usize, Vec<Block>)>) {
    let Some((_, blocks)) = stack.pop() else {
        return;
    };
    let Some((_, parent)) = stack.last_mut() else {
        return;
    };
    let attribution = attribution_of(parent);
    if parent.len() >= Limits::MAX_BLOCKS {
        return;
    }
    parent.push(Block::Quote {
        attribution,
        blocks,
    });
}

fn attribution_of(parent: &mut Vec<Block>) -> Option<Vec<Span>> {
    let text = match parent.last() {
        Some(Block::Paragraph { spans, .. }) => span_text_plain(spans),
        _ => return None,
    };
    if !is_attribution(&text) {
        return None;
    }
    match parent.pop() {
        Some(Block::Paragraph { spans, .. }) => Some(spans),
        Some(other) => {
            parent.push(other);
            None
        }
        None => None,
    }
}

fn span_text_plain(spans: &[Span]) -> String {
    let mut out = String::new();
    for span in spans {
        if let Span::Text(text) = span {
            out.push_str(text);
        }
    }
    out
}

/// Column alignment, rule lines, or a source shape. Letter-heavy prose is a
/// veto, so a justified newsletter's extra spaces do not become code.
fn is_code(lines: &[Line]) -> bool {
    if lines.len() < 2 {
        return false;
    }
    let texts: Vec<&str> = lines.iter().map(|line| line.text.as_str()).collect();
    if looks_like_source(&texts) {
        return true;
    }
    let mut letters = 0u32;
    let mut symbols = 0u32;
    let mut rules = 0u32;
    for text in &texts {
        if is_rule_line(text) {
            rules += 1;
        }
        for ch in text.chars() {
            if ch.is_alphabetic() {
                letters += 1;
            } else if matches!(
                ch,
                '{' | '}' | '(' | ')' | ';' | '|' | '+' | '=' | '[' | ']' | '<' | '>'
            ) {
                symbols += 1;
            }
        }
    }
    if rules > 0 {
        return true;
    }
    let letter_heavy = letters > 20 && symbols.saturating_mul(8) < letters;
    if letter_heavy {
        return false;
    }
    stable_columns(&texts)
}

fn looks_like_source(lines: &[&str]) -> bool {
    lines.iter().any(|line| {
        let trimmed = line.trim_start();
        trimmed.starts_with("fn ")
            || trimmed.starts_with("pub fn ")
            || trimmed.starts_with("impl ")
            || line.contains("println!")
            || trimmed == "{"
            || trimmed == "}"
    })
}

fn code_lang(text: &str) -> Option<String> {
    let source = looks_like_source(&text.lines().collect::<Vec<_>>());
    source.then(|| "rust".to_owned())
}

fn is_rule_line(line: &str) -> bool {
    let line = line.trim();
    line.len() >= 4
        && line
            .chars()
            .filter(|ch| matches!(ch, '-' | '=' | '_' | '+' | '|'))
            .count()
            * 5
            >= line.chars().count() * 4
}

fn stable_columns(lines: &[&str]) -> bool {
    let mut hits = [0u16; 128];
    for line in lines {
        let mut column = 0usize;
        let mut run = 0usize;
        for ch in line.chars() {
            if ch == ' ' {
                run += 1;
            } else {
                if run >= 2 && column < hits.len() {
                    let at = column - run;
                    if at < hits.len() {
                        hits[at] = hits[at].saturating_add(1);
                    }
                }
                run = 0;
            }
            column += 1;
            if column >= 512 {
                break;
            }
        }
    }
    hits.iter().any(|count| *count >= 2)
}

fn note(reached: &mut Reached, next: Reached) {
    if *reached == Reached::Nothing {
        *reached = next;
    }
}
