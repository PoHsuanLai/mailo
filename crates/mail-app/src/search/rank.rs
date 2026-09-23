//! How a candidate outranks another one.
//!
//! The numbers in [`WEIGHTS`] are not a promise. Tests assert orderings: an unread recent thread
//! from someone you write to beats an old read one when bm25 is equal; a subject hit beats a
//! body-only hit; a large bm25 gap beats every bonus at once. [`MAX_BONUS`] is that last claim,
//! written down so the gap in the test is the gap in the table.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use mail_domain::ThreadSummary;
use mail_domain::filter::search_tokens;

use super::parsing::Parsed;

/// Thirty days, the recency half-life, in seconds.
const THIRTY_DAYS_SECS: f64 = 30.0 * 24.0 * 60.0 * 60.0;

/// How many of a sender's threads still move the score. Past this, one very chatty address
/// would outrank a genuinely better bm25.
const AFFINITY_THREAD_CAP: u32 = 20;

/// One weight per signal. See the module docs for which orderings the ratios protect.
#[derive(Debug, Clone, Copy)]
pub struct Weights {
    /// Multiplies the store's bm25. Unscaled, so a gap past [`MAX_BONUS`] wins on its own.
    pub bm25: f64,
    /// Multiplies `exp(−age / 30 days)`. The value is at most 1.
    pub recency: f64,
    pub unread: f64,
    pub starred: f64,
    /// Per thread with that sender, up to [`AFFINITY_THREAD_CAP`].
    pub per_thread: f64,
    /// Added once when you have written to that sender.
    pub replied: f64,
    /// Added when every query word occurs in the subject.
    pub subject: f64,
    /// Added when a quoted phrase occurs in the subject or the snippet.
    pub phrase: f64,
}

/// The one table. Change a relationship here, not at a call site.
pub const WEIGHTS: Weights = Weights {
    bm25: 1.0,
    recency: 1.0,
    unread: 0.8,
    starred: 0.4,
    per_thread: 0.03,
    replied: 0.6,
    subject: 1.2,
    phrase: 0.8,
};

/// The most the additives can contribute together, bm25 excluded.
pub const MAX_BONUS: f64 = WEIGHTS.recency
    + WEIGHTS.unread
    + WEIGHTS.starred
    + WEIGHTS.per_thread * AFFINITY_THREAD_CAP as f64
    + WEIGHTS.replied
    + WEIGHTS.subject
    + WEIGHTS.phrase;

/// How many threads you have had with an address, and whether you have written to them.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SenderStats {
    pub threads: u32,
    pub replied: bool,
}

/// Sender history keyed by address. Building it from the store is a later step; callers pass it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Affinity {
    by_email: HashMap<String, SenderStats>,
}

impl Affinity {
    /// Record `stats` for `email`. The key is matched ASCII-case-insensitively.
    pub fn insert(&mut self, email: impl AsRef<str>, stats: SenderStats) {
        self.by_email
            .insert(email.as_ref().to_ascii_lowercase(), stats);
    }

    /// The stats for `email`, if any were recorded.
    pub fn get(&self, email: &str) -> Option<&SenderStats> {
        self.by_email.get(&email.to_ascii_lowercase())
    }

    /// Every recorded address, in no particular order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &SenderStats)> {
        self.by_email
            .iter()
            .map(|(email, stats)| (email.as_str(), stats))
    }
}

/// Re-score `rows`, which arrive with the store's bm25. Highest score first; ties go to the
/// newer thread.
///
/// Pure: the candidates are whatever the caller fetched, which is what keeps this ranker's cost
/// bounded by the few rows a top-results strip holds rather than by how common a word is.
pub fn rank(
    rows: Vec<(ThreadSummary, f64)>,
    affinity: &Affinity,
    parsed: &Parsed,
    now: DateTime<Utc>,
) -> Vec<(ThreadSummary, f64)> {
    let words = parsed.query_words();
    let mut rows: Vec<(ThreadSummary, f64)> = rows
        .into_iter()
        .map(|(summary, bm25)| {
            let score = score(&summary, bm25, &words, &parsed.phrases, affinity, now);
            (summary, score)
        })
        .collect();
    rows.sort_by(|a, b| {
        b.1.total_cmp(&a.1)
            .then(b.0.last_date.cmp(&a.0.last_date))
            .then(a.0.id.cmp(&b.0.id))
    });
    rows
}

fn score(
    summary: &ThreadSummary,
    bm25: f64,
    words: &[String],
    phrases: &[String],
    affinity: &Affinity,
    now: DateTime<Utc>,
) -> f64 {
    let mut total = WEIGHTS.bm25 * bm25 + WEIGHTS.recency * recency(summary.last_date, now);
    if summary.read == mail_domain::ReadState::Unread {
        total += WEIGHTS.unread;
    }
    if summary.star == mail_domain::Star::Starred {
        total += WEIGHTS.starred;
    }
    total += affinity_score(affinity.get(&summary.from.email));
    if subject_hit(&summary.subject, words) {
        total += WEIGHTS.subject;
    }
    if phrase_hit(summary, phrases) {
        total += WEIGHTS.phrase;
    }
    total
}

