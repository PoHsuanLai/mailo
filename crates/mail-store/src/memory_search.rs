//! Prefix terms and date-ordered ranking for the in-memory store.
//!
//! Terms come from the same `message_index` the SQLite store writes into `messages_fts`, counted
//! once per message — FTS5's `doc` column, not the number of times a word is repeated. Ranking
//! has no `bm25` here: every score is 0.0 and the order is `last_date`.

use std::collections::{BTreeMap, BTreeSet};

use mail_domain::{Message, ThreadSummary};

use crate::Term;
use crate::prefix::{is_cjk_bigram, prefix_range, term_has_prefix};
use crate::sql::message_index;
use crate::term::by_frequency;

pub(crate) fn terms_with_prefix<'a>(
    messages: impl Iterator<Item = &'a Message>,
    prefix: &str,
    limit: usize,
) -> Vec<Term> {
    // An empty prefix, or one that folds to nothing, is not a request for every term.
    if limit == 0 || prefix_range(prefix).is_none() {
        return Vec::new();
    }
    let mut counts: BTreeMap<String, u64> = BTreeMap::new();
    for message in messages {
        let indexed = message_index(
            &message.subject,
            &message.from,
            &message.to,
            &message.cc,
            message.body.text(),
        );
        let mut seen = BTreeSet::new();
        for token in indexed.split_whitespace() {
            if term_has_prefix(token, prefix) {
                seen.insert(token.to_owned());
            }
        }
        for token in seen {
            *counts.entry(token).or_default() += 1;
        }
    }
    let mut terms: Vec<Term> = counts
        .into_iter()
        .map(|(text, docs)| Term {
            cjk_bigram: is_cjk_bigram(&text),
            text,
            docs,
        })
        .collect();
    by_frequency(&mut terms);
    terms.truncate(limit);
    terms
}

/// Every score is 0.0. Newest `last_date` first. The sort is stable, so equal dates keep the
/// thread-id order the messages were visited in.
pub(crate) fn order_by_last_date(
    mut rows: Vec<ThreadSummary>,
    limit: usize,
) -> Vec<(ThreadSummary, f64)> {
    rows.sort_by_key(|summary| std::cmp::Reverse(summary.last_date));
    rows.truncate(limit);
    rows.into_iter().map(|summary| (summary, 0.0)).collect()
}
