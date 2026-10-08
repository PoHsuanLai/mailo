//! One account's line in the Connection Doctor: how it stands, in a few words, and the one thing
//! the person can do about it.
//!
//! Pure, like [`super::status`]: a [`Link`] and the time in, words out. No line tells the person
//! to type a command; every remedy is a button in the sheet.

use super::status::{ago, clock};
use chrono::{DateTime, TimeZone, Utc};
use mail_core::fetch::{Link, Live, Pause, Trouble};

/// The glyph at the start of an account's line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Standing {
    /// All is well.
    Fine,
    /// A pass is running.
    Working,
    /// Something is wrong that the person can act on, or that waiting may fix.
    Warn,
    /// The server cannot be reached.
    Offline,
    /// Nothing will fix itself.
    Broken,
}

/// What the line's one button does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Remedy {
    /// Replace the password.
    SignIn,
    /// Let Mail use the desktop's account again: the grant was withdrawn.
    Allow,
    /// Ask for a pass now instead of waiting.
    TryAgain,
    /// Open the account's settings.
    Settings,
}

/// An account's line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountLine {
    pub standing: Standing,
    pub text: String,
    pub remedy: Option<Remedy>,
}

fn said(standing: Standing, text: impl Into<String>, remedy: Option<Remedy>) -> AccountLine {
    AccountLine {
        standing,
        text: text.into(),
        remedy,
    }
}

/// How `link` stands at `now`.
pub fn account_line<Tz: TimeZone>(link: &Link, now: DateTime<Utc>, zone: &Tz) -> AccountLine
where
    Tz::Offset: std::fmt::Display,
{
    match link {
        Link::Fresh => said(Standing::Fine, "Not checked yet", Some(Remedy::TryAgain)),
        Link::Syncing { .. } => said(Standing::Working, "Checking\u{2026}", None),
        Link::NeedsSignIn { .. } => said(Standing::Warn, "Sign-in needed", Some(Remedy::SignIn)),
        Link::NeedsAllow { .. } => said(
            Standing::Warn,
            "Mail isn\u{2019}t allowed to use this account",
            Some(Remedy::Allow),
        ),
        Link::Broken { why, .. } => said(Standing::Broken, why.clone(), Some(Remedy::Settings)),
        Link::Waiting { until, why, .. } => {
            let at = clock(*until, zone);
            let (standing, what) = match why {
                Pause::Unreachable => (Standing::Offline, "Can\u{2019}t reach the server"),
                Pause::Throttled => (Standing::Warn, "The server asked to slow down"),
                Pause::ServerBusy => (Standing::Warn, "The server is busy"),
            };
            said(
                standing,
                format!("{what} \u{2014} trying again at {at}"),
                Some(Remedy::TryAgain),
            )
        }
        Link::Current { at, trouble, live } if trouble.is_empty() => said(
            Standing::Fine,
            match live {
                Live::Pushed => "Connected".to_owned(),
                Live::Polling => ago(*at, now, zone),
            },
            None,
        ),
        Link::Current { trouble, .. } => said(
            Standing::Warn,
            folders_that_failed(trouble),
            Some(Remedy::TryAgain),
        ),
    }
}