fn recency(last: DateTime<Utc>, now: DateTime<Utc>) -> f64 {
    let age = (now - last).num_seconds();
    let age = if age < 0 { 0.0 } else { age as f64 };
    (-age / THIRTY_DAYS_SECS).exp()
}

fn affinity_score(stats: Option<&SenderStats>) -> f64 {
    let Some(stats) = stats else {
        return 0.0;
    };
    let threads = f64::from(stats.threads.min(AFFINITY_THREAD_CAP));
    let replied = if stats.replied { WEIGHTS.replied } else { 0.0 };
    threads * WEIGHTS.per_thread + replied
}

/// Every query word occurs, as a whole word, in the subject. An empty query is not a hit:
/// otherwise every thread would collect the bonus.
fn subject_hit(subject: &str, words: &[String]) -> bool {
    if words.is_empty() {
        return false;
    }
    let hay = search_tokens(subject);
    words.iter().all(|word| {
        let needle = search_tokens(word);
        !needle.is_empty()
            && needle
                .iter()
                .all(|token| hay.iter().any(|got| got == token))
    })
}

fn phrase_hit(summary: &ThreadSummary, phrases: &[String]) -> bool {
    phrases.iter().any(|phrase| {
        let needle = search_tokens(phrase);
        !needle.is_empty()
            && (field_has(&summary.subject, &needle) || field_has(&summary.snippet, &needle))
    })
}

fn field_has(field: &str, needle: &[String]) -> bool {
    let tokens = search_tokens(field);
    tokens.len() >= needle.len() && tokens.windows(needle.len()).any(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use mail_domain::{ReadState, Star, ThreadSummary};

    use super::super::parsing::parse;
    use super::super::support::{self, at};
    use super::{Affinity, MAX_BONUS, SenderStats, rank};

    fn ids(rows: &[(ThreadSummary, f64)]) -> Vec<u128> {
        rows.iter()
            .map(|(summary, _)| summary.id.as_uuid().as_u128())
            .collect()
    }

    fn words(text: &str) -> super::super::Parsed {
        parse(text, &Utc, &|_| Vec::new())
    }

    #[test]
    fn orderings() {
        let now = at(0);
        // Equal bm25. Unread, recent, from someone we write to, against old and read.
        let close = support::summary(
            1,
            "hello",
            "",
            "ada@b.c",
            0,
            ReadState::Unread,
            Star::Unstarred,
        );
        let distant = support::summary(
            2,
            "hello",
            "",
            "stranger@b.c",
            -400 * 24 * 60 * 60,
            ReadState::Read,
            Star::Unstarred,
        );
        let mut affinity = Affinity::default();
        affinity.insert(
            "ada@b.c",
            SenderStats {
                threads: 12,
                replied: true,
            },
        );
        let rows = vec![(distant.clone(), 1.0), (close.clone(), 1.0)];
        let ranked = rank(rows, &affinity, &words("hello"), now);
        assert_eq!(
            ids(&ranked),
            vec![1, 2],
            "unread recent correspondent first"
        );

        // Subject hit against a body-only hit. Same everything else.
        let subject = support::summary(
            3,
            "invoice due",
            "",
            "a@b.c",
            0,
            ReadState::Read,
            Star::Unstarred,
        );
        let body = support::summary(
            4,
            "hello",
            "invoice due",
            "a@b.c",
            0,
            ReadState::Read,
            Star::Unstarred,
        );
        let rows = vec![(body, 2.0), (subject, 2.0)];
        let ranked = rank(rows, &Affinity::default(), &words("invoice"), now);
        assert_eq!(ids(&ranked), vec![3, 4], "subject beats body");

        // Every bonus on one side, a bm25 gap larger than MAX_BONUS on the other.
        let bonuses = support::summary(
            5,
            "invoice pay now",
            "pay now",
            "ada@b.c",
            0,
            ReadState::Unread,
            Star::Starred,
        );
        let bm25 = support::summary(
            6,
            "unrelated",
            "",
            "other@b.c",
            -400 * 24 * 60 * 60,
            ReadState::Read,
            Star::Unstarred,
        );
        let mut affinity = Affinity::default();
        affinity.insert(
            "ada@b.c",
            SenderStats {
                threads: 10_000,
                replied: true,
            },
        );
        let mut parsed = words("invoice");
        parsed.phrases = vec!["pay now".to_owned()];
        let rows = vec![(bonuses, 0.0), (bm25, MAX_BONUS + 1.0)];
        let ranked = rank(rows, &affinity, &parsed, now);
        assert_eq!(ids(&ranked), vec![6, 5], "bm25 gap beats every bonus");
    }
}
