//! How long History is kept, shared by both stores so they cannot disagree.

use chrono::{DateTime, TimeDelta, Utc};

/// How long a conversation stays in History after it was last opened. Recording an open forgets
/// whatever is older, so nothing else has to sweep.
pub const OPENED_KEPT: TimeDelta = TimeDelta::days(90);

/// The instant before which an open recorded at `at` forgets the rest.
pub(crate) fn cutoff(at: DateTime<Utc>) -> DateTime<Utc> {
    at - OPENED_KEPT
}
