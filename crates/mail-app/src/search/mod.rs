//! Search, from what was typed to the menu.
//!
//! `parse` splits operators from free words, `expand` completes the word being typed without
//! changing what [`Filter::Text`] means, and [`prepare`] is the two together. What a prepared
//! query is then asked for is the split Gmail and Apple Mail make: the matches in date order,
//! which is cheap and paginated ([`Prepared::listed`]), and a few top results ranked inside a
//! window of the newest matches ([`Prepared::top`]), so a common word costs no more to rank than
//! a rare one. `mailo search` and the list box call [`search_list`], Ctrl T calls [`run`], and
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

#[cfg(test)]
mod live;
#[cfg(test)]
mod superset;
#[cfg(test)]
mod support;

use chrono::{DateTime, TimeZone, Utc};
use mail_domain::{LabelId, ThreadSummary};

pub use expanding::{Expansion, expand, suggestions};
pub use find::{Find, Highlight, Step, find_highlight, list_highlight};
pub use fuzzy::{FuzzyHit, match_list};
pub use group::{ActionHit, Command, MailHit, PersonHit, Results, Top};
pub use highlight::{marks, snippet};
pub use parsing::{Parsed, parse};
pub use pipeline::{MENU, Prepared, STRIP, Searched, WINDOW, prepare, search_list};
pub use ranking::{Affinity, MAX_BONUS, SenderStats, WEIGHTS, rank};
pub use regex::{Extracted, compile, extract, hits as regex_hits};
pub use source::{Source, Term, first};

/// A query that has been parsed, expanded and ranked.
#[derive(Debug, Clone)]
pub struct RankedMail {
    pub parsed: Parsed,
    pub expansion: Expansion,
    pub hits: Vec<(ThreadSummary, f64)>,
}

/// Parse, expand, and rank the best `k` of the newest [`WINDOW`] matches of `input`.
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
    label: &dyn Fn(&str) -> Vec<LabelId>,
    k: usize,
    now: DateTime<Utc>,
) -> Result<RankedMail, String> {
    let prepared = prepare(input, source, zone, label)?;
    let hits = prepared.top(source, affinity, k, now);
    Ok(prepared.ranked(hits))
}

/// The menu: top hit, mail, people, actions.
///
/// Mail is the [`MENU`] best of the newest [`WINDOW`] matches. A query of operators alone has
/// nothing to rank, so its mail is the newest [`MENU`] matches, scored by the ranker's other
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
    let Ok(prepared) = prepare(input, source, &Utc, &|_| Vec::new()) else {
        return Results::default();
    };
    let hits = if prepared.has_text() {
        prepared.top(source, affinity, MENU, now)
    } else {
        let newest = prepared
            .listed(source, first(MENU), now)
            .into_iter()
            .map(|summary| (summary, 0.0))
            .collect();
        rank(newest, affinity, &prepared.parsed, now)
    };
    group::assemble(&prepared.ranked(hits), affinity, commands)
}
