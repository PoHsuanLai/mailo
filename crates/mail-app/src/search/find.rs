//! What to mark on screen, and Ctrl F's place in a thread.
//!
//! A [`Highlight`] is the typed words and phrases, plus at most one `re:/pattern/`. The list
//! box and Ctrl F both build one, and both mark text with [`Highlight::ranges`], so a row, a
//! parsed body and a find in a thread cannot disagree about what matched.
//!
//! The words are what was typed, not the vocabulary they expanded to: typing `uidval` marks
//! `UIDVAL`, the part of `UIDVALIDITY` the person actually asked for. Every expansion starts
//! with the typed prefix, so marking the prefix marks the start of every expanded hit.

use std::ops::Range;

use chrono::TimeZone;
use mail_domain::LabelId;

use super::highlight::marks;
use super::parsing::parse;
use super::regex::extract;

/// Words, phrases and a pattern to mark. Empty marks nothing.
#[derive(Debug, Clone, Default)]
pub struct Highlight {
    /// Matched ASCII-case-insensitively and through `search_tokens` folding.
    pub terms: Vec<String>,
    /// The `re:/…/` clause, when there was one.
    pub pattern: Option<regex::Regex>,
}

/// Two highlights are equal when they would mark the same text: the same terms, and the same
/// pattern source. `Regex` has no `PartialEq` of its own.
impl PartialEq for Highlight {
    fn eq(&self, other: &Self) -> bool {
        self.terms == other.terms
            && self.pattern.as_ref().map(regex::Regex::as_str)
                == other.pattern.as_ref().map(regex::Regex::as_str)
    }
}

impl Eq for Highlight {}

impl Highlight {
    /// Whether there is nothing to mark.
    pub fn is_empty(&self) -> bool {
        self.terms.is_empty() && self.pattern.is_none()
    }

    /// Byte ranges to mark in `text`: sorted, merged, on char boundaries.
    ///
    /// Empty matches of the pattern (`re:/x*/` matches everywhere) mark nothing.
    pub fn ranges(&self, text: &str) -> Vec<Range<usize>> {
        let mut found = marks(text, &self.terms);
        if let Some(pattern) = &self.pattern {
            found.extend(
                pattern
                    .find_iter(text)
                    .filter(|hit| hit.start() < hit.end())
                    .map(|hit| hit.range()),
            );
            found = merge(found);
        }
        found
    }
}

/// What the list box marks for `input`: its free words and phrases, and its pattern.
///
/// Operators mark nothing — `from:dana` is not a word in the subject. `label` resolves
/// `label:` the way the list does, so a label name is not marked as if it were text.
pub fn list_highlight<Tz: TimeZone>(
    input: &str,
    zone: &Tz,
    label: &dyn Fn(&str) -> Vec<LabelId>,
) -> Result<Highlight, String> {
    let extracted = extract(input)?;
    let parsed = parse(&extracted.rest, zone, label);
    let mut terms = parsed.query_words();
    terms.extend(parsed.phrases);
    Ok(Highlight {
        terms,
        pattern: extracted.regex,
    })
}

/// What Ctrl F looks for in a thread: whitespace-separated words, or one `re:/…/`.
///
/// No operators here, because a thread is already one conversation: `from:` in a find box is
/// the text `from:`. A pattern takes the whole box; words beside it are ignored rather than
/// half-applied. An invalid pattern is the `regex` crate's own message.
pub fn find_highlight(input: &str) -> Result<Highlight, String> {
    let extracted = extract(input)?;
    if let Some(pattern) = extracted.regex {
        return Ok(Highlight {
            terms: Vec::new(),
            pattern: Some(pattern),
        });
    }
    Ok(Highlight {
        terms: input.split_whitespace().map(str::to_owned).collect(),
        pattern: None,
    })
}

/// Ctrl F, while it is open: what was typed, and which match is the current one.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Find {
    pub query: String,
    /// Counted from zero, across the whole thread. Taken modulo the count wherever it is read,
    /// so it never has to be corrected when the count changes under it.
    pub current: usize,
}

/// Which way Enter moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Next,
    Previous,
}

