//! ⌘F's matches in one message, as the message is drawn: runs of text in reading order.
//!
//! A message's frame holds its words as text nodes, and an inline element (`<b>`, a link) cuts a
//! word into several. [`find_in`] reads the runs of one block as one text, so `wor` in a `<b>`
//! and `ld` after it are the match `world`, and says each match as the pieces of the runs it
//! covers, which is what the frame marks.
//!
//! What was typed is a phrase, as a browser's find takes it, matched without regard to case or
//! accents (`resume` finds `Résumé`, through the same folding search uses, `search_tokens`), with
//! any run of white space matching any other: a line the sender wrapped still holds the phrase.
//! Matches do not overlap: `aa` is found twice in `aaaa`, as a browser counts it.

use std::ops::Range;

use mail_domain::filter::search_tokens;

/// The most matches one message is marked with: past this a find is not finding anything.
pub const MOST_FOUND: usize = 1000;

/// One match: each run it covers, by index, and the bytes of that run it covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found(pub Vec<(usize, Range<usize>)>);

/// Every match of `query` in `runs`, in order, at most [`MOST_FOUND`]. Nothing for a query of
/// nothing but white space.
pub fn find_in(runs: &[&str], query: &str) -> Vec<Found> {
    let wanted: Vec<char> = folded(std::iter::once((0, query)))
        .into_iter()
        .map(|(ch, _)| ch)
        .collect();
    if wanted.iter().all(|ch| *ch == ' ') {
        return Vec::new();
    }
    let (text, origin): (Vec<char>, Vec<Origin>) =
        folded(runs.iter().copied().enumerate()).into_iter().unzip();
    let mut found = Vec::new();
    let mut at = 0;
    while at + wanted.len() <= text.len() && found.len() < MOST_FOUND {
        if text[at..at + wanted.len()] == wanted[..] {
            found.push(pieces(&origin[at..at + wanted.len()]));
            at += wanted.len();
        } else {
            at += 1;
        }
    }
    found
}

/// Where a folded char came from: its run, and its bytes there.
type Origin = (usize, Range<usize>);

/// The chars of `runs` as they are compared, each with where it came from: folded, a run of
/// white space (across runs too) one space, and what draws nothing part of the char before it,
/// so an accent written apart from its letter is marked with it.
fn folded<'a>(runs: impl Iterator<Item = (usize, &'a str)>) -> Vec<(char, Origin)> {
    let mut out: Vec<(char, Origin)> = Vec::new();
    for (run, text) in runs {
        for (byte, ch) in text.char_indices() {
            let end = byte + ch.len_utf8();
            match fold(ch) {
                None => {
                    if let Some((_, (last, bytes))) = out.last_mut()
                        && *last == run
                    {
                        bytes.end = end;
                    }
                }
                Some(' ') if out.last().is_some_and(|(last, _)| *last == ' ') => {}
                Some(folded) => out.push((folded, (run, byte..end))),
            }
        }
    }
    out
}

/// `ch` as it is compared: white space a space, a letter lowercase and without its accent, and
/// nothing for what is drawn as nothing (a combining accent, a soft hyphen, a zero-width space).
fn fold(ch: char) -> Option<char> {
    if ch.is_whitespace() {
        return Some(' ');
    }
    if ch.is_ascii() {
        return Some(ch.to_ascii_lowercase());
    }
    if invisible(ch) {
        return None;
    }
    // One char's tokens are that char folded, or nothing for punctuation, which is kept as it is.
    let token = search_tokens(ch.encode_utf8(&mut [0; 4]));
    token
        .first()
        .and_then(|token| token.chars().next())
        .or_else(|| ch.to_lowercase().next())
}

/// Whether `ch` draws nothing of its own: a combining mark, which an accent written apart from
/// its letter is, or a character a sender puts between letters to steer breaking.
fn invisible(ch: char) -> bool {
    matches!(
        ch,
        '\u{0300}'..='\u{036f}'
            | '\u{1ab0}'..='\u{1aff}'
            | '\u{1dc0}'..='\u{1dff}'
            | '\u{20d0}'..='\u{20ff}'
            | '\u{fe20}'..='\u{fe2f}'
            | '\u{ad}'
            | '\u{200b}'..='\u{200d}'
            | '\u{2060}'
            | '\u{feff}'
    )
}

