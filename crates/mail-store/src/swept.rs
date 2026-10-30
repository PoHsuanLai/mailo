//! When a mailbox was last swept (migration 0053): what keeps the sweeps on their schedule.

use chrono::{DateTime, Utc};

/// When each scheduled sweep of one mailbox last finished. `None` for one that has not.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Swept {
    /// The last time every flag was brought up to date.
    pub flags: Option<DateTime<Utc>>,
    /// The last time what the server had deleted was found.
    pub expunges: Option<DateTime<Utc>>,
}
