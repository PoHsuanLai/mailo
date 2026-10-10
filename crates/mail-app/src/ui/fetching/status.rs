//! The list's subtitle: what mail fetching is doing, in a few words.

use chrono::{DateTime, TimeZone, Utc};
use mail_core::fetch::{Count, Link, Live, Pause, Step};

/// How much the line should draw the eye.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tone {
    Plain,
    Warn,
    Danger,
}

/// The subtitle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusLine {
    pub text: String,
    /// Done and total, when something is being counted; the fraction to draw.
    pub progress: Option<(u32, u32)>,
    pub tone: Tone,
}

/// How far `done` of `of` is, in thousandths: what a determinate progress bar is drawn from.
/// An unknown total (zero) is nothing done, and more done than there is is all of it.
pub fn thousandths(done: u32, of: u32) -> u16 {
    if of == 0 {
        return 0;
    }
    let share = u64::from(done.min(of)) * 1000 / u64::from(of);
    u16::try_from(share).unwrap_or(1000)
}

fn line(text: impl Into<String>, tone: Tone) -> StatusLine {
    StatusLine {
        text: text.into(),
        progress: None,
        tone,
    }
}

/// What to say about `links`, the accounts in scope.
///
/// Busy wins, because it is what is happening now; then the worst problem, because it is what
/// needs a person; then how long ago the least recently updated account was, because the line
/// is only as fresh as its stalest part.
pub fn status_line<Tz: TimeZone>(links: &[&Link], now: DateTime<Utc>, zone: &Tz) -> StatusLine
where
    Tz::Offset: std::fmt::Display,
{
    if let Some(busy) = busy(links) {
        return busy;
    }
    if let Some(worst) = links.iter().filter_map(|link| trouble_rank(link)).max() {
        let worst = links.iter().find(|link| trouble_rank(link) == Some(worst));
        return worst.map_or_else(|| line("", Tone::Plain), |link| trouble(link, zone));
    }
    updated(links, now, zone)
}

fn busy(links: &[&Link]) -> Option<StatusLine> {
    let running: Vec<(&Step, Option<Count>)> = links
        .iter()
        .filter_map(|link| match link {
            Link::Syncing { step, count, .. } => Some((step, *count)),
            _ => None,
        })
        .collect();
    if running.is_empty() {
        return None;
    }
    let counted = running
        .iter()
        .filter(|(step, _)| matches!(step, Step::Headers { .. } | Step::Bodies))
        .filter_map(|(_, count)| *count)
        .fold(None, |sum: Option<Count>, count| {
            Some(Count {
                done: sum.map_or(0, |s| s.done) + count.done,
                of: sum.map_or(0, |s| s.of) + count.of,
            })
        });
    Some(match counted {
        Some(Count { done, of }) => StatusLine {
            text: format!("Downloading {done} of {of}"),
            progress: Some((done, of)),
            tone: Tone::Plain,
        },
        None if running.iter().any(|(step, _)| **step == Step::Sending) => {
            line("Sending\u{2026}", Tone::Plain)
        }
        None => line("Checking for mail\u{2026}", Tone::Plain),
    })
}

/// How badly a resting link is off. Higher is worse; `None` is fine.
fn trouble_rank(link: &Link) -> Option<u8> {
    match link {
        Link::Broken { .. } => Some(5),
        Link::NeedsSignIn { .. } | Link::NeedsAllow { .. } => Some(4),
        Link::Waiting {
            why: Pause::Throttled,
            ..
        } => Some(3),
        Link::Waiting {
            why: Pause::ServerBusy,
            ..
        } => Some(2),
        Link::Waiting {
            why: Pause::Unreachable,
            ..
        } => Some(1),
        _ => None,
    }
}

fn trouble<Tz: TimeZone>(link: &Link, zone: &Tz) -> StatusLine
where
    Tz::Offset: std::fmt::Display,
{
    match link {
        Link::Broken { .. } => line("Can\u{2019}t fetch mail", Tone::Danger),
        Link::NeedsSignIn { .. } => line("Sign-in needed", Tone::Warn),
        Link::NeedsAllow { .. } => line("Mail not allowed", Tone::Warn),
        Link::Waiting { until, why, .. } => {
            let what = match why {
                Pause::Unreachable => "Offline",
                Pause::Throttled => "Slowing down",
                Pause::ServerBusy => "Server busy",
            };
            line(
                format!("{what} \u{b7} trying again at {}", clock(*until, zone)),
                Tone::Warn,
            )
        }
        _ => line("", Tone::Plain),
    }
}

