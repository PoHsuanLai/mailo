//! The expanded filter's threads cover the plain `Text` filter's threads.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use mail_domain::filter::search_tokens;
use mail_domain::*;
use mail_store::{MemoryStore, Store};
use proptest::prelude::*;

use super::source::{Source, Term};
use super::support::{self, at};
use super::{expand, parse};

const WORDS: &[&str] = &[
    "alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel", "india", "juliet",
    "kilo", "lima", "郵件",
];

struct Indexed<'a> {
    terms: Vec<Term>,
    store: &'a MemoryStore,
}

impl Source for Indexed<'_> {
    fn terms_with_prefix(&self, prefix: &str, limit: usize) -> Vec<Term> {
        let mut terms: Vec<Term> = self
            .terms
            .iter()
            .filter(|term| term.text.starts_with(prefix))
            .cloned()
            .collect();
        terms.sort_by(|a, b| b.docs.cmp(&a.docs).then(a.text.cmp(&b.text)));
        terms.truncate(limit);
        terms
    }

    fn listed(&self, filter: &Filter, page: PageReq, now: DateTime<Utc>) -> Vec<ThreadSummary> {
        <MemoryStore as Source>::listed(self.store, filter, page, now)
    }

    fn top(
        &self,
        filter: &Filter,
        k: usize,
        window: &[ThreadId],
        now: DateTime<Utc>,
    ) -> Vec<(ThreadSummary, f64)> {
        <MemoryStore as Source>::top(self.store, filter, k, window, now)
    }
}

fn vocabulary(store: &MemoryStore, now: DateTime<Utc>) -> Vec<Term> {
    let page = store
        .threads(
            &Query {
                filter: Filter::All,
                sort: Sort {
                    property: Property::Date,
                    dir: SortDir::Desc,
                },
                page: PageReq {
                    after: None,
                    limit: 1000,
                },
            },
            now,
        )
        .expect("list");
    let mut docs: BTreeMap<String, u64> = BTreeMap::new();
    for summary in page.items {
        let thread = store.thread(summary.id).expect("thread");
        let mut tokens = BTreeSet::new();
        for id in thread.messages {
            let message = store.message(id).expect("message");
            tokens.extend(search_tokens(&message.subject));
            if let Some(text) = message.body.text() {
                tokens.extend(search_tokens(text));
            }
            tokens.extend(search_tokens(&message.from.email));
        }
        for token in tokens {
            *docs.entry(token).or_default() += 1;
        }
    }
    docs.into_iter()
        .map(|(text, count)| Term {
            cjk_bigram: text.chars().count() == 2 && text.chars().all(|ch| !ch.is_ascii()),
            text,
            docs: count,
        })
        .collect()
}

fn matching(store: &MemoryStore, filter: &Filter, now: DateTime<Utc>) -> BTreeSet<ThreadId> {
    store
        .threads(
            &Query {
                filter: filter.clone(),
                sort: Sort {
                    property: Property::Date,
                    dir: SortDir::Desc,
                },
                page: PageReq {
                    after: None,
                    limit: 1000,
                },
            },
            now,
        )
        .expect("query")
        .items
        .into_iter()
        .map(|summary| summary.id)
        .collect()
}

fn spec() -> impl Strategy<Value = (Vec<usize>, Vec<usize>)> {
    (
        prop::collection::vec(0..WORDS.len(), 1..3),
        prop::collection::vec(0..WORDS.len(), 0..3),
    )
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

    #[test]
    fn expanded_ids_cover_the_plain_text_ids(
        rows in prop::collection::vec(spec(), 1..5),
        word in 0..WORDS.len(),
        prefix_len in 1usize..9,
    ) {
        let now = at(5_000);
        let store = MemoryStore::new();
        for (i, (subject, body)) in rows.iter().enumerate() {
            let subject = subject.iter().map(|n| WORDS[*n]).collect::<Vec<_>>().join(" ");
            let body = body.iter().map(|n| WORDS[*n]).collect::<Vec<_>>().join(" ");
            support::remember(&store, i as u128, &subject, &body, i as i64);
        }
        let indexed = Indexed {
            terms: vocabulary(&store, now),
            store: &store,
        };
        let full = WORDS[word];
        let take = prefix_len.clamp(1, full.chars().count());
        let query: String = full.chars().take(take).collect();
        let parsed = parse(&query, &Utc, &|_| Vec::new());
        let expansion = expand(&parsed, &indexed);
        let plain = Filter::Text(TextMatch::Contains(query.clone()));
        let expanded_ids = matching(&store, &expansion.filter, now);
        let plain_ids = matching(&store, &plain, now);
        prop_assert!(
            plain_ids.is_subset(&expanded_ids),
            "query {query:?}\nplain {plain_ids:?}\nexpanded {expanded_ids:?}\nfilter {:?}",
            expansion.filter
        );
    }
}
