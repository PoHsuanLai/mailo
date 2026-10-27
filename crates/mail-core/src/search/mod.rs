//! Search, from what was typed to the menu.
//!
//! `parse` splits operators from free words, `expand` completes the word being typed without
//! changing what [`Filter::Text`] means, and [`prepare`] is the two together. What a prepared
//! query is then asked for is the split Gmail and Apple Mail make: the matches in date order,
//! which is cheap and paginated ([`Prepared::listed`]), and a few top results ranked inside a
//! window of the newest matches ([`Prepared::top`]), so a common word costs no more to rank than
//! a rare one. `mailo search` and the list box call [`search_list`], ⌘K calls [`run`], and
//! all three go through [`prepare`], so the terminal and the window cannot drift on a query.

#[path = "expand.rs"]
mod expanding;
mod find;
mod fuzzy;
mod group;
mod highlight;
#[path = "parse.rs"]
mod parsing;
mod pipeline;
#[path = "rank.rs"]
mod ranking;
mod regex;
mod source;
mod within;

#[cfg(test)]
mod live;
#[cfg(test)]
mod superset;

use crate::error::CoreError;
use chrono::{DateTime, TimeZone, Utc};
use mail_domain::{ThreadId, ThreadSummary};

pub use expanding::{Expansion, expand, suggestions};
pub use find::{Find, Highlight, Step, list_highlight};
pub use fuzzy::{FuzzyHit, match_list};
pub use group::{ActionHit, Command, MailHit, PersonHit, Results, Top};
pub use highlight::{marks, snippet};
pub use parsing::{Parsed, parse};
pub use pipeline::{NEWEST_POOL, Prepared, QUICK_LIMIT, Searched, TOP_LIMIT, prepare, search_list};
pub use ranking::{Affinity, MAX_BONUS, SenderStats, WEIGHTS, rank};
pub use regex::{Extracted, compile, extract, hits as regex_hits};
pub use source::{Source, Term, first};
pub use within::{Found, MOST_FOUND, find_in};

/// A query that has been parsed, expanded and ranked.
#[derive(Debug, Clone)]
pub struct RankedMail {
    pub parsed: Parsed,
    pub expansion: Expansion,
    pub hits: Vec<(ThreadSummary, f64)>,
}

/// The newest [`NEWEST_POOL`] matches of `prepared`: the window its top hits are chosen from.
fn newest(prepared: &Prepared, source: &dyn Source, now: DateTime<Utc>) -> Vec<ThreadId> {
    prepared
        .listed(source, first(NEWEST_POOL), now)
        .iter()
        .map(|summary| summary.id)
        .collect()
}

/// Parse, expand, and rank the best `k` of the newest [`NEWEST_POOL`] matches of `input`.
///
/// `zone` and `label` are the CLI's: `before:` is the reader's midnight, `label:` is every
/// account's row of that name. [`run`] has neither in its signature, so the menu resolves dates
/// in UTC and an unknown `label:` as text — the parser's own fallback.
///
/// Operators alone rank nothing: `hits` is empty, and the date-ordered list is the answer.
/// An invalid `re:/pattern/` is `Err` of the `regex` crate's message.
pub fn rank_query<Tz: TimeZone>(
    input: &str,
    source: &dyn Source,
    affinity: &Affinity,
    zone: &Tz,
    label: &dyn crate::query::Names,
    k: usize,
    now: DateTime<Utc>,
) -> Result<RankedMail, CoreError> {
    let prepared = prepare(input, source, zone, label)?;
    let hits = prepared.top(source, affinity, k, &newest(&prepared, source, now), now);
    Ok(prepared.ranked(hits))
}

/// The menu: top hit, mail, people, actions.
///
/// Mail is the [`QUICK_LIMIT`] best of the newest [`NEWEST_POOL`] matches. A query of operators alone has
/// nothing to rank, so its mail is the newest [`QUICK_LIMIT`] matches, scored by the ranker's other
/// signals. Pure apart from the [`Source`] calls. An invalid regex yields an empty menu rather
/// than a panic; the message itself is [`extract`]'s `Err`.
pub fn run(
    input: &str,
    source: &dyn Source,
    affinity: &Affinity,
    commands: &[Command],
    now: DateTime<Utc>,
) -> Results {
    if input.trim().is_empty() {
        return group::empty_query(source, commands, now);
    }
    let Ok(prepared) = prepare(input, source, &Utc, &crate::query::unnamed) else {
        return Results::default();
    };
    let hits = if prepared.has_text() {
        prepared.top(
            source,
            affinity,
            QUICK_LIMIT,
            &newest(&prepared, source, now),
            now,
        )
    } else {
        let newest = prepared
            .listed(source, first(QUICK_LIMIT), now)
            .into_iter()
            .map(|summary| (summary, 0.0))
            .collect();
        rank(newest, affinity, &prepared.parsed, now)
    };
    group::assemble(&prepared.ranked(hits), affinity, commands)
}