fn updated<Tz: TimeZone>(links: &[&Link], now: DateTime<Utc>, zone: &Tz) -> StatusLine
where
    Tz::Offset: std::fmt::Display,
{
    let currents: Vec<(DateTime<Utc>, usize, Live)> = links
        .iter()
        .filter_map(|link| match link {
            Link::Current { at, trouble, live } => Some((*at, trouble.len(), *live)),
            _ => None,
        })
        .collect();
    let Some(&(oldest, _, live)) = currents.iter().min_by_key(|(at, _, _)| *at) else {
        return match links.is_empty() {
            true => line("", Tone::Plain),
            false => line("Not checked yet", Tone::Plain),
        };
    };
    let base = match live {
        Live::Pushed => "Up to date".to_owned(),
        Live::Polling => ago(oldest, now, zone),
    };
    match currents
        .iter()
        .map(|(_, folders, _)| folders)
        .sum::<usize>()
    {
        0 => line(base, Tone::Plain),
        1 => line(
            format!("{base} \u{b7} 1 folder couldn\u{2019}t update"),
            Tone::Warn,
        ),
        n => line(
            format!("{base} \u{b7} {n} folders couldn\u{2019}t update"),
            Tone::Warn,
        ),
    }
}

pub(super) fn ago<Tz: TimeZone>(at: DateTime<Utc>, now: DateTime<Utc>, zone: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let minutes = (now - at).num_minutes();
    match (now - at).num_seconds() {
        ..60 => "Updated just now".to_owned(),
        _ if minutes == 1 => "Updated 1 minute ago".to_owned(),
        _ if minutes < 60 => format!("Updated {minutes} minutes ago"),
        _ => format!("Updated at {}", clock(at, zone)),
    }
}

