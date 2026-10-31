//! What to mark on screen, and ⌘F's place in a thread.
//!
//! A [`Highlight`] is the typed words and phrases, plus at most one `re:/pattern/`. The list
//! box builds one and marks text with [`Highlight::ranges`], so a row and a parsed body cannot
//! disagree about what matched. ⌘F in a thread finds a phrase ([`super::within::find_in`]) and
//! keeps only its place here ([`Find`]); a pattern in its box is searched as the text it is.
//!
//! The words are what was typed, not the vocabulary they expanded to: typing `uidval` marks
//! `UIDVAL`, the part of `UIDVALIDITY` the person actually asked for. Every expansion starts
//! with the typed prefix, so marking the prefix marks the start of every expanded hit.

use crate::error::CoreError;
use std::ops::Range;

use chrono::TimeZone;

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
    label: &dyn mail_logic::query::Names,
) -> Result<Highlight, CoreError> {
    let extracted = extract(input)?;
    // Operators mark nothing whoever they name, so `is:vip` needs no list here.
    let parsed = parse(&extracted.rest, zone, label);
    let mut terms = parsed.query_words();
    terms.extend(parsed.phrases);
    Ok(Highlight {
        terms,
        pattern: extracted.regex,
    })
}

/// ⌘F, while it is open: what was typed, and which match is the current one.
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

    use super::{Find, Step, list_highlight};

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
            let highlight = list_highlight(input, &Utc, &mail_logic::query::unnamed)
                .unwrap_or_else(|why| panic!("{input:?}: {why}"));
            let ranges = highlight.ranges(text);
            assert_eq!(marked(text, &ranges), *want, "typed {input:?}");
        }
    }

    #[test]
    fn enter_wraps_both_ways() {
        let mut at = Find {
            query: "q".to_owned(),
            current: 0,
        };
        let mut seen = Vec::new();
        for _ in 0..4 {
            seen.push(at.at(3));
            at.step(Step::Next, 3);
        }
        assert_eq!(seen, [Some(0), Some(1), Some(2), Some(0)]);
        at.current = 0;
        at.step(Step::Previous, 3);
        assert_eq!(at.at(3), Some(2));
        assert_eq!(at.at(0), None, "no match to be at");
    }
}
