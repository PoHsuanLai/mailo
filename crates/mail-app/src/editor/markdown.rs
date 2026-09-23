//! Markdown typed at the caret, turned into a kind or a mark plus the deletion of the markers.
//!
//! The two edits are one undo step: the caller records the returned ops as a single group.

use crate::editor::doc::{Check, Doc, Level, Mark, Node, Object, ParaKind, Pos, Presence, Range};
use crate::editor::op::Op;
use crate::editor::text::{byte_at, grapheme_len, para_len, runs_text};

/// A shortcut ready to apply, and where the caret sits afterwards.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shortcut {
    /// SetKind or SetMark, then the deletes of the markers. A divider is one [`Op::Replace`].
    pub ops: Vec<Op>,
    /// The caret after `ops`.
    pub caret: Pos,
    /// The inline mark the shortcut closed, if it was inline. Text typed next does not
    /// continue it.
    pub closes: Option<Mark>,
}

/// The shortcut at `at`, if the text before the caret is one.
///
/// Line markers (`# `, `---`, …) fire only in a plain paragraph, and only when they are the
/// whole text before the caret. Inline markers fire in any paragraph except code.
pub fn shortcuts(doc: &Doc, at: Pos) -> Option<Shortcut> {
    let Node::Para { kind, runs } = doc.nodes.get(at.node)? else {
        return None;
    };
    if matches!(kind, ParaKind::Code) || at.offset > para_len(runs) {
        return None;
    }
    let text = runs_text(runs);
    let before = grapheme_prefix(&text, at.offset)?;
    if *kind == ParaKind::Paragraph
        && let Some(shortcut) = line_marker(at.node, before, &text)
    {
        return Some(shortcut);
    }
    inline_marker(at.node, before)
}

fn line_marker(node: usize, before: &str, text: &str) -> Option<Shortcut> {
    if before == "---" && text == "---" {
        return Some(Shortcut {
            ops: vec![Op::Replace {
                index: node,
                take: 1,
                nodes: vec![Node::Object(Object::Divider)],
            }],
            caret: Pos::new(node, 1),
            closes: None,
        });
    }
    const MARKERS: &[(&str, ParaKind)] = &[
        ("### ", ParaKind::Heading(Level::Three)),
        ("## ", ParaKind::Heading(Level::Two)),
        ("# ", ParaKind::Heading(Level::One)),
        ("- ", ParaKind::Bullet),
        ("* ", ParaKind::Bullet),
        ("1. ", ParaKind::Numbered),
        ("1) ", ParaKind::Numbered),
        ("[ ] ", ParaKind::Todo(Check::Open)),
        ("[] ", ParaKind::Todo(Check::Open)),
        ("> ", ParaKind::Quote),
        ("```", ParaKind::Code),
    ];
    let kind = MARKERS.iter().find(|(marker, _)| before == *marker)?.1;
    let len = grapheme_len(before);
    Some(Shortcut {
        ops: vec![
            Op::SetKind {
                range: Range {
                    start: Pos::new(node, 0),
                    end: Pos::new(node, 0),
                },
                kind,
            },
            Op::Delete {
                range: Range {
                    start: Pos::new(node, 0),
                    end: Pos::new(node, len),
                },
            },
        ],
        caret: Pos::new(node, 0),
        closes: None,
    })
}

fn inline_marker(node: usize, before: &str) -> Option<Shortcut> {
    if let Some(found) = wrap_token(before, "~~", '~') {
        return Some(marked(node, before, found, Mark::Strike));
    }
    if let Some(found) = wrap_token(before, "**", '*') {
        return Some(marked(node, before, found, Mark::Bold));
    }
    if let Some(found) = wrap_token(before, "`", '`') {
        return Some(marked(node, before, found, Mark::Code));
    }
    if let Some(found) = star_italic(before) {
        return Some(marked(node, before, found, Mark::Italic));
    }
    if let Some(found) = underscore_italic(before) {
        return Some(marked(node, before, found, Mark::Italic));
    }
    None
}

struct Wrapped<'a> {
    /// Byte index of the opening marker in the text before the caret.
    open_at: usize,
    open: &'a str,
    inner: &'a str,
    close: &'a str,
}

fn marked(node: usize, before: &str, found: Wrapped<'_>, mark: Mark) -> Shortcut {
    let start = grapheme_len(&before[..found.open_at]);
    let open_len = grapheme_len(found.open);
    let inner_len = grapheme_len(found.inner);
    let close_len = grapheme_len(found.close);
    let inner_from = start + open_len;
    let inner_to = inner_from + inner_len;
    Shortcut {
        ops: vec![
            Op::SetMark {
                range: Range {
                    start: Pos::new(node, inner_from),
                    end: Pos::new(node, inner_to),
                },
                mark,
                on: Presence::On,
            },
            Op::Delete {
                range: Range {
                    start: Pos::new(node, inner_to),
                    end: Pos::new(node, inner_to + close_len),
                },
            },
            Op::Delete {
                range: Range {
                    start: Pos::new(node, start),
                    end: Pos::new(node, start + open_len),
                },
            },
        ],
        caret: Pos::new(node, inner_len + start),
        closes: Some(mark),
    }
}

fn wrap_token<'a>(before: &'a str, token: &'a str, banned: char) -> Option<Wrapped<'a>> {
    if !before.ends_with(token) {
        return None;
    }
    let head = &before[..before.len() - token.len()];
    let open_at = head.rfind(token)?;
    let inner = &head[open_at + token.len()..];
    if inner.is_empty() || inner.contains(banned) {
        return None;
    }
    Some(Wrapped {
        open_at,
        open: token,
        inner,
        close: token,
    })
}

fn star_italic(before: &str) -> Option<Wrapped<'_>> {
    if !before.ends_with('*') {
        return None;
    }
    let head = &before[..before.len() - 1];
    let open_at = head.rfind('*')?;
    if open_at > 0 && before.as_bytes().get(open_at - 1) == Some(&b'*') {
        return None;
    }
    let inner = &head[open_at + 1..];
    if inner.is_empty() || inner.starts_with(char::is_whitespace) || inner.contains('*') {
        return None;
    }
    Some(Wrapped {
        open_at,
        open: "*",
        inner,
        close: "*",
    })
}

fn underscore_italic(before: &str) -> Option<Wrapped<'_>> {
    if !before.ends_with('_') {
        return None;
    }
    let head = &before[..before.len() - 1];
    let open_at = head.rfind('_')?;
    if open_at > 0 {
        let preceded_by_space = before[..open_at]
            .chars()
            .next_back()
            .is_some_and(char::is_whitespace);
        if !preceded_by_space {
            return None;
        }
    }
    let inner = &head[open_at + 1..];
    if inner.is_empty() || inner.contains('_') {
        return None;
    }
    Some(Wrapped {
        open_at,
        open: "_",
        inner,
        close: "_",
    })
}

fn grapheme_prefix(text: &str, graphemes: usize) -> Option<&str> {
    let byte = byte_at(text, graphemes)?;
    text.get(..byte)
}