/// HH:MM in the person's zone.
pub(super) fn clock<Tz: TimeZone>(at: DateTime<Utc>, zone: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    at.with_timezone(zone).format("%H:%M").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeDelta;
    use mail_core::fetch::{First, Trouble};
    use mail_domain::Retry;

    fn t(minute: i64, second: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 1, 12, 0, 0).unwrap()
            + TimeDelta::minutes(minute)
            + TimeDelta::seconds(second)
    }

    fn say(links: &[Link], now: DateTime<Utc>) -> StatusLine {
        let refs: Vec<&Link> = links.iter().collect();
        status_line(&refs, now, &Utc)
    }

    fn current(at: DateTime<Utc>, live: Live, folders: usize) -> Link {
        let one = Trouble {
            mailbox: None,
            retry: Retry::Now,
            why: "x".into(),
        };
        Link::Current {
            at,
            trouble: vec![one; folders],
            live,
        }
    }

    fn syncing(step: Step, count: Option<Count>) -> Link {
        Link::Syncing {
            first: First::No,
            step,
            count,
            after: Box::new(Link::Fresh),
        }
    }

    fn waiting(why: Pause, until: DateTime<Utc>) -> Link {
        Link::Waiting {
            until,
            why,
            failures: 1,
            first: First::No,
        }
    }

    #[test]
    fn a_share_is_in_thousandths_and_never_leaves_the_bar() {
        assert_eq!(thousandths(0, 0), 0);
        assert_eq!(thousandths(3, 40), 75);
        assert_eq!(thousandths(1, 3), 333);
        assert_eq!(thousandths(40, 40), 1000);
        assert_eq!(thousandths(50, 40), 1000);
    }

    #[test]
    fn busy_says_what_is_happening() {
        let c = Some(Count { done: 3, of: 40 });
        let cases = [
            (
                syncing(Step::Connecting, None),
                "Checking for mail\u{2026}",
                None,
            ),
            (syncing(Step::Flags, c), "Checking for mail\u{2026}", None),
            (
                syncing(Step::Bodies, c),
                "Downloading 3 of 40",
                Some((3, 40)),
            ),
            (
                syncing(
                    Step::Headers {
                        mailbox: "A".into(),
                    },
                    c,
                ),
                "Downloading 3 of 40",
                Some((3, 40)),
            ),
            (
                syncing(Step::Bodies, None),
                "Checking for mail\u{2026}",
                None,
            ),
            (syncing(Step::Sending, None), "Sending\u{2026}", None),
        ];
        for (link, text, progress) in cases {
            let got = say(std::slice::from_ref(&link), t(0, 0));
            assert_eq!(
                (got.text.as_str(), got.progress, got.tone),
                (text, progress, Tone::Plain),
                "{link:?}"
            );
        }
    }

    #[test]
    fn busy_beats_trouble_and_sums_counts() {
        let a = Some(Count { done: 1, of: 10 });
        let b = Some(Count { done: 2, of: 5 });
        let links = [
            Link::Broken {
                why: "x".into(),
                first: First::No,
            },
            syncing(Step::Bodies, a),
            syncing(Step::Bodies, b),
        ];
        assert_eq!(say(&links, t(0, 0)).progress, Some((3, 15)));
    }

    /// What the line says when nothing is wrong: an account's age, read from the oldest one,
    /// "Up to date" for one the server pushes to, and "Not checked yet" before any fetch.
    #[test]
    fn the_status_line_says() {
        let polled = || vec![current(t(0, 0), Live::Polling, 0)];
        // (row, the links, now, what it says, its tone where the row is about the tone too)
        type Row = (
            &'static str,
            Vec<Link>,
            DateTime<Utc>,
            &'static str,
            Option<Tone>,
        );
        let cases: Vec<Row> = vec![
            (
                "0 s old",
                polled(),
                t(0, 0),
                "Updated just now",
                Some(Tone::Plain),
            ),
            (
                "59 s old",
                polled(),
                t(0, 59),
                "Updated just now",
                Some(Tone::Plain),
            ),
            (
                "60 s old",
                polled(),
                t(0, 60),
                "Updated 1 minute ago",
                Some(Tone::Plain),
            ),
            (
                "5 min old",
                polled(),
                t(0, 5 * 60),
                "Updated 5 minutes ago",
                Some(Tone::Plain),
            ),
            (
                "59 min 59 s old",
                polled(),
                t(0, 59 * 60 + 59),
                "Updated 59 minutes ago",
                Some(Tone::Plain),
            ),
            (
                "an hour old",
                polled(),
                t(0, 3600),
                "Updated at 12:00",
                Some(Tone::Plain),
            ),
            (
                "from the future",
                polled(),
                t(0, -30),
                "Updated just now",
                Some(Tone::Plain),
            ),
            (
                "pushed",
                vec![current(t(0, 0), Live::Pushed, 0)],
                t(50, 0),
                "Up to date",
                None,
            ),
            (
                "the oldest account sets the age",
                vec![
                    current(t(9, 0), Live::Polling, 0),
                    current(t(0, 0), Live::Polling, 0),
                ],
                t(10, 0),
                "Updated 10 minutes ago",
                None,
            ),
            (
                "nothing fetched yet",
                vec![Link::Fresh],
                t(0, 0),
                "Not checked yet",
                None,
            ),
            ("no accounts", vec![], t(0, 0), "", None),
        ];
        for (name, links, now, text, tone) in cases {
            let got = say(&links, now);
            assert_eq!(got.text, text, "{name}");
            if let Some(tone) = tone {
                assert_eq!(got.tone, tone, "{name}");
            }
        }
    }

    #[test]
    fn folders_that_could_not_update_are_counted() {
        let one = say(&[current(t(0, 0), Live::Polling, 1)], t(0, 0));
        assert_eq!(
            one.text,
            "Updated just now \u{b7} 1 folder couldn\u{2019}t update"
        );
        assert_eq!(one.tone, Tone::Warn);
        let two = say(&[current(t(0, 0), Live::Pushed, 2)], t(0, 0));
        assert_eq!(
            two.text,
            "Up to date \u{b7} 2 folders couldn\u{2019}t update"
        );
    }

    #[test]
    fn trouble_beats_a_healthy_account() {
        let at = t(30, 0);
        let cases = [
            (
                waiting(Pause::Unreachable, at),
                "Offline \u{b7} trying again at 12:30",
                Tone::Warn,
            ),
            (
                waiting(Pause::Throttled, at),
                "Slowing down \u{b7} trying again at 12:30",
                Tone::Warn,
            ),
            (
                waiting(Pause::ServerBusy, at),
                "Server busy \u{b7} trying again at 12:30",
                Tone::Warn,
            ),
            (
                Link::NeedsSignIn {
                    why: "x".into(),
                    first: First::No,
                },
                "Sign-in needed",
                Tone::Warn,
            ),
            (
                Link::Broken {
                    why: "x".into(),
                    first: First::No,
                },
                "Can\u{2019}t fetch mail",
                Tone::Danger,
            ),
        ];
        for (link, text, tone) in cases {
            let got = say(&[current(t(0, 0), Live::Polling, 0), link.clone()], t(1, 0));
            assert_eq!((got.text.as_str(), got.tone), (text, tone), "{link:?}");
        }
    }

    #[test]
    fn the_worst_problem_is_the_one_said() {
        let links = [
            waiting(Pause::Unreachable, t(5, 0)),
            Link::Broken {
                why: "x".into(),
                first: First::No,
            },
        ];
        assert_eq!(say(&links, t(0, 0)).tone, Tone::Danger);
    }
}
