//! What a pass running means for a spinner.
//!
//! The client's scheduler holds every account's link (`mail_client::fetch`); a spinner wants one
//! more thing, a token kept for as long as a pass runs, so the motion it draws runs unbroken
//! from the pass's start to its end.

use ds::motion::detail::operation::{Operation, PendingToken};
use mail_core::fetch::Link;

/// The operation to show once `link` is what it is: running only while a pass is, and under the
/// token it started with.
pub(super) fn operation(before: Operation, link: &Link) -> Operation {
    match (link.is_busy(), before) {
        (false, _) => Operation::Idle,
        (true, running @ Operation::Running(_)) => running,
        (true, Operation::Idle) => Operation::Running(PendingToken::start()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, TimeZone, Utc};
    use mail_core::fetch::{First, Live, Pause, Step};

    fn at(second: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 1, 12, 0, 0).unwrap() + chrono::TimeDelta::seconds(second)
    }

    fn current() -> Link {
        Link::Current {
            at: at(0),
            trouble: vec![],
            live: Live::Polling,
        }
    }

    #[test]
    fn the_spinner_runs_only_while_a_pass_does() {
        let token = Operation::Running(PendingToken::start());
        let syncing = Link::Syncing {
            first: First::No,
            step: Step::Flags,
            count: None,
            after: Box::new(current()),
        };
        let resting = [
            current(),
            Link::Fresh,
            Link::Waiting {
                until: at(5),
                why: Pause::Unreachable,
                failures: 1,
                first: First::No,
            },
            Link::NeedsSignIn {
                why: "x".into(),
                first: First::No,
            },
            Link::Broken {
                why: "x".into(),
                first: First::No,
            },
        ];
        assert_eq!(operation(token, &syncing), token, "it keeps its own token");
        assert!(
            matches!(operation(Operation::Idle, &syncing), Operation::Running(_)),
            "a pass that begins shows as running"
        );
        for link in resting {
            assert_eq!(operation(token, &link), Operation::Idle, "{link:?}");
        }
        assert_eq!(operation(Operation::Idle, &current()), Operation::Idle);
    }

    #[test]
    fn mail_with_no_stamp_is_a_fetched_account_current_as_of_now() {
        let (store, _dir) = crate::ui::fixtures::seeded();
        let link = mail_core::schedule::probe(&store, crate::ui::fixtures::acct_account(), at(0));
        assert_eq!(
            link,
            Link::Current {
                at: at(0),
                trouble: vec![],
                live: Live::Polling
            }
        );
    }
}
