//! What mail fetching is doing, in a few words: the sidebar sync mark's tip and its warning.

use chrono::{DateTime, TimeZone, Utc};
use mail_client::fetch::{Count, Link, Live, Pause, Step};
use mail_client::sync::fresh::{self, Freshness, Pace};
use mail_logic::when::Clock;

/// How much the line should draw the eye.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tone {
    Plain,
    Warn,
    Danger,
}

/// The line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusLine {
    pub text: String,
    /// The line in the fewer words the sync mark's tip says it in, after "Sync ·": "3 min ago"
    /// where the line says "Updated 3 minutes ago". The same as `text` for most lines.
    pub tip: String,
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
    let text = text.into();
    StatusLine {
        tip: text.clone(),
        text,
        progress: None,
        tone,
    }
}

/// What to say about `paced`, the accounts in scope, each with what its freshness is judged by.
///
/// Busy wins, because it is what is happening now; then the worst problem, because it is what
/// needs a person; then an account that has not been updated for longer than its schedule
/// allows (`mail_client::sync::fresh`), because "up to date" would be a lie; then how long ago
/// the least recently updated account was, because the line is only as fresh as its stalest
/// part.
pub fn status_line<Tz: TimeZone>(
    paced: &[(&Link, Pace)],
    now: DateTime<Utc>,
    zone: &Tz,
    clock: Clock,
) -> StatusLine
where
    Tz::Offset: std::fmt::Display,
{
    let links: Vec<&Link> = paced.iter().map(|(link, _)| *link).collect();
    if let Some(busy) = busy(&links) {
        return busy;
    }
    if let Some(worst) = links.iter().filter_map(|link| trouble_rank(link)).max() {
        let worst = links.iter().find(|link| trouble_rank(link) == Some(worst));
        return worst.map_or_else(|| line("", Tone::Plain), |link| trouble(link, zone, clock));
    }
    if let Some(since) = stalest(paced, now) {
        return line(not_updated_since(since, now, zone, clock), Tone::Warn);
    }
    updated(paced, now, zone, clock)
}

/// The oldest finished pass among `paced` that is older than its schedule allows.
fn stalest(paced: &[(&Link, Pace)], now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    paced
        .iter()
        .filter_map(|(link, pace)| match fresh::of_link(link, *pace, now) {
            Freshness::Stale { since } => Some(since),
            Freshness::Current | Freshness::Unknown => None,
        })
        .min()
}

/// "Not updated since 15:12", with the day when it was not today.
pub(super) fn not_updated_since<Tz: TimeZone>(
    since: DateTime<Utc>,
    now: DateTime<Utc>,
    zone: &Tz,
    clock: Clock,
) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let (then, today) = (since.with_timezone(zone), now.with_timezone(zone));
    if then.date_naive() == today.date_naive() {
        format!("Not updated since {}", clock_at(since, zone, clock))
    } else {
        format!(
            "Not updated since {}, {}",
            then.format("%b %-d"),
            mail_logic::when::time_of_day(&then, clock)
        )
    }
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
            progress: Some((done, of)),
            ..line(format!("Downloading {done} of {of}"), Tone::Plain)
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

fn trouble<Tz: TimeZone>(link: &Link, zone: &Tz, clock: Clock) -> StatusLine
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
                format!(
                    "{what} \u{b7} trying again at {}",
                    clock_at(*until, zone, clock)
                ),
                Tone::Warn,
            )
        }
        _ => line("", Tone::Plain),
    }
}

