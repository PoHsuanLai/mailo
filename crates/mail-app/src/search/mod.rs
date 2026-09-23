// The re-exports below are for the Ctrl T menu (E10), which is their first caller; only
// `rank_query` has one today. Remove this allow when the menu lands.
#![allow(unused_imports)]

//! Search, from what was typed to the menu.
//!
//! `parse` splits operators from free words, `expand` completes the word being typed without
//! changing what [`Filter::Text`] means, `rank` orders the candidates, and [`run`] groups them.
//! `mailo search` calls [`rank_query`], which is the same parse, expand and rank, so the terminal
//! and the window cannot drift on a text query.

#[path = "expand.rs"]
mod expanding;
mod fuzzy;
mod group;
mod highlight;
#[path = "parse.rs"]
mod parsing;
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
use mail_domain::{Filter, LabelId, ThreadSummary};

pub use expanding::{Expansion, expand, suggestions};
pub use fuzzy::{FuzzyHit, match_list};
pub use group::{ActionHit, Command, MailHit, PersonHit, Results, Top};
pub use highlight::{marks, snippet};
pub use parsing::{Parsed, parse};
pub use ranking::{Affinity, MAX_BONUS, SenderStats, WEIGHTS, rank};
pub use regex::{Extracted, compile, extract, hits as regex_hits};
pub use source::{Source, Term};

/// A query that has been parsed, expanded and ranked.
#[derive(Debug, Clone)]
pub struct RankedMail {
    pub parsed: Parsed,
    pub expansion: Expansion,
    pub hits: Vec<(ThreadSummary, f64)>,
}

/// Parse, expand and rank `input`.
///
/// `zone` and `label` are the CLI's: `before:` is the reader's midnight, `label:` is every
/// account's row of that name. [`run`] has neither in its signature, so the menu resolves dates
/// in UTC and an unknown `label:` as text — the parser's own fallback.
///
/// An invalid `re:/pattern/` is `Err` of the `regex` crate's message.
pub fn rank_query<Tz: TimeZone>(
    input: &str,
    source: &dyn Source,
    affinity: &Affinity,
    zone: &Tz,
    label: &dyn Fn(&str) -> Vec<LabelId>,
    now: DateTime<Utc>,
) -> Result<RankedMail, String> {
    let extracted = extract(input)?;
    if extracted.rest.trim().is_empty() && extracted.regex.is_none() {
        let parsed = parse("", zone, label);
        return Ok(RankedMail {
            expansion: expand(&parsed, source),
            parsed,
            hits: Vec::new(),
        });
    }
    // Trailing whitespace is significant: it means the last word is finished and must not be
    // prefix-expanded. Trim only the emptiness check above.
    let parsed = parse(&extracted.rest, zone, label);
    let expansion = expand(&parsed, source);
    let filter = if extracted.rest.trim().is_empty() {
        Filter::All
    } else {
        expansion.filter.clone()
    };
    let mut hits = rank(&filter, source, affinity, &parsed, now);
    if let Some(regex) = &extracted.regex {
        hits.retain(|(summary, _)| regex_hits(regex, &summary.subject, &summary.snippet));
    }
    Ok(RankedMail {
        parsed,
        expansion,
        hits,
    })
}

/// The menu: top hit, mail, people, actions.
///
/// Pure apart from the two [`Source`] calls. An invalid regex yields an empty menu rather than a
/// panic; the message itself is [`extract`]'s `Err`.
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
    let Ok(ranked) = rank_query(input, source, affinity, &Utc, &|_| Vec::new(), now) else {
        return Results::default();
    };
    group::assemble(&ranked, affinity, commands)
}
