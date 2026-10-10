//! History in the in-memory store. Kept in step with `sqlite/opened.rs`, which says what it
//! keeps; the parity tests in `tests/store/opened.rs` hold the two to each other.

use super::Inner;
use chrono::{DateTime, Utc};
use mail_domain::{ThreadId, ThreadSummary};

impl Inner {
    pub(super) fn record_opened(&mut self, thread: ThreadId, at: DateTime<Utc>) {
        if self.threads.contains_key(&thread) {
            let last = self.opened.entry(thread).or_insert(at);
            *last = (*last).max(at);
        }
        let cutoff = crate::opened::cutoff(at);
        self.opened.retain(|_, opened| *opened >= cutoff);
    }

    /// SQLite's `ORDER BY at DESC, thread`.
    pub(super) fn opened_in_order(&self) -> Vec<ThreadSummary> {
        let mut order: Vec<(&ThreadId, &DateTime<Utc>)> = self.opened.iter().collect();
        order.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
        order
            .into_iter()
            .filter_map(|(thread, _)| self.view(*thread).map(|(summary, _)| summary))
            .collect()
    }
}