/// How long ago the least recently updated account finished a pass: by its own link, or by a
/// pass another process recorded since (`Pace::finished`).
fn updated<Tz: TimeZone>(
    paced: &[(&Link, Pace)],
    now: DateTime<Utc>,
    zone: &Tz,
    clock: Clock,
) -> StatusLine
where
    Tz::Offset: std::fmt::Display,
{
    let currents: Vec<(DateTime<Utc>, usize, Live)> = paced
        .iter()
        .filter_map(|(link, pace)| match link {
            Link::Current { at, trouble, live } => {
                Some((pace.finished(link).unwrap_or(*at), trouble.len(), *live))
            }
            _ => None,
        })
        .collect();
    let Some(&(oldest, _, live)) = currents.iter().min_by_key(|(at, _, _)| *at) else {
        return match paced.is_empty() {
            true => line("", Tone::Plain),
            false => line("Not checked yet", Tone::Plain),
        };
    };
    let (base, short) = match live {
        Live::Pushed => {
            let said = "Up to date".to_owned();
            (said.clone(), said)
        }
        Live::Polling => (
            ago(oldest, now, zone, clock),
            ago_short(oldest, now, zone, clock),
        ),
    };
    let failed = currents
        .iter()
        .map(|(_, folders, _)| folders)
        .sum::<usize>();
    let suffix = match failed {
        0 => None,
        1 => Some("1 folder couldn\u{2019}t update".to_owned()),
        n => Some(format!("{n} folders couldn\u{2019}t update")),
    };
    match suffix {
        None => StatusLine {
            tip: short,
            ..line(base, Tone::Plain)
        },
        Some(suffix) => StatusLine {
            tip: format!("{short} \u{b7} {suffix}"),
            ..line(format!("{base} \u{b7} {suffix}"), Tone::Warn)
        },
    }
}

pub(super) fn ago<Tz: TimeZone>(
    at: DateTime<Utc>,
    now: DateTime<Utc>,
    zone: &Tz,
    clock: Clock,
) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let minutes = (now - at).num_minutes();
    match (now - at).num_seconds() {
        ..60 => "Updated just now".to_owned(),
        _ if minutes == 1 => "Updated 1 minute ago".to_owned(),
        _ if minutes < 60 => format!("Updated {minutes} minutes ago"),
        _ => format!("Updated at {}", clock_at(at, zone, clock)),
    }
}

/// [`ago`] in the sync mark's tip's fewer words: "3 min ago", "just now", "at 12:00".
fn ago_short<Tz: TimeZone>(at: DateTime<Utc>, now: DateTime<Utc>, zone: &Tz, clock: Clock) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let minutes = (now - at).num_minutes();
    match (now - at).num_seconds() {
        ..60 => crate::ui::copy::status::JUST_NOW.to_owned(),
        _ if minutes < 60 => crate::ui::copy::status::minutes_ago(minutes),
        _ => crate::ui::copy::status::at_clock(&clock_at(at, zone, clock)),
    }
}