impl Find {
    /// The current match, as an index below `total`. `None` when there is none.
    pub fn at(&self, total: usize) -> Option<usize> {
        (total > 0).then(|| self.current % total)
    }

    /// Move to the next or previous match, wrapping at either end.
    pub fn step(&mut self, step: Step, total: usize) {
        let Some(at) = self.at(total) else {
            self.current = 0;
            return;
        };
        self.current = match step {
            Step::Next => (at + 1) % total,
            Step::Previous => (at + total - 1) % total,
        };
    }

    /// "3 of 12", "no matches", or nothing while the box is empty.
    pub fn count(&self, total: usize) -> Option<String> {
        if self.query.trim().is_empty() {
            return None;
        }
        Some(match self.at(total) {
            Some(at) => format!("{} of {total}", at + 1),
            None => "no matches".to_owned(),
        })
    }
}

fn merge(mut ranges: Vec<Range<usize>>) -> Vec<Range<usize>> {
    ranges.sort_by_key(|range| (range.start, range.end));
    let mut out: Vec<Range<usize>> = Vec::new();
    for range in ranges {
        match out.last_mut() {
            Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
            _ => out.push(range),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::{Find, Step, find_highlight, list_highlight};

    fn marked<'a>(text: &'a str, ranges: &[std::ops::Range<usize>]) -> Vec<&'a str> {
        ranges.iter().map(|range| &text[range.clone()]).collect()
    }

    #[test]
    fn the_list_marks_what_was_typed_and_not_the_operators() {
        let cases: &[(&str, &str, &[&str])] = &[
            (
                "uidval",
                "Re: UIDL stability across a UIDVALIDITY change",
                &["UIDVAL"],
            ),
            ("from:dana uidl", "UIDL from dana", &["UIDL"]),
            (
                "\"sync review\"",
                "Notes from the sync review",
                &["sync review"],
            ),
            ("re:/[0-9]+/ cut", "0.9 cut on Thursday", &["0", "9", "cut"]),
            ("郵", "校園郵件通知", &["郵"]),
        ];
        for (input, text, want) in cases {
            let highlight = list_highlight(input, &Utc, &|_| Vec::new())
                .unwrap_or_else(|why| panic!("{input:?}: {why}"));
            let ranges = highlight.ranges(text);
            assert_eq!(marked(text, &ranges), *want, "typed {input:?}");
        }
    }

    #[test]
    fn find_takes_words_or_one_pattern() {
        let words = find_highlight("cursor list").expect("plain words");
        assert_eq!(words.terms, vec!["cursor".to_owned(), "list".to_owned()]);
        assert!(words.pattern.is_none());

        let pattern = find_highlight("re:/c[a-z]+r/ ignored").expect("a valid pattern");
        assert!(
            pattern.terms.is_empty(),
            "words beside a pattern are ignored"
        );
        let text = "the cursor and the cellar";
        assert_eq!(
            marked(text, &pattern.ranges(text)),
            vec!["cursor", "cellar"]
        );

        let broken = find_highlight("re:/(/").expect_err("an unclosed group");
        let own = regex::Regex::new(&String::from("("))
            .expect_err("the same pattern")
            .to_string();
        assert_eq!(broken, own, "the message is the regex crate's own");
    }

    #[test]
    fn an_empty_match_marks_nothing() {
        let pattern = find_highlight("re:/x*/").expect("valid");
        assert!(pattern.ranges("abc").is_empty());
    }

    #[test]
    fn enter_wraps_both_ways() {
        let mut at = Find {
            query: "q".to_owned(),
            current: 0,
        };
        let mut seen = Vec::new();
        for _ in 0..4 {
            seen.push(at.count(3).unwrap_or_default());
            at.step(Step::Next, 3);
        }
        assert_eq!(seen, ["1 of 3", "2 of 3", "3 of 3", "1 of 3"]);
        at.current = 0;
        at.step(Step::Previous, 3);
        assert_eq!(at.count(3).as_deref(), Some("3 of 3"));
        assert_eq!(at.count(0).as_deref(), Some("no matches"));
        at.query.clear();
        assert_eq!(at.count(3), None, "an empty box counts nothing");
    }
}
