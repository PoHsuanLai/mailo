//! Writing an instant the way the person reading it keeps time: the list's column, the
//! reader's header, the attribution line above quoted text.

use chrono::{DateTime, Datelike as _, NaiveDate, NaiveTime, TimeZone, Timelike, Utc};

/// How the person reading keeps the hours of the day: `14:05` or `2:05 PM`.
///
/// One setting for the whole desktop, which the application reads and hands down; this crate
/// reads no locale and no settings file. Twenty-four hours when nothing says otherwise, because
/// that is what every stamp here wrote before there was a choice, and it is the reading no one
/// mistakes for the other half of the day.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Clock {
    /// `2:05 PM`, `12:00 AM` at midnight.
    H12,
    /// `14:05`, `00:00` at midnight.
    #[default]
    H24,
}

/// A time of day as the person keeps time: `14:05`, or `2:05 PM`.
///
/// The one place an hour is written for a person. The snooze menu wrote twelve hours while
/// everything around it wrote twenty-four, each with its own pattern; every stamp below, and
/// every surface that shows an hour, now comes here. Takes anything with a time of day, so a
/// calendar's floating times (no zone) are written the same way as an instant in one.
pub fn time_of_day(at: &impl Timelike, clock: Clock) -> String {
    let time = NaiveTime::from_hms_opt(at.hour(), at.minute(), 0).unwrap_or(NaiveTime::MIN);
    let pattern = match clock {
        Clock::H12 => "%-I:%M %p",
        Clock::H24 => "%H:%M",
    };
    time.format(pattern).to_string()
}

/// Where an instant appears, and therefore how much of it is written.
///
/// An enum rather than a format string at each call site, because the call sites disagreed
/// about the pattern and agreed about the bug.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stamp {
    /// A row in a list: `09-22 09:02`. The year is omitted; the column is narrow and the
    /// question a list answers is "when today".
    Row,
    /// A message in the reader: `2026-09-22 09:02`. Opened deliberately, so it says everything.
    Full,
    /// The attribution line above quoted text: `Tue, 22 Sep 2026 at 09:02`.
    ///
    /// The one stamp that leaves this machine. It is written into the body of a reply, so a
    /// wrong one is wrong in someone else's mailbox, permanently, and no later fix reaches it.
    Quote,
    /// A moment a few days off, where the day matters less than the hour: `Tue 08:00`.
    Weekday,
    /// The time of day alone: `08:00`.
    Clock,
}

impl Stamp {
    /// What is written ahead of the time of day, which [`time_of_day`] writes.
    fn day(self) -> Option<&'static str> {
        match self {
            Stamp::Row => Some("%m-%d"),
            Stamp::Full => Some("%Y-%m-%d"),
            Stamp::Quote => Some("%a, %d %b %Y at"),
            Stamp::Weekday => Some("%a"),
            Stamp::Clock => None,
        }
    }
}

/// How a list row writes an instant: precisely enough to be useful, briefly enough to fit.
///
/// The time for today, a weekday for the last week, a day and month within the year, and a full
/// date beyond it. This is what every mail client does and for the same reason — the shell wrote
/// `Sep 22` on a message that arrived an hour ago, which is both the least useful answer
/// available and the same answer it gives for a message from three weeks ago.
///
/// `now` is a parameter for the usual reason: a function that reads the clock decides its own
/// test's answer.
pub fn listed<Tz: TimeZone>(
    instant: DateTime<Utc>,
    now: DateTime<Utc>,
    zone: &Tz,
    clock: Clock,
) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let then = instant.with_timezone(zone);
    let today = now.with_timezone(zone);
    // Calendar days, not elapsed hours: 23:50 yesterday is not "today" because it is within
    // twenty-four hours, and 00:10 this morning is.
    let days = days_between(then.date_naive(), today.date_naive());
    if days == 0 {
        time_of_day(&then, clock)
    } else if (1..7).contains(&days) {
        then.format("%a").to_string()
    } else if then.format("%Y").to_string() == today.format("%Y").to_string() {
        then.format("%b %d").to_string()
    } else {
        // A year old. The year is the only part that still says anything.
        then.format("%Y-%m-%d").to_string()
    }
}

/// How many calendar days `to` is after `from`; negative when it is before.
fn days_between(from: NaiveDate, to: NaiveDate) -> i64 {
    to.signed_duration_since(from).num_days()
}

