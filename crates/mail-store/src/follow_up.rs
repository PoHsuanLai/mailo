//! The order the waiting list is read in, shared by both stores so they cannot disagree.

use chrono::{DateTime, Utc};
use mail_domain::{DraftId, FollowUp, ThreadId, ThreadSummary};
use porter_core::AccountId;

/// A composer's reminder, waiting for its message to leave (migration 0027).
///
/// It belongs on a conversation only once the message is on its way, so it is kept here, by the
/// draft, until then.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FollowUpHold {
    pub draft: DraftId,
    pub account: AccountId,
    /// The `Message-ID` the message goes with, normalized.
    pub message_id: String,
    /// The conversation a reply answers.
    pub thread: Option<ThreadId>,
    pub at: DateTime<Utc>,
    /// When the message leaves.
    pub set: DateTime<Utc>,
}

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
