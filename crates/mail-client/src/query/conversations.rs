//! Conversations, by each way mail can be asked for: matching a query, searched for, waiting on
//! a reply, opened before.
//!
//! A search is [`mail_core::search::search_list`], the function `mailo search` calls, over the
//! accounts asked for. So `from:`, a prefix, a phrase and `re:/…/` mean the same thing to every
//! caller. A bare `re:/…/` narrows nothing, so it is run over the conversations of the query it
//! is asked within ([`How::Within`]) rather than over a ranked sample of all mail that would look
//! like a search and not be one.

use crate::{ClientError, Query as Asked, Topics};
use chrono::{DateTime, Utc};
use mail_core::search::{self, Highlight, Source, Term};
use mail_domain::{Filter, LabelId, MessageId, PageReq, Query, ThreadId, ThreadSummary};
use mail_store::{SqliteStore, Store};
use porter_core::AccountId;

/// Which conversations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Conversations {
    /// The ones `Query` matches, in its order and to its depth.
    Matching(Query),
    /// The messages `Query` matches, each a row of its own with the conversation it is of
    /// ([`mail_core::single::listed`]).
    Singles(Query),
    /// The ones `query` matches, under the conversations whose follow-up reminder came back
    /// with no reply, on the accounts `scope` narrows to.
    WithReturned { query: Query, scope: Option<Filter> },
    /// The ones waiting on a reply, on the accounts `scope` narrows to.
    Waiting { scope: Option<Filter> },
    /// Every one opened, newest first, on the accounts `scope` narrows to.
    Opened { scope: Option<Filter> },
    /// The ones a search line finds.
    Searched(Search),
    /// None: what no query can list (drafts are not conversations).
    None,
}

/// A search line, and what it is read against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Search {
    pub input: String,
    /// What `label:` resolves against.
    pub labels: Vec<(String, LabelId)>,
    /// What `account:` resolves against: each account's address.
    pub accounts: Vec<(String, AccountId)>,
    /// The accounts searched, as a filter; `None` is every account.
    pub scope: Option<Filter>,
    /// What a bare pattern is run within. `None` when there is nothing to run it within.
    pub within: Option<Query>,
    pub limit: u32,
}

/// The conversations found, and how.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Found {
    pub threads: Vec<ThreadSummary>,
    /// Which message each of [`Self::threads`] is, in the same order, when the rows are
    /// messages ([`Conversations::Singles`]). Empty when they are conversations.
    pub messages: Vec<MessageId>,
    /// For a search: the few best matches among the newest. Empty when nothing was searched
    /// for, when only operators were given, and when it would repeat the first conversations.
    pub top: Vec<ThreadSummary>,
    /// What a search's words, phrases and pattern match, to mark.
    pub highlight: Highlight,
    pub how: How,
}

/// How [`Found`] came to be.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum How {
    /// Listed: nothing was searched for.
    #[default]
    Listed,
    /// Searched for: the matches in date order, under the best of them. `line` is what the
    /// store was asked, which a server can be asked too.
    Searched { line: String },
    /// A bare pattern, run within the conversations of the query it was asked within.
    Within,
    /// The search could not be read: the reason, in the parser's words.
    Invalid(String),
}

impl Asked for Conversations {
    type Output = Found;

    /// The clock too: a snoozed conversation comes back, and a reminder comes due, as time
    /// passes. And everything a search reads: its labels, and the index of what was opened.
    const READS: Topics = Topics::SUMMARIES
        .union(Topics::LABELS)
        .union(Topics::FOLLOW_UPS)
        .union(Topics::CLOCK);

