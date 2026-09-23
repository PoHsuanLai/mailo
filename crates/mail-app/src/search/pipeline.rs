//! From a typed line to what the store is asked for: a date-ordered page, and a few top hits.
//!
//! Ranking every match of a common word is what a keystroke cannot afford: the store measured
//! 62–188 ms to score `the` over 50,000 messages. So a search lists its matches in date order,
//! paginated like a place, and ranks only a small strip of top results chosen from a capped
//! window of the newest matches. The window bounds the scoring cost whatever the word.

use chrono::{DateTime, TimeZone, Utc};
use mail_domain::{Filter, LabelId, PageReq, ThreadSummary};

use super::RankedMail;
use super::expanding::{Expansion, expand};
use super::parsing::{Parsed, parse};
use super::ranking::{Affinity, rank};
use super::regex::{extract, hits as regex_hits};
use super::source::Source;

/// Top results the list and `mailo search` show above the date-ordered rows.
pub const STRIP: usize = 5;

/// Mail Ctrl T asks for: the top hit, and six rows under it.
pub const MENU: usize = 7;

/// How many of the newest matches top results are chosen from. A strong match older than this
/// is not a top result; the date-ordered list below still finds it.
pub const WINDOW: usize = 200;

/// A line parsed and expanded, ready to ask the store for.
#[derive(Debug, Clone)]
pub struct Prepared {
    pub parsed: Parsed,
    pub expansion: Expansion,
    /// What the store is asked for: the expansion's filter, or `All` under a bare pattern.
    pub filter: Filter,
    /// A `re:/…/`, applied to each row the store returns.
    pub regex: Option<regex::Regex>,
}

/// Parse and expand `input`. An invalid `re:/pattern/` is `Err` of the `regex` crate's message.
pub fn prepare<Tz: TimeZone>(
    input: &str,
    source: &dyn Source,
    zone: &Tz,
    label: &dyn Fn(&str) -> Vec<LabelId>,
) -> Result<Prepared, String> {
    let extracted = extract(input)?;
    // Trailing whitespace is significant: it means the last word is finished and must not be
    // prefix-expanded. Trim only the emptiness checks.
    let blank = extracted.rest.trim().is_empty();
    let parsed = if blank {
        parse("", zone, label)
    } else {
        parse(&extracted.rest, zone, label)
    };
    let expansion = expand(&parsed, source);
    let filter = if blank {
        Filter::All
    } else {
        expansion.filter.clone()
    };
    Ok(Prepared {
        parsed,
        expansion,
        filter,
        regex: extracted.regex,
    })
}

impl Prepared {
    /// Whether a word or a phrase was typed. Operators alone have nothing to rank by.
    pub fn has_text(&self) -> bool {
        !self.parsed.query_words().is_empty() || !self.parsed.phrases.is_empty()
    }

    /// Nothing at all was typed: no operator, no word, no pattern.
    fn is_blank(&self) -> bool {
        self.parsed.is_empty() && self.regex.is_none()
    }

    /// The best `k` of the newest [`WINDOW`] matches, re-scored by [`rank`], best first. Empty
    /// without free text.
    pub fn top(
        &self,
        source: &dyn Source,
        affinity: &Affinity,
        k: usize,
        now: DateTime<Utc>,
    ) -> Vec<(ThreadSummary, f64)> {
        if !self.has_text() {
            return Vec::new();
        }
        let mut rows = rank(
            source.top(&self.filter, k, WINDOW, now),
            affinity,
            &self.parsed,
            now,
        );
        if let Some(regex) = &self.regex {
            rows.retain(|(summary, _)| regex_hits(regex, &summary.subject, &summary.snippet));
        }
        rows
    }

    /// One page of the matches, newest first. Empty when nothing was typed.
    ///
    /// A pattern is applied to the page the store returned, so a page under `re:/…/` can hold
    /// fewer rows than it asked for.
    pub fn listed(
        &self,
        source: &dyn Source,
        page: PageReq,
        now: DateTime<Utc>,
    ) -> Vec<ThreadSummary> {
        if self.is_blank() {
            return Vec::new();
        }
        let mut rows = source.listed(&self.filter, page, now);
        if let Some(regex) = &self.regex {
            rows.retain(|summary| regex_hits(regex, &summary.subject, &summary.snippet));
        }
        rows
    }

    /// This query with `hits` as its ranked mail.
    pub(super) fn ranked(self, hits: Vec<(ThreadSummary, f64)>) -> RankedMail {
        RankedMail {
            parsed: self.parsed,
            expansion: self.expansion,
            hits,
        }
    }
}

/// A search as the list box and `mailo search` show it.
#[derive(Debug, Clone)]
pub struct Searched {
    pub prepared: Prepared,
    /// Up to [`STRIP`] top results, best first.
    pub top: Vec<(ThreadSummary, f64)>,
    /// The page of matches, newest first. A top result is in here too, where its date puts it.
    pub rows: Vec<ThreadSummary>,
}

impl Searched {
    /// The top results worth a strip of their own.
    ///
    /// None when nothing free was typed (operators only), and none when the strip would repeat
    /// the first rows of the list in the same order: then it tells the reader nothing.
    pub fn strip(&self) -> Vec<ThreadSummary> {
        if !self.prepared.has_text() || self.top.is_empty() {
            return Vec::new();
        }
        let repeats = self.top.len() <= self.rows.len()
            && self
                .top
                .iter()
                .zip(&self.rows)
                .all(|((top, _), row)| top.id == row.id);
        if repeats {
            return Vec::new();
        }
        self.top
            .iter()
            .map(|(summary, _)| summary.clone())
            .collect()
    }
}

/// Parse and expand `input`, then ask for one `page` of its matches and its [`STRIP`] top hits.
///
/// An invalid `re:/pattern/` is `Err` of the `regex` crate's message.
pub fn search_list<Tz: TimeZone>(
    input: &str,
    source: &dyn Source,
    affinity: &Affinity,
    zone: &Tz,
    label: &dyn Fn(&str) -> Vec<LabelId>,
    page: PageReq,
    now: DateTime<Utc>,
) -> Result<Searched, String> {
    let prepared = prepare(input, source, zone, label)?;
    let rows = prepared.listed(source, page, now);
    let top = prepared.top(source, affinity, STRIP, now);
    Ok(Searched {
        prepared,
        top,
        rows,
    })
}

#[cfg(test)]
#[path = "pipeline_tests.rs"]
mod tests;
