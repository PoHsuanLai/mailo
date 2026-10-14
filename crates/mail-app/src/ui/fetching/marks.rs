//! What the sync button shows of the links behind it: a pure mapping from fetch state to the
//! availability quire draws, so it can be tested with a table.

use ds::prelude::Availability;
use mail_core::fetch::Link;

/// The sync button's availability: busy while any account in view is fetching.
pub fn sync_availability(links: &[&Link]) -> Availability {
    if links.iter().any(|link| link.is_busy()) {
        Availability::Busy
    } else {
        Availability::Enabled
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use mail_core::fetch::{First, Live, Step};

    #[test]
    fn the_sync_button_is_busy_while_any_link_runs() {
        let idle = Link::Current {
            at: Utc.with_ymd_and_hms(2026, 10, 1, 12, 30, 0).unwrap(),
            live: Live::Polling,
            trouble: vec![],
        };
        let busy = Link::Syncing {
            first: First::No,
            step: Step::Connecting,
            count: None,
            after: Box::new(Link::Fresh),
        };
        assert_eq!(sync_availability(&[]), Availability::Enabled);
        assert_eq!(sync_availability(&[&idle]), Availability::Enabled);
        assert_eq!(sync_availability(&[&idle, &busy]), Availability::Busy);
    }
}
