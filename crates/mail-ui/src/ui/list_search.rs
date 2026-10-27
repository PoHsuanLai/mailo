//! The list box, as the client is asked it.
//!
//! A search is the client's ([`mail_client::query::Conversations::Searched`]), which runs
//! [`mail_client::search::search_list`], the function `mailo search` calls, with the window's label
//! index, over the accounts the Space shows. So `from:`, a prefix, a phrase and `re:/…/` mean the
//! same thing in the list, in the search bar's panel and in the terminal. The rows are the
//! matches in date order, paginated like a place; above them, a strip of the few top results
//! ranked within the newest matches. An empty box is the place's own list.
//!
//! One exception, said on screen: a bare `re:/…/` narrows nothing, so it runs over the page the
//! place already has loaded ("pattern over this page") rather than over a ranked sample of all
//! mail that would look like a search and not be one.

use crate::ui::view::{Listing, Shell};
use mail_client::query::{Conversations, Found, How};
use mail_client::search::{self, Highlight};
use mail_domain::{Filter, LabelId, Query, ThreadSummary};
use std::ops::Range;

/// What the list pane was asked for, taken out of the shell.
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
    /// The Space's narrowing, applied to every candidate.
    pub scope: Option<Filter>,
    /// The place's own query, for a bare pattern. `None` while Drafts is selected.
    pub page: Option<Query>,
    pub limit: u32,
}

impl Request {
    /// What the list asks for while `shell` is showing and `text` is the settled search, at
    /// most `limit` rows. The box itself may already hold more: the debounce decides when that
    /// is searched for, not this.
    pub(super) fn of(shell: &Shell, text: &str, limit: u32) -> Self {
        let mut place = shell.clone();
        place.search.clear();
        if text.trim().is_empty() {
            return Self::Place(place.listing(limit));
        }
        let page = match place.listing(limit) {
            Listing::Threads(query) | Listing::Inbox { query, .. } => Some(query),
            Listing::Drafts
            | Listing::Scheduled
            | Listing::Waiting { .. }
            | Listing::History { .. } => None,
        };
        Self::Search(Search {
            input: text.to_owned(),
            labels: shell.labels.clone(),
            scope: shell.account_filter(),
            page,
            limit,
        })
    }
}

/// How the list came to be what it is. The list bar says so when that is not obvious.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) enum Scope {
    /// Not a search: the place's list.
    #[default]
    Place,
    /// Searched: the matches in date order, under their top results.
    Searched,
    /// A bare pattern, over the rows the place had loaded. Not said: the marks show what it
    /// matched.
    OverPage,
    /// The pattern did not compile. The `regex` crate's own message.
    Invalid(String),
}

/// What the rows mark, and what the bar says.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct Marking {
    pub highlight: Highlight,
    pub scope: Scope,
    /// The line the store was asked, under [`Scope::Searched`]: what "Search … on the server"
    /// asks the server. Empty otherwise.
    pub line: String,
}

impl Marking {
    /// The note the list bar shows, when there is one.
    pub(super) fn note(&self) -> Option<String> {
        match &self.scope {
            Scope::Place | Scope::Searched | Scope::OverPage => None,
            Scope::Invalid(why) => Some(why.clone()),
        }
    }
}

/// The rows, the top results above them, and how to mark both.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct Listed {
    pub threads: Vec<ThreadSummary>,
    /// The "Top results" strip. Empty when nothing is searched for, when only operators were
    /// typed, and when it would repeat the first rows.
    pub top: Vec<ThreadSummary>,
    pub marking: Marking,
}

impl Request {
    /// What the place lists, when it is a query a conversation's summary can be held to; `None`
    /// for a search and for the lists a query does not make.
    pub(super) fn filter(&self) -> Option<Filter> {
        match self {
            Request::Place(Listing::Threads(query) | Listing::Inbox { query, .. }) => {
                Some(query.filter.clone())
            }
            _ => None,
        }
    }

    /// What the client is asked for this request.
    pub(super) fn asked(&self) -> Conversations {
        match self {
            Request::Place(listing) => match listing.clone() {
                Listing::Threads(query) => Conversations::Matching(query),
                Listing::Inbox { query, scope } => Conversations::WithReturned { query, scope },
                Listing::Waiting { scope } => Conversations::Waiting { scope },
                Listing::History { scope } => Conversations::Opened { scope },
                // Not conversations: the list draws them from elsewhere.
                Listing::Drafts | Listing::Scheduled => Conversations::None,
            },
            Request::Search(search) => Conversations::Searched(mail_client::query::Search {
                input: search.input.clone(),
                labels: search.labels.clone(),
                scope: search.scope.clone(),
                within: search.page.clone(),
                limit: search.limit,
            }),
        }
    }
}

impl From<&Found> for Listed {
    /// What the client found, as the list draws it.
    fn from(found: &Found) -> Self {
        let (scope, line) = match &found.how {
            How::Listed => (Scope::Place, String::new()),
            How::Searched { line } => (Scope::Searched, line.clone()),
            How::Within => (Scope::OverPage, String::new()),
            How::Invalid(why) => (Scope::Invalid(why.clone()), String::new()),
        };
        Listed {
            threads: found.threads.clone(),
            top: found.top.clone(),
            marking: Marking {
                highlight: found.highlight.clone(),
                scope,
                line,
            },
        }
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
