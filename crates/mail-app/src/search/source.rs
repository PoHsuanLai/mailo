//! The store calls the pipeline needs, behind a trait the tests can fake.
//!
//! The real answers are `Store::terms_with_prefix` and `Store::search_ranked`. [`Source`] is
//! the seam so a test can assert against terms and scores it chose, which a real index would
//! not give it on demand.

use chrono::{DateTime, Utc};
use mail_domain::{Filter, ThreadSummary};
use mail_store::Store;

/// One entry in the full-text vocabulary, as the store reports it.
pub use mail_store::Term;

/// Where prefix expansion and candidate retrieval come from.
///
/// Two implementations are swapped here: the real store, and a fake the tests build with the
/// terms and scores they want to assert about. The pipeline takes `&dyn Source` for that reason.
pub trait Source {
    /// Vocabulary entries whose text starts with `prefix`, most frequent first is the caller's
    /// job. `prefix` is already folded with `search_tokens`.
    fn terms_with_prefix(&self, prefix: &str, limit: usize) -> Vec<Term>;

    /// Threads matching `filter`, with a bm25 score where higher is better.
    ///
    /// `0.0` when the filter has no text. The real store negates SQLite's bm25, which is
    /// negative and lower-better; this method keeps the sign the ranker adds up.
    fn ranked(
        &self,
        filter: &Filter,
        limit: usize,
        now: DateTime<Utc>,
    ) -> Vec<(ThreadSummary, f64)>;
}

impl<S: Store + ?Sized> Source for S {
    fn terms_with_prefix(&self, prefix: &str, limit: usize) -> Vec<Term> {
        // Every account's vocabulary, on purpose: a term that lives on another account still
        // has to pass the account filter on the results. A store error is an empty suggestion
        // list, and the word falls back to matching as typed.
        Store::terms_with_prefix(self, prefix, limit).unwrap_or_default()
    }

    fn ranked(
        &self,
        filter: &Filter,
        limit: usize,
        now: DateTime<Utc>,
    ) -> Vec<(ThreadSummary, f64)> {
        // A search box that panics because the store missed a row is worse than an empty
        // list. The error is the store's to log; here there is nothing to rank.
        Store::search_ranked(self, filter, limit, now).unwrap_or_default()
    }
}