/// "Couldn't update 2 folders: Archive, Sent"; a failure that belongs to no folder says why.
fn folders_that_failed(trouble: &[Trouble]) -> String {
    let names: Vec<&str> = trouble
        .iter()
        .filter_map(|one| one.mailbox.as_deref())
        .collect();
    match (names.as_slice(), trouble.first()) {
        ([], Some(first)) => first.why.clone(),
        ([one], _) => format!("Couldn\u{2019}t update 1 folder: {one}"),
        (many, _) => format!(
            "Couldn\u{2019}t update {} folders: {}",
            many.len(),
            many.join(", ")
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mail_core::fetch::{First, Step};
    use mail_domain::Retry;

    fn at() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 1, 14, 5, 0).unwrap()
    }

    fn line(link: &Link) -> AccountLine {
        account_line(link, at() + chrono::TimeDelta::minutes(5), &Utc)
    }

    fn waiting(why: Pause) -> Link {
        Link::Waiting {
            until: at(),
            why,
            failures: 1,
            first: First::No,
        }
    }

    fn trouble(mailbox: Option<&str>) -> Trouble {
        Trouble {
            mailbox: mailbox.map(str::to_owned),
            retry: Retry::Now,
            why: "no room".into(),
        }
    }

    fn current(live: Live, trouble: Vec<Trouble>) -> Link {
        Link::Current {
            at: at(),
            trouble,
            live,
        }
    }

    #[test]
    fn each_state_has_its_words_glyph_and_remedy() {
        let syncing = Link::Syncing {
            first: First::No,
            step: Step::Connecting,
            count: None,
            after: Box::new(Link::Fresh),
        };
        let signed_out = Link::NeedsSignIn {
            why: "x".into(),
            first: First::No,
        };
        let broken = Link::Broken {
            why: "The server is gone.".into(),
            first: First::No,
        };
        let withdrawn = Link::NeedsAllow {
            why: "x".into(),
            first: First::No,
        };
        let cases = [
            (
                Link::Fresh,
                Standing::Fine,
                "Not checked yet",
                Some(Remedy::TryAgain),
            ),
            (syncing, Standing::Working, "Checking\u{2026}", None),
            (
                signed_out,
                Standing::Warn,
                "Sign-in needed",
                Some(Remedy::SignIn),
            ),
            (
                broken,
                Standing::Broken,
                "The server is gone.",
                Some(Remedy::Settings),
            ),
            (
                withdrawn,
                Standing::Warn,
                "Mail isn\u{2019}t allowed to use this account",
                Some(Remedy::Allow),
            ),
            (
                waiting(Pause::Unreachable),
                Standing::Offline,
                "Can\u{2019}t reach the server \u{2014} trying again at 14:05",
                Some(Remedy::TryAgain),
            ),
            (
                waiting(Pause::Throttled),
                Standing::Warn,
                "The server asked to slow down \u{2014} trying again at 14:05",
                Some(Remedy::TryAgain),
            ),
            (
                waiting(Pause::ServerBusy),
                Standing::Warn,
                "The server is busy \u{2014} trying again at 14:05",
                Some(Remedy::TryAgain),
            ),
            (
                current(Live::Pushed, vec![]),
                Standing::Fine,
                "Connected",
                None,
            ),
            (
                current(Live::Polling, vec![]),
                Standing::Fine,
                "Updated 5 minutes ago",
                None,
            ),
        ];
        for (link, standing, text, remedy) in cases {
            assert_eq!(line(&link), said(standing, text, remedy), "{link:?}");
        }
    }

    #[test]
    fn folders_that_could_not_update_are_named() {
        let two = current(
            Live::Polling,
            vec![trouble(Some("Archive")), trouble(Some("Sent"))],
        );
        assert_eq!(
            line(&two),
            said(
                Standing::Warn,
                "Couldn\u{2019}t update 2 folders: Archive, Sent",
                Some(Remedy::TryAgain)
            )
        );
        let one = current(Live::Polling, vec![trouble(Some("Archive"))]);
        assert_eq!(line(&one).text, "Couldn\u{2019}t update 1 folder: Archive");
        let none = current(Live::Polling, vec![trouble(None)]);
        assert_eq!(line(&none).text, "no room");
    }

    #[test]
    fn no_line_speaks_of_a_command() {
        let links = [
            Link::Fresh,
            waiting(Pause::Unreachable),
            waiting(Pause::Throttled),
            waiting(Pause::ServerBusy),
            Link::NeedsSignIn {
                why: "x".into(),
                first: First::No,
            },
            current(Live::Polling, vec![trouble(Some("A"))]),
        ];
        for link in links {
            let text = line(&link).text.to_lowercase();
            assert!(!text.contains("mailo "), "{text}");
        }
    }
}
