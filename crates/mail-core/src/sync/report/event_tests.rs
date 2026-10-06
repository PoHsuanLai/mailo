use super::{AccountReport, Counts, PassEnd, Progress, Trouble as Found, outcome};
use crate::fetch::{Count, Effect, Event, First, Link, Pause, Step, Trouble, step};
use mail_domain::Retry;
use mail_domain::id::account_id_from_uuid;
use porter_core::AccountId;
use std::time::Duration;

fn acct_account() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000f2"))
}
fn acct_other() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000f3"))
}

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn finished(trouble: Vec<Found>) -> PassEnd {
    PassEnd::Finished(AccountReport {
        account: acct_account(),
        address: "ada@example.test".to_owned(),
        counts: Counts::default(),
        trouble,
    })
}

fn found(mailbox: Option<&str>, retry: Retry, why: Option<&str>) -> Found {
    Found {
        mailbox: mailbox.map(str::to_owned),
        retry,
        why: why.map(str::to_owned),
    }
}

#[test]
fn each_step_of_a_pass_is_a_step_of_the_link() {
    let cases: Vec<(&str, Progress, Event)> = vec![
        (
            "connecting",
            Progress::Connecting,
            Event::Stepped(Step::Connecting, None),
        ),
        (
            "folders",
            Progress::Folders,
            Event::Stepped(Step::Folders, None),
        ),
        (
            "headers with a total",
            Progress::Headers {
                mailbox: "INBOX".into(),
                done: 12,
                of: Some(340),
            },
            Event::Stepped(
                Step::Headers {
                    mailbox: "INBOX".into(),
                },
                Some(Count { done: 12, of: 340 }),
            ),
        ),
        (
            "headers the server gave no total for",
            Progress::Headers {
                mailbox: "INBOX".into(),
                done: 12,
                of: None,
            },
            Event::Stepped(
                Step::Headers {
                    mailbox: "INBOX".into(),
                },
                None,
            ),
        ),
        (
            "flags drop the mailbox the link has no use for",
            Progress::Flags {
                mailbox: "INBOX".into(),
            },
            Event::Stepped(Step::Flags, None),
        ),
        (
            "bodies with a total",
            Progress::Bodies {
                done: 0,
                of: Some(7),
            },
            Event::Stepped(Step::Bodies, Some(Count { done: 0, of: 7 })),
        ),
        (
            "bodies without",
            Progress::Bodies { done: 3, of: None },
            Event::Stepped(Step::Bodies, None),
        ),
        (
            "sending",
            Progress::Sending,
            Event::Stepped(Step::Sending, None),
        ),
    ];
    for (name, progress, expected) in cases {
        assert_eq!(progress.event(), expected, "{name}");
    }
}

#[test]
fn how_a_pass_ended_is_what_the_link_hears() {
    let cases: Vec<(&str, PassEnd, Event)> = vec![
        (
            "clean",
            finished(vec![]),
            Event::Finished { trouble: vec![] },
        ),
        (
            "a folder that failed is kept, in words",
            finished(vec![found(
                Some("Archive"),
                Retry::Fatal("cannot select".into()),
                Some("Archive: cannot select"),
            )]),
            Event::Finished {
                trouble: vec![Trouble {
                    mailbox: Some("Archive".into()),
                    retry: Retry::Fatal("cannot select".into()),
                    why: "Archive: cannot select".into(),
                }],
            },
        ),
        (
            "a refusal the pass did not word is worded for it",
            finished(vec![found(None, Retry::Now, None)]),
            Event::Finished {
                trouble: vec![Trouble {
                    mailbox: None,
                    retry: Retry::Now,
                    why: "The connection dropped".into(),
                }],
            },
        ),
        (
            "a sign-in refused partway is a refused sign-in, not an update",
            finished(vec![
                found(Some("INBOX"), Retry::After(secs(9)), Some("slow")),
                found(None, Retry::NeedsReauth, Some("rules: refused")),
            ]),
            Event::Failed {
                retry: Retry::NeedsReauth,
                why: "rules: refused".into(),
                pause: Pause::ServerBusy,
            },
        ),
        (
            "a wait the server named is the longest of them",
            finished(vec![
                found(None, Retry::After(secs(60)), None),
                found(None, Retry::After(secs(3600)), None),
            ]),
            Event::Failed {
                retry: Retry::After(secs(3600)),
                why: "The server asked to be left alone for a while".into(),
                pause: Pause::Throttled,
            },
        ),
        (
            "a failure to run keeps its decision and its kind of wait",
            PassEnd::Failed {
                account: acct_account(),
                address: "a".into(),
                retry: Retry::After(secs(5)),
                why: "cannot connect".into(),
                pause: Pause::Unreachable,
            },
            Event::Failed {
                retry: Retry::After(secs(5)),
                why: "cannot connect".into(),
                pause: Pause::Unreachable,
            },
        ),
        (
            "cancelled",
            PassEnd::Cancelled {
                account: acct_account(),
                address: "a".into(),
            },
            Event::Cancel,
        ),
    ];
    for (name, end, expected) in cases {
        assert_eq!(end.event(), expected, "{name}");
    }
}

#[test]
fn a_whole_run_is_read_for_the_account_that_asked() {
    let other = PassEnd::Failed {
        account: acct_other(),
        address: "b".into(),
        retry: Retry::NeedsReauth,
        why: "not yours".into(),
        pause: Pause::ServerBusy,
    };
    assert_eq!(
        outcome(Ok(vec![other.clone(), finished(vec![])]), acct_account()),
        Event::Finished { trouble: vec![] },
        "another account's end is not this one's"
    );
    assert_eq!(
        outcome(Ok(vec![other]), acct_account()),
        Event::Finished { trouble: vec![] },
        "a run that left the account out had nothing to do for it"
    );
}

#[test]
fn a_run_that_could_not_happen_waits_and_backs_off_like_any_failure() {
    let event = outcome(Err("cannot start the async runtime".into()), acct_account());
    assert_eq!(
        event,
        Event::Failed {
            retry: Retry::After(Duration::ZERO),
            why: "cannot start the async runtime".into(),
            pause: Pause::ServerBusy,
        }
    );
    // Through the machine: it waits at least the interval, and is not given up on.
    let syncing = Link::Syncing {
        first: First::No,
        step: Step::Connecting,
        count: None,
        after: Box::new(Link::Fresh),
    };
    let now = chrono::Utc::now();
    let (link, effects) = step(&syncing, event, now, secs(300));
    assert!(matches!(link, Link::Waiting { .. }), "{link:?}");
    assert!(
        matches!(effects.as_slice(), [Effect::WakeAt(at)] if *at >= now + chrono::TimeDelta::seconds(300))
    );
}
