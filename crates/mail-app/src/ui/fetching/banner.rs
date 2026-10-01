//! The banner above the list when an account needs something from the person.

use super::status::clock;
use chrono::{DateTime, TimeZone, Utc};
use mail_core::fetch::{Link, Pause};

/// An account, by the address the person knows it by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountName(pub String);

/// How loud the banner is. Ordered, so the worst of several is `max`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Sev {
    Info,
    Warn,
    Danger,
}

/// What the banner's button does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BannerAction {
    /// Try the account now instead of waiting.
    TryNow(AccountName),
    /// Sign the account in again.
    SignIn(AccountName),
    /// Open the account's settings.
    Settings(AccountName),
    /// More than one account needs attention: show which.
    ShowAll,
}

/// The banner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Banner {
    pub severity: Sev,
    pub text: String,
    pub action: BannerAction,
    /// How many accounts it speaks for.
    pub count: usize,
}

/// The banner for `links`, if any account needs attention.
///
/// A link that is running a pass is not in trouble, whatever it was before: it is trying.
pub fn banner<Tz: TimeZone>(
    links: &[(AccountName, &Link)],
    _now: DateTime<Utc>,
    zone: &Tz,
) -> Option<Banner>
where
    Tz::Offset: std::fmt::Display,
{
    let mut troubled: Vec<Banner> = links
        .iter()
        .filter_map(|(name, link)| one(name, link, zone))
        .collect();
    match troubled.len() {
        0 => None,
        1 => troubled.pop(),
        count => {
            let severity = troubled.iter().map(|banner| banner.severity).max()?;
            Some(Banner {
                severity,
                text: format!("{count} accounts need attention"),
                action: BannerAction::ShowAll,
                count,
            })
        }
    }
}

fn one<Tz: TimeZone>(name: &AccountName, link: &Link, zone: &Tz) -> Option<Banner>
where
    Tz::Offset: std::fmt::Display,
{
    let addr = &name.0;
    let (severity, text, action) = match link {
        Link::NeedsSignIn { .. } => (
            Sev::Warn,
            format!("Sign in to {addr} again to keep receiving mail."),
            BannerAction::SignIn(name.clone()),
        ),
        Link::Broken { why, .. } => (
            Sev::Danger,
            why.clone(),
            BannerAction::Settings(name.clone()),
        ),
        Link::Waiting { until, why, .. } => {
            let at = clock(*until, zone);
            let (severity, text) = match why {
                Pause::Unreachable => (
                    Sev::Info,
                    format!(
                        "Can\u{2019}t reach the server for {addr}. Mailo will try again at {at}."
                    ),
                ),
                Pause::Throttled => (
                    Sev::Warn,
                    format!("The server asked mailo to slow down. Trying again at {at}."),
                ),
                Pause::ServerBusy => (
                    Sev::Info,
                    format!("The server for {addr} is busy. Mailo will try again at {at}."),
                ),
            };
            (severity, text, BannerAction::TryNow(name.clone()))
        }
        Link::Fresh | Link::Syncing { .. } | Link::Current { .. } => return None,
    };
    Some(Banner {
        severity,
        text,
        action,
        count: 1,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mail_core::fetch::{First, Live};

    fn at() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 1, 12, 30, 0).unwrap()
    }

    fn name(addr: &str) -> AccountName {
        AccountName(addr.to_owned())
    }

    fn show(links: &[(AccountName, Link)]) -> Option<Banner> {
        let refs: Vec<(AccountName, &Link)> = links.iter().map(|(n, l)| (n.clone(), l)).collect();
        banner(&refs, at(), &Utc)
    }

    fn waiting(why: Pause) -> Link {
        Link::Waiting {
            until: at(),
            why,
            failures: 1,
            first: First::No,
        }
    }

    #[test]
    fn each_trouble_has_its_words_and_button() {
        let a = name("a@x.com");
        let cases = [
            (
                Link::NeedsSignIn {
                    why: "x".into(),
                    first: First::No,
                },
                Sev::Warn,
                "Sign in to a@x.com again to keep receiving mail.",
                BannerAction::SignIn(a.clone()),
            ),
            (
                Link::Broken {
                    why: "The server is gone.".into(),
                    first: First::No,
                },
                Sev::Danger,
                "The server is gone.",
                BannerAction::Settings(a.clone()),
            ),
            (
                waiting(Pause::Unreachable),
                Sev::Info,
                "Can\u{2019}t reach the server for a@x.com. Mailo will try again at 12:30.",
                BannerAction::TryNow(a.clone()),
            ),
            (
                waiting(Pause::Throttled),
                Sev::Warn,
                "The server asked mailo to slow down. Trying again at 12:30.",
                BannerAction::TryNow(a.clone()),
            ),
            (
                waiting(Pause::ServerBusy),
                Sev::Info,
                "The server for a@x.com is busy. Mailo will try again at 12:30.",
                BannerAction::TryNow(a.clone()),
            ),
        ];
        for (link, severity, text, action) in cases {
            let got = show(&[(a.clone(), link.clone())]);
            assert_eq!(
                got,
                Some(Banner {
                    severity,
                    text: text.into(),
                    action,
                    count: 1
                }),
                "{link:?}"
            );
        }
    }

    #[test]
    fn healthy_accounts_have_no_banner() {
        let ok = Link::Current {
            at: at(),
            trouble: vec![],
            live: Live::Polling,
        };
        assert_eq!(show(&[(name("a"), ok), (name("b"), Link::Fresh)]), None);
        assert_eq!(show(&[]), None);
    }

    #[test]
    fn several_accounts_in_trouble_share_one_banner_at_the_worst_severity() {
        let links = [
            (name("a"), waiting(Pause::Unreachable)),
            (
                name("b"),
                Link::Broken {
                    why: "x".into(),
                    first: First::No,
                },
            ),
            (
                name("c"),
                Link::Current {
                    at: at(),
                    trouble: vec![],
                    live: Live::Polling,
                },
            ),
        ];
        assert_eq!(
            show(&links),
            Some(Banner {
                severity: Sev::Danger,
                text: "2 accounts need attention".into(),
                action: BannerAction::ShowAll,
                count: 2
            })
        );
    }

    #[test]
    fn a_healthy_account_does_not_hide_one_in_trouble() {
        let ok = Link::Current {
            at: at(),
            trouble: vec![],
            live: Live::Polling,
        };
        let links = [
            (name("a"), ok),
            (
                name("b"),
                Link::NeedsSignIn {
                    why: "x".into(),
                    first: First::No,
                },
            ),
        ];
        assert_eq!(
            show(&links).map(|b| b.action),
            Some(BannerAction::SignIn(name("b")))
        );
    }
}