/// The time of day in the person's zone, by their clock: `14:05`, or `2:05 PM`.
pub(super) fn clock_at<Tz: TimeZone>(at: DateTime<Utc>, zone: &Tz, clock: Clock) -> String
where
    Tz::Offset: std::fmt::Display,
{
    mail_logic::when::stamp(at, zone, mail_logic::when::Stamp::Clock, clock)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeDelta;
    use mail_client::fetch::{First, Trouble};
    use mail_domain::{Retry, WatchMode};
    use std::time::Duration;

    fn t(minute: i64, second: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 1, 12, 0, 0).unwrap()
            + TimeDelta::minutes(minute)
            + TimeDelta::seconds(second)
    }

    /// Every account polled hourly, so that an age reads as an age here; when an account is
    /// too old for its schedule has rows of its own, at the five minutes most accounts poll at.
    fn hourly() -> Pace {
        Pace::of_watch(&WatchMode::Poll {
            every: Duration::from_secs(3600),
        })
    }

    fn say(links: &[Link], now: DateTime<Utc>) -> StatusLine {
        let paced: Vec<(&Link, Pace)> = links.iter().map(|link| (link, hourly())).collect();
        status_line(&paced, now, &Utc, Clock::H24)
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
            (
                "a folder that could not update",
                vec![current(t(0, 0), Live::Polling, 1)],
                t(0, 0),
                "Updated just now \u{b7} 1 folder couldn\u{2019}t update",
                Some(Tone::Warn),
            ),
            (
                "pushed, with two that could not",
                vec![current(t(0, 0), Live::Pushed, 2)],
                t(0, 0),
                "Up to date \u{b7} 2 folders couldn\u{2019}t update",
                None,
            ),
            (
                "the worst problem is the one said",
                vec![
                    waiting(Pause::Unreachable, t(5, 0)),
                    Link::Broken {
                        why: "x".into(),
                        first: First::No,
                    },
                ],
                t(0, 0),
                "Can\u{2019}t fetch mail",
                Some(Tone::Danger),
            ),
        ];
        for (name, links, now, text, tone) in cases {
            let got = say(&links, now);
            assert_eq!(got.text, text, "{name}");
            if let Some(tone) = tone {
                assert_eq!(got.tone, tone, "{name}");
            }
        }
    }

    /// An account that has not finished a pass for longer than its schedule allows says since
    /// when, as a warning: a pushed account whose IDLE a NAT forgot said "Up to date" for forty
    /// hours.
    #[test]
    fn an_account_past_its_schedule_says_since_when() {
        let polled = Pace::of_watch(&WatchMode::Poll {
            every: Duration::from_secs(300),
        });
        let idle = Pace::of_watch(&WatchMode::Idle);
        let pushed = current(t(0, 0), Live::Pushed, 0);
        let quiet = current(t(0, 0), Live::Polling, 0);
        let recent = current(t(50, 0), Live::Polling, 0);
        // A window that leaves fetching to a watch: its own link is old, the watch's last pass
        // is in the store.
        let told = Pace {
            recorded: Some(t(40, 0)),
            ..idle
        };
        type Row<'a> = (
            &'static str,
            Vec<(&'a Link, Pace)>,
            DateTime<Utc>,
            &'static str,
            Tone,
        );
        let cases: Vec<Row> = vec![
            (
                "pushed, within one re-issue",
                vec![(&pushed, idle)],
                t(35, 0),
                "Up to date",
                Tone::Plain,
            ),
            (
                "pushed, past it",
                vec![(&pushed, idle)],
                t(36, 0),
                "Not updated since 12:00",
                Tone::Warn,
            ),
            (
                "polled, three intervals",
                vec![(&quiet, polled)],
                t(15, 0),
                "Updated 15 minutes ago",
                Tone::Plain,
            ),
            (
                "polled, past three",
                vec![(&quiet, polled)],
                t(16, 0),
                "Not updated since 12:00",
                Tone::Warn,
            ),
            (
                "one stale account among fresh ones",
                vec![(&recent, polled), (&pushed, idle)],
                t(51, 0),
                "Not updated since 12:00",
                Tone::Warn,
            ),
            (
                "forty hours: the day is said",
                vec![(&pushed, idle)],
                t(40 * 60, 0),
                "Not updated since Oct 1, 12:00",
                Tone::Warn,
            ),
            (
                "the watch's pass counts, and its age is said",
                vec![(&quiet, told)],
                t(50, 0),
                "Updated 10 minutes ago",
                Tone::Plain,
            ),
        ];
        for (name, links, now, text, tone) in cases {
            let got = status_line(&links, now, &Utc, Clock::H24);
            assert_eq!((got.text.as_str(), got.tone), (text, tone), "{name}");
        }
    }

    /// The sync mark's tip says how fresh the mail is in fewer words than the line, and the
    /// rest of the line as it is.
    #[test]
    fn the_tip_says_the_age_in_fewer_words() {
        use crate::ui::copy::status::{JUST_NOW, at_clock, minutes_ago};
        let cases = [
            (t(0, 0), 0, JUST_NOW.to_owned()),
            (t(1, 0), 0, minutes_ago(1)),
            (t(3, 0), 0, minutes_ago(3)),
            (t(60, 0), 0, at_clock("12:00")),
            (
                t(3, 0),
                1,
                format!("{} \u{b7} 1 folder couldn\u{2019}t update", minutes_ago(3)),
            ),
        ];
        for (now, folders, tip) in cases {
            let got = say(&[current(t(0, 0), Live::Polling, folders)], now);
            assert_eq!(got.tip, tip, "{}", got.text);
        }
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
}
