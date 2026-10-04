//! Text with `<mark>` nodes in it, built from byte ranges.
//!
//! Never from strings: a range comes from [`mail_core::search::Highlight::ranges`] and is cut out of
//! the text it was measured on, so a subject cannot smuggle markup into the page and a match
//! cannot be marked somewhere it did not occur. A range that is not on a char boundary, runs
//! past the end, or overlaps the one before is dropped rather than trusted.

use std::ops::Range;

/// A run of text, marked or not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Piece<'a> {
    Plain(&'a str),
    Marked(&'a str),
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
