//! Text with `<mark>` nodes in it, built from byte ranges.
//!
//! Never from strings: a range comes from [`crate::search::Highlight::ranges`] and is cut out of
//! the text it was measured on, so a subject cannot smuggle markup into the page and a match
//! cannot be marked somewhere it did not occur. A range that is not on a char boundary, runs
//! past the end, or overlaps the one before is dropped rather than trusted.

use dioxus::prelude::*;
use std::ops::Range;

/// A run of text, marked or not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Piece<'a> {
    Plain(&'a str),
    Marked(&'a str),
}

/// How the marks in one run of text are numbered across the thread, for Ctrl F.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) struct Numbering {
    /// The number of this text's first mark.
    pub first: usize,
    /// The match Enter last moved to, when a find is open.
    pub current: Option<usize>,
}

/// `text` cut at `marks`. Every piece is a slice of `text`.
pub(super) fn pieces<'a>(text: &'a str, marks: &[Range<usize>]) -> Vec<Piece<'a>> {
    let mut out = Vec::new();
    let mut at = 0;
    for range in marks {
        let (Some(before), Some(inside)) = (text.get(at..range.start), text.get(range.clone()))
        else {
            continue;
        };
        if inside.is_empty() {
            continue;
        }
        if !before.is_empty() {
            out.push(Piece::Plain(before));
        }
        out.push(Piece::Marked(inside));
        at = range.end;
    }
    if at < text.len() || out.is_empty() {
        out.push(Piece::Plain(&text[at..]));
    }
    out
}

/// `text`, with `marks` drawn as `<mark>`. No marks is the bare text node it always was.
///
/// Each mark carries `data-hit`, its number across the thread, and the current one is
/// `mark.hit.now` — the element Enter scrolls into view.
pub(super) fn marked(text: &str, marks: &[Range<usize>], numbering: Numbering) -> Element {
    if marks.is_empty() {
        return rsx! { "{text}" };
    }
    let mut number = numbering.first;
    let drawn: Vec<(String, Option<(usize, bool)>)> = pieces(text, marks)
        .into_iter()
        .map(|piece| match piece {
            Piece::Plain(plain) => (plain.to_owned(), None),
            Piece::Marked(inside) => {
                let this = number;
                number += 1;
                (
                    inside.to_owned(),
                    Some((this, numbering.current == Some(this))),
                )
            }
        })
        .collect();
    rsx! {
        for (text, mark) in drawn {
            match mark {
                None => rsx! { "{text}" },
                Some((hit, now)) => rsx! {
                    mark { class: if now { "hit now" } else { "hit" }, "data-hit": "{hit}", "{text}" }
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Piece, pieces};

    #[test]
    fn marks_are_cut_on_char_boundaries_and_bad_ranges_are_dropped() {
        let cjk = "校園郵件通知";
        let at = cjk.find("郵件").expect("in the text");
        let emoji = "😀valid";
        let four = "😀".len();
        struct Case<'a> {
            name: &'a str,
            text: &'a str,
            /// `(start, end)` byte offsets.
            marks: Vec<(usize, usize)>,
            want: Vec<Piece<'a>>,
        }
        let cases = [
            Case {
                name: "cjk",
                text: cjk,
                marks: vec![(at, at + "郵件".len())],
                want: vec![
                    Piece::Plain("校園"),
                    Piece::Marked("郵件"),
                    Piece::Plain("通知"),
                ],
            },
            Case {
                name: "emoji before",
                text: emoji,
                marks: vec![(four, emoji.len())],
                want: vec![Piece::Plain("😀"), Piece::Marked("valid")],
            },
            Case {
                name: "inside a char",
                text: emoji,
                marks: vec![(1, 3)],
                want: vec![Piece::Plain(emoji)],
            },
            Case {
                name: "past the end",
                text: "abc",
                marks: vec![(1, 9)],
                want: vec![Piece::Plain("abc")],
            },
            Case {
                name: "overlap after the first",
                text: "abcdef",
                marks: vec![(0, 3), (2, 5)],
                want: vec![Piece::Marked("abc"), Piece::Plain("def")],
            },
            Case {
                name: "empty text",
                text: "",
                marks: vec![],
                want: vec![Piece::Plain("")],
            },
        ];
        for case in cases {
            let marks: Vec<_> = case.marks.iter().map(|(start, end)| *start..*end).collect();
            assert_eq!(pieces(case.text, &marks), case.want, "{}", case.name);
        }
    }
}