/// A time to come as a person says it: `Today 17:00`, `Tomorrow 08:00`, `Tue 08:00` within the
/// week, `Tue 6 Oct 08:00` past it, with the year only when it is not this one. The one wording
/// for a time something waits until: the window's Sends menu and outbox pill, and the command
/// line's scheduled sends.
pub fn upcoming<Tz: TimeZone>(
    instant: DateTime<Utc>,
    now: DateTime<Utc>,
    zone: &Tz,
    clock: Clock,
) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let there = instant.with_timezone(zone);
    let here = now.with_timezone(zone);
    let clock = time_of_day(&there, clock);
    match days_between(here.date_naive(), there.date_naive()) {
        -1 => format!("Yesterday {clock}"),
        0 => format!("Today {clock}"),
        1 => format!("Tomorrow {clock}"),
        2..=6 => format!("{} {clock}", there.format("%a")),
        _ if there.year() == here.year() => format!("{} {clock}", there.format("%a %-d %b")),
        _ => format!("{} {clock}", there.format("%a %-d %b %Y")),
    }
}

/// Write an instant the way the person reading it keeps time.
///
/// Every instant is stored in UTC, which is the only sane way to keep one, and every instant a
/// person reads is in their own zone. Nothing converted between the two: the list, the reader
/// and the drafts pane each formatted the `DateTime<Utc>` directly. On this machine —
/// `Asia/Taipei`, `+0800` — mail that arrived at 09:02 was shown as 01:02, and every message
/// that arrived after 16:00 was filed under the previous day. At the moment this was found the
/// clock read 07:25 on the 22nd and the whole application was showing the 21st.
///
/// The zone is a parameter and not `Local` read from inside, because a function that reads the
/// machine's zone can only be tested against whatever that machine is set to — which on a
/// machine set to UTC is the bug passing.
///
/// The hour is written as `clock` says ([`time_of_day`]).
pub fn stamp<Tz: TimeZone>(instant: DateTime<Utc>, zone: &Tz, stamp: Stamp, clock: Clock) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let there = instant.with_timezone(zone);
    let time = time_of_day(&there, clock);
    match stamp.day() {
        Some(day) => format!("{} {time}", there.format(day)),
        None => time,
    }
}

/// Dates, which were shown in UTC everywhere.
#[cfg(test)]
mod stamps {
    use super::*;
    use chrono::FixedOffset;

    fn taipei() -> FixedOffset {
        FixedOffset::east_opt(8 * 3600).unwrap()
    }