/// A match's chars as the pieces of the runs they came from, one per run.
fn pieces(origin: &[Origin]) -> Found {
    let mut out: Vec<(usize, Range<usize>)> = Vec::new();
    for (run, bytes) in origin {
        match out.last_mut() {
            Some((last, range)) if last == run => range.end = bytes.end,
            _ => out.push((*run, bytes.clone())),
        }
    }
    Found(out)
}

#[cfg(test)]
mod tests {
    use super::{Found, MOST_FOUND, find_in};

    /// Each match as the text of its pieces.
    fn said(runs: &[&str], found: &[Found]) -> Vec<Vec<String>> {
        found
            .iter()
            .map(|Found(pieces)| {
                pieces
                    .iter()
                    .map(|(run, bytes)| runs[*run][bytes.clone()].to_owned())
                    .collect()
            })
            .collect()
    }

    #[test]
    fn a_phrase_is_found_whatever_its_case_accents_spacing_or_inline_elements() {
        struct Case {
            name: &'static str,
            runs: &'static [&'static str],
            query: &'static str,
            want: &'static [&'static [&'static str]],
        }
        let cases = [
            Case {
                name: "case",
                runs: &["The Cursor moved; the CURSOR stayed."],
                query: "cursor",
                want: &[&["Cursor"], &["CURSOR"]],
            },
            Case {
                name: "accents in the text",
                runs: &["Attached: my Résumé and a résumé."],
                query: "resume",
                want: &[&["Résumé"], &["résumé"]],
            },
            Case {
                name: "accents typed",
                runs: &["Café at noon"],
                query: "CAFÉ",
                want: &[&["Café"]],
            },
            Case {
                name: "an accent written apart from its letter",
                runs: &["Cafe\u{301} at noon"],
                query: "café",
                want: &[&["Cafe\u{301}"]],
            },
            Case {
                name: "overlapping matches are counted as a browser counts them",
                runs: &["aaaa aaa"],
                query: "aa",
                want: &[&["aa"], &["aa"], &["aa"]],
            },
            Case {
                name: "across inline elements",
                runs: &["Hello wo", "rl", "d, world"],
                query: "world",
                want: &[&["wo", "rl", "d"], &["world"]],
            },
            Case {
                name: "white space of any kind and length",
                runs: &["see the\n   quarterly ", " report"],
                query: "the quarterly report",
                want: &[&["the\n   quarterly ", "report"]],
            },
            Case {
                name: "a soft hyphen inside a word",
                runs: &["an extra\u{ad}ordinary day"],
                query: "extraordinary",
                want: &[&["extra\u{ad}ordinary"]],
            },
            Case {
                name: "punctuation is itself",
                runs: &["v1.2 or v1-2"],
                query: "v1.2",
                want: &[&["v1.2"]],
            },
            Case {
                name: "CJK",
                runs: &["校園郵件通知，郵件"],
                query: "郵件",
                want: &[&["郵件"], &["郵件"]],
            },
            Case {
                name: "nothing typed",
                runs: &["text"],
                query: "  ",
                want: &[],
            },
            Case {
                name: "not there",
                runs: &["text"],
                query: "texts",
                want: &[],
            },
        ];
        for case in cases {
            let found = find_in(case.runs, case.query);
            let want: Vec<Vec<String>> = case
                .want
                .iter()
                .map(|pieces| pieces.iter().map(|piece| (*piece).to_owned()).collect())
                .collect();
            assert_eq!(said(case.runs, &found), want, "{}", case.name);
        }
    }

    #[test]
    fn a_match_names_the_runs_it_covers_and_their_bytes() {
        let found = find_in(&["Hé", "llo"], "éll");
        assert_eq!(found, vec![Found(vec![(0, 1..3), (1, 0..2)])]);
    }

    #[test]
    fn a_find_stops_at_the_most_it_marks() {
        let text = "a ".repeat(MOST_FOUND + 50);
        assert_eq!(find_in(&[&text], "a").len(), MOST_FOUND);
    }

    /// ⌘F finds a phrase: a pattern's notation is the text it is, and two words are one phrase.
    #[test]
    fn a_pattern_in_the_box_is_searched_as_the_text_it_is() {
        assert_eq!(find_in(&["a re:/a/ b"], "re:/a/").len(), 1);
        assert!(find_in(&["a cat"], "re:/a/").is_empty());
        assert_eq!(find_in(&["one two", "two one"], "one two").len(), 1);
    }
}