    fn run(&self, store: &SqliteStore, now: DateTime<Utc>) -> Result<Found, ClientError> {
        let listed = |threads| Found {
            threads,
            ..Found::default()
        };
        Ok(match self {
            Conversations::Matching(query) => listed(matching(store, query, now)?),
            Conversations::Singles(query) => {
                let (threads, messages) = mail_core::single::listed(store, query, now)?
                    .items
                    .into_iter()
                    .map(|single| (single.summary, single.message))
                    .unzip();
                Found {
                    threads,
                    messages,
                    ..Found::default()
                }
            }
            Conversations::WithReturned { query, scope } => listed(mail_core::follow_up::on_top(
                mail_core::follow_up::returned(store, scope.as_ref(), now),
                matching(store, query, now)?,
            )),
            Conversations::Waiting { scope } => {
                listed(mail_core::follow_up::waiting(store, scope.as_ref(), now))
            }
            Conversations::Opened { scope } => {
                listed(mail_core::history::listed(store, scope.as_ref(), now))
            }
            Conversations::Searched(search) => searched(store, search, now)?,
            Conversations::None => Found::default(),
        })
    }
}

fn matching(
    store: &SqliteStore,
    query: &Query,
    now: DateTime<Utc>,
) -> Result<Vec<ThreadSummary>, ClientError> {
    Ok(store.threads(query, now)?.items)
}

fn searched(
    store: &SqliteStore,
    search: &Search,
    now: DateTime<Utc>,
) -> Result<Found, ClientError> {
    let label = mail_core::query::Known {
        labels: &search.labels,
        accounts: &search.accounts,
    };
    let highlight = match search::list_highlight(&search.input, &chrono::Local, &label) {
        Ok(highlight) => highlight,
        Err(why) => return Ok(invalid(why.to_string())),
    };
    // Nothing but a pattern: no operator, no word, no phrase to narrow the candidates with.
    let bare = highlight.pattern.is_some()
        && search::extract(&search.input).is_ok_and(|extracted| extracted.rest.trim().is_empty());
    if bare && let Some(within) = &search.within {
        let threads = matching(store, within, now)?;
        let threads = match &highlight.pattern {
            Some(pattern) => threads
                .into_iter()
                .filter(|summary| search::regex_hits(pattern, &summary.subject, &summary.snippet))
                .collect(),
            None => threads,
        };
        return Ok(Found {
            threads,
            messages: Vec::new(),
            top: Vec::new(),
            highlight,
            how: How::Within,
        });
    }
    let scoped = Scoped {
        store,
        scope: search.scope.clone(),
    };
    let searched = match search::search_list(
        &search.input,
        &scoped,
        &search::Affinity::default(),
        &chrono::Local,
        &label,
        search::first(usize::try_from(search.limit).unwrap_or(usize::MAX)),
        now,
    ) {
        Ok(searched) => searched,
        Err(why) => return Ok(invalid(why.to_string())),
    };
    Ok(Found {
        top: searched.top_results(),
        threads: searched.rows,
        messages: Vec::new(),
        highlight,
        how: How::Searched {
            line: search.input.clone(),
        },
    })
}

fn invalid(why: String) -> Found {
    Found {
        how: How::Invalid(why),
        ..Found::default()
    }
}

/// The store, narrowed to the accounts asked for.
///
/// `search_list` has no account in its signature, and should not: the terminal searches every
/// account. The narrowing is applied here, to both calls that return conversations, so it is
/// the same `And(accounts, …)` a query over them is.
struct Scoped<'a> {
    store: &'a SqliteStore,
    scope: Option<Filter>,
}

impl Scoped<'_> {
    fn narrow(&self, filter: &Filter) -> Filter {
        match &self.scope {
            Some(scope) => Filter::And(vec![scope.clone(), filter.clone()]),
            None => filter.clone(),
        }
    }
}

impl Source for Scoped<'_> {
    fn terms_with_prefix(&self, prefix: &str, limit: usize) -> Vec<Term> {
        <SqliteStore as Source>::terms_with_prefix(self.store, prefix, limit)
    }

    fn listed(&self, filter: &Filter, page: PageReq, now: DateTime<Utc>) -> Vec<ThreadSummary> {
        <SqliteStore as Source>::listed(self.store, &self.narrow(filter), page, now)
    }

    fn top(
        &self,
        filter: &Filter,
        k: usize,
        window: &[ThreadId],
        now: DateTime<Utc>,
    ) -> Vec<(ThreadSummary, f64)> {
        <SqliteStore as Source>::top(self.store, &self.narrow(filter), k, window, now)
    }
}
