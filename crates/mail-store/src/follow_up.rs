//! The order the waiting list is read in, shared by both stores so they cannot disagree.

use mail_domain::{FollowUp, ThreadSummary};

/// Conversations with a reminder, soonest due first, ties by thread id. A summary with no
/// reminder is dropped: it is waiting on nothing.
pub(crate) fn in_due_order(
    summaries: impl IntoIterator<Item = ThreadSummary>,
) -> Vec<ThreadSummary> {
    let mut out: Vec<ThreadSummary> = summaries
        .into_iter()
        .filter(|summary| summary.follow_up != FollowUp::Inactive)
        .collect();
    out.sort_by_key(|summary| {
        let due = match summary.follow_up {
            FollowUp::Until { at, .. } | FollowUp::Returned { at, .. } => Some(at),
            FollowUp::Inactive => None,
        };
        (due, summary.id)
    });
    out
}
