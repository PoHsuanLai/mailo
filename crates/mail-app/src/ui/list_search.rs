//! The list box, run through the search pipeline.
//!
//! A search is [`crate::search::rank_query`], the function Ctrl T and `mailo search` call, with
//! the window's label index and local midnight, over the accounts the Space shows. So `from:`,
//! a prefix, a phrase and `re:/…/` mean the same thing in all three places, and the rows come
//! in the order `rank` gives them. An empty box is the place's own list, with its own row keys.
//!
//! One exception, said on screen: a bare `re:/…/` narrows nothing, so it runs over the page the
//! place already has loaded ("pattern over this page") rather than over a ranked sample of all
//! mail that would look like a search and not be one.

use super::data::list_for;
use crate::search::{self, Highlight, Source, Term};
use crate::view::{Listing, Shell};
use chrono::{DateTime, Utc};
use mail_domain::{Filter, LabelId, Query, ThreadSummary};
use mail_store::{SqliteStore, Store};
use std::ops::Range;

/// What the list pane was asked for, taken out of the shell so a blocking thread can run it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Request {
    /// The box is empty: the place's own list.
    Place(Listing),
    /// The box has text in it.
    Search(Search),
}

/// A search, and what it needs from the shell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Search {
    pub input: String,
    /// `label:` resolves against these, as it does everywhere else in the window.
    pub labels: Vec<(String, LabelId)>,
    /// The tile or Space narrowing, applied to every candidate.
    pub scope: Option<Filter>,
    /// The place's own query, for a bare pattern. `None` while Drafts is selected.
    pub page: Option<Query>,
    pub limit: usize,
}

impl Request {
    /// What `shell` asks the list for, `limit` rows at most.
    pub(super) fn of(shell: &Shell, limit: u32) -> Self {
        if shell.search.trim().is_empty() {
            return Self::Place(shell.listing(limit));
        }
        let mut place = shell.clone();
        place.search.clear();
        let page = match place.listing(limit) {
            Listing::Threads(query) => Some(query),
            Listing::Drafts => None,
        };
        Self::Search(Search {
            input: shell.search.clone(),
            labels: shell.labels.clone(),
            scope: shell.account_filter(),
            page,
            limit: usize::try_from(limit).unwrap_or(usize::MAX),
        })
    }
}

/// How the list came to be what it is. The list bar says so when that is not obvious.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) enum Scope {
    /// Not a search: the place's list.
    #[default]
    Place,
    /// Ranked by the pipeline.
    Ranked,
    /// A bare pattern, over the rows the place had loaded.
    OverPage,
    /// The pattern did not compile. The `regex` crate's own message.
    Invalid(String),
}

/// What the rows mark, and what the bar says.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct Marking {
    pub highlight: Highlight,
    pub scope: Scope,
}

impl Marking {
    /// The note the list bar shows, when there is one.
    pub(super) fn note(&self) -> Option<String> {
        match &self.scope {
            Scope::Place | Scope::Ranked => None,
            Scope::OverPage => Some("pattern over this page".to_owned()),
            Scope::Invalid(why) => Some(why.clone()),
        }
    }
}

/// The rows, and how to mark them.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct Listed {
    pub threads: Vec<ThreadSummary>,
    pub marking: Marking,
}

/// Run `request`. Pure apart from the store reads.
pub(super) fn listed(store: &SqliteStore, request: Request, now: DateTime<Utc>) -> Listed {
    match request {
        Request::Place(listing) => Listed {
            threads: list_for(store, listing),
            marking: Marking::default(),
        },
        Request::Search(search) => searched(store, &search, now),
    }
}

fn searched(store: &SqliteStore, search: &Search, now: DateTime<Utc>) -> Listed {
    let label = crate::query::named(&search.labels);
    let highlight = match search::list_highlight(&search.input, &chrono::Local, &label) {
        Ok(highlight) => highlight,
        Err(why) => return invalid(why),
    };
    // Nothing but a pattern: no operator, no word, no phrase to narrow the candidates with.
    let bare = highlight.pattern.is_some()
        && search::extract(&search.input).is_ok_and(|extracted| extracted.rest.trim().is_empty());
    if bare && let Some(page) = &search.page {
        return over_page(store, page, highlight, now);
    }
    let scoped = Scoped {
        store,
        scope: search.scope.clone(),
    };
    let ranked = match search::rank_query(
        &search.input,
        &scoped,
        &search::Affinity::default(),
        &chrono::Local,
        &label,
        now,
    ) {
        Ok(ranked) => ranked,
        Err(why) => return invalid(why),
    };
    Listed {
        threads: ranked
            .hits
            .into_iter()
            .take(search.limit)
            .map(|(summary, _)| summary)
            .collect(),
        marking: Marking {
            highlight,
            scope: Scope::Ranked,
        },
    }
}

fn over_page(
    store: &SqliteStore,
    page: &Query,
    highlight: Highlight,
    now: DateTime<Utc>,
) -> Listed {
    let rows = store
        .threads(page, now)
        .map(|page| page.items)
        .unwrap_or_default();
    let threads = match &highlight.pattern {
        Some(pattern) => rows
            .into_iter()
            .filter(|summary| search::regex_hits(pattern, &summary.subject, &summary.snippet))
            .collect(),
        None => rows,
    };
    Listed {
        threads,
        marking: Marking {
            highlight,
            scope: Scope::OverPage,
        },
    }
}

fn invalid(why: String) -> Listed {
    Listed {
        threads: Vec::new(),
        marking: Marking {
            highlight: Highlight::default(),
            scope: Scope::Invalid(why),
        },
    }
}

/// The store, narrowed to the accounts on screen.
///
/// `rank_query` has no account in its signature, and should not: the terminal searches every
/// account. The window's tile and Space narrowing is applied here, to the one call that returns
/// threads, so it is the same `And(account, …)` that [`Shell::query`] builds.
struct Scoped<'a> {
    store: &'a SqliteStore,
    scope: Option<Filter>,
}

impl Source for Scoped<'_> {
    fn terms_with_prefix(&self, prefix: &str, limit: usize) -> Vec<Term> {
        <SqliteStore as Source>::terms_with_prefix(self.store, prefix, limit)
    }

    fn ranked(
        &self,
        filter: &Filter,
        limit: usize,
        now: DateTime<Utc>,
    ) -> Vec<(ThreadSummary, f64)> {
        let filter = match &self.scope {
            Some(scope) => Filter::And(vec![scope.clone(), filter.clone()]),
            None => filter.clone(),
        };
        <SqliteStore as Source>::ranked(self.store, &filter, limit, now)
    }
}

/// A row's subject and snippet, marked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RowHit {
    pub subject: Vec<Range<usize>>,
    /// The stored snippet cut around its first match, or the stored snippet when nothing in it
    /// matched (the subject did, or the body past the snippet).
    pub snippet: String,
    pub snippet_marks: Vec<Range<usize>>,
}

/// What `summary`'s row marks under `highlight`. `None` when nothing is being searched.
pub(super) fn row_hit(summary: &ThreadSummary, highlight: &Highlight) -> Option<RowHit> {
    if highlight.is_empty() {
        return None;
    }
    let subject = highlight.ranges(&summary.subject);
    let stored = &summary.snippet;
    let snippet = match highlight.ranges(stored).first() {
        Some(first) => search::snippet(stored, first.clone()),
        None => stored.clone(),
    };
    let snippet_marks = highlight.ranges(&snippet);
    Some(RowHit {
        subject,
        snippet,
        snippet_marks,
    })
}

#[cfg(test)]
#[path = "list_search_tests.rs"]
mod tests;