    /// 2026-09-22 09:02 in Taipei, which is 01:02 the same day in UTC.
    fn morning() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 22, 1, 2, 0).unwrap()
    }

    #[test]
    fn an_instant_is_written_in_the_readers_zone_not_in_utc() {
        // The whole finding in one line: this read "09-22 01:02" for mail that arrived while
        // the user was having breakfast.
        assert_eq!(
            stamp(morning(), &taipei(), Stamp::Row, Clock::H24),
            "09-22 09:02"
        );
        assert_eq!(
            stamp(morning(), &taipei(), Stamp::Full, Clock::H24),
            "2026-09-22 09:02"
        );
    }

    #[test]
    fn an_evening_message_is_not_filed_under_tomorrow() {
        // 23:30 UTC is 07:30 the next morning in Taipei. Shown as UTC, every message that
        // arrived after 16:00 local carried the previous day's date — which is precisely the
        // state the application was in when this was found: the clock read the 22nd and every
        // row said the 21st.
        let evening = Utc.with_ymd_and_hms(2026, 9, 21, 23, 30, 0).unwrap();
        assert_eq!(
            stamp(evening, &taipei(), Stamp::Row, Clock::H24),
            "09-22 07:30"
        );
        assert_eq!(stamp(evening, &Utc, Stamp::Row, Clock::H24), "09-21 23:30");
    }

    #[test]
    fn a_zone_behind_utc_rolls_the_other_way() {
        // Not "add eight hours somewhere". New York is five behind, so the same instant is the
        // day before, and a client that only ever shifted forward would be wrong here.
        let newyork = FixedOffset::west_opt(5 * 3600).unwrap();
        let just_after_midnight = Utc.with_ymd_and_hms(2026, 9, 22, 3, 15, 0).unwrap();
        assert_eq!(
            stamp(just_after_midnight, &newyork, Stamp::Row, Clock::H24),
            "09-21 22:15"
        );
    }

    /// The clock reads 2026-09-22 14:00 in Taipei.
    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 22, 6, 0, 0).unwrap()
    }

    #[test]
    fn todays_mail_shows_a_time_and_older_mail_does_not() {
        // The shell wrote "Sep 22" on a message that arrived an hour ago — the least useful
        // answer available, and the same one it gave for a message from three weeks back.
        let zone = taipei();
        assert_eq!(listed(morning(), now(), &zone, Clock::H24), "09:02");
        // Yesterday evening in Taipei, which is a different calendar day and so not a time.
        let yesterday = Utc.with_ymd_and_hms(2026, 9, 21, 12, 0, 0).unwrap();
        assert_eq!(listed(yesterday, now(), &zone, Clock::H24), "Mon");
        // Three weeks: past the weekday window, inside the year.
        let weeks_ago = Utc.with_ymd_and_hms(2026, 9, 1, 2, 0, 0).unwrap();
        assert_eq!(listed(weeks_ago, now(), &zone, Clock::H24), "Sep 01");
        // Last year, where the year is the only part still worth printing.
        let old = Utc.with_ymd_and_hms(2024, 11, 14, 22, 13, 0).unwrap();
        assert_eq!(listed(old, now(), &zone, Clock::H24), "2024-11-15");
    }

    #[test]
    fn today_is_a_calendar_day_not_the_last_twenty_four_hours() {
        // 00:10 this morning is today although it is fourteen hours ago; 23:50 last night is
        // not, although it is fourteen hours ago too. Elapsed-hours arithmetic gets both wrong.
        let zone = taipei();
        let just_after_midnight = Utc.with_ymd_and_hms(2026, 9, 21, 16, 10, 0).unwrap();
        assert_eq!(
            listed(just_after_midnight, now(), &zone, Clock::H24),
            "00:10"
        );
        let late_last_night = Utc.with_ymd_and_hms(2026, 9, 21, 15, 50, 0).unwrap();
        assert_eq!(listed(late_last_night, now(), &zone, Clock::H24), "Mon");
    }

    #[test]
    fn the_list_stamp_is_in_the_readers_zone_too() {
        // The same instant is today in Taipei and yesterday in New York, so the two disagree
        // about which shape to use at all — not just about the digits.
        let newyork = chrono::FixedOffset::west_opt(5 * 3600).unwrap();
        assert_eq!(listed(morning(), now(), &taipei(), Clock::H24), "09:02");
        assert_eq!(listed(morning(), now(), &newyork, Clock::H24), "Mon");
    }

    #[test]
    fn utc_is_left_alone() {
        // Someone whose machine is on UTC must see what was stored — this is the case that
        // made the bug invisible, because it is the one every test machine is in.
        assert_eq!(
            stamp(morning(), &Utc, Stamp::Full, Clock::H24),
            "2026-09-22 01:02"
        );
    }

    #[test]
    fn the_hour_is_written_as_the_person_keeps_time() {
        let at = |h, m| NaiveTime::from_hms_opt(h, m, 0).unwrap();
        for (time, h24, h12) in [
            (at(14, 5), "14:05", "2:05 PM"),
            (at(0, 0), "00:00", "12:00 AM"),
            (at(12, 0), "12:00", "12:00 PM"),
            (at(9, 30), "09:30", "9:30 AM"),
            (at(23, 59), "23:59", "11:59 PM"),
        ] {
            assert_eq!(time_of_day(&time, Clock::H24), h24);
            assert_eq!(time_of_day(&time, Clock::H12), h12);
        }
        // Twenty-four hours is what nothing chosen means: what every stamp wrote before.
        assert_eq!(Clock::default(), Clock::H24);
    }

    #[test]
    fn every_stamp_writes_its_hour_by_the_persons_clock() {
        let zone = taipei();
        assert_eq!(
            stamp(morning(), &zone, Stamp::Row, Clock::H12),
            "09-22 9:02 AM"
        );
        assert_eq!(
            stamp(morning(), &zone, Stamp::Quote, Clock::H12),
            "Tue, 22 Sep 2026 at 9:02 AM"
        );
        assert_eq!(
            stamp(morning(), &zone, Stamp::Quote, Clock::H24),
            "Tue, 22 Sep 2026 at 09:02"
        );
        assert_eq!(
            stamp(morning(), &zone, Stamp::Weekday, Clock::H12),
            "Tue 9:02 AM"
        );
        assert_eq!(stamp(morning(), &zone, Stamp::Clock, Clock::H12), "9:02 AM");
    }

    #[test]
    fn the_list_and_a_time_to_come_follow_the_clock() {
        let zone = taipei();
        assert_eq!(listed(morning(), now(), &zone, Clock::H12), "9:02 AM");
        // Only today's row is an hour; the other shapes have none to write.
        let yesterday = Utc.with_ymd_and_hms(2026, 9, 21, 12, 0, 0).unwrap();
        assert_eq!(listed(yesterday, now(), &zone, Clock::H12), "Mon");
        let tomorrow_evening = Utc.with_ymd_and_hms(2026, 9, 23, 11, 0, 0).unwrap();
        assert_eq!(
            upcoming(tomorrow_evening, now(), &zone, Clock::H12),
            "Tomorrow 7:00 PM"
        );
        assert_eq!(
            upcoming(tomorrow_evening, now(), &zone, Clock::H24),
            "Tomorrow 19:00"
        );
    }
}
