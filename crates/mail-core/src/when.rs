//! Writing an instant the way the person reading it keeps time: the list's column, the
//! reader's header, the attribution line above quoted text.

use chrono::{DateTime, TimeZone, Utc};

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
}

impl Stamp {
    fn pattern(self) -> &'static str {
        match self {
            Stamp::Row => "%m-%d %H:%M",
            Stamp::Full => "%Y-%m-%d %H:%M",
            Stamp::Quote => "%a, %d %b %Y at %H:%M",
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
pub fn listed<Tz: TimeZone>(instant: DateTime<Utc>, now: DateTime<Utc>, zone: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let then = instant.with_timezone(zone);
    let today = now.with_timezone(zone);
    // Calendar days, not elapsed hours: 23:50 yesterday is not "today" because it is within
    // twenty-four hours, and 00:10 this morning is.
    let days = today
        .date_naive()
        .signed_duration_since(then.date_naive())
        .num_days();
    if days == 0 {
        then.format("%H:%M").to_string()
    } else if (1..7).contains(&days) {
        then.format("%a").to_string()
    } else if then.format("%Y").to_string() == today.format("%Y").to_string() {
        then.format("%b %d").to_string()
    } else {
        // A year old. The year is the only part that still says anything.
        then.format("%Y-%m-%d").to_string()
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
pub fn stamp<Tz: TimeZone>(instant: DateTime<Utc>, zone: &Tz, stamp: Stamp) -> String
where
    Tz::Offset: std::fmt::Display,
{
    instant
        .with_timezone(zone)
        .format(stamp.pattern())
        .to_string()
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
        assert_eq!(stamp(morning(), &taipei(), Stamp::Row), "09-22 09:02");
        assert_eq!(stamp(morning(), &taipei(), Stamp::Full), "2026-09-22 09:02");
    }

    #[test]
    fn an_evening_message_is_not_filed_under_tomorrow() {
        // 23:30 UTC is 07:30 the next morning in Taipei. Shown as UTC, every message that
        // arrived after 16:00 local carried the previous day's date — which is precisely the
        // state the application was in when this was found: the clock read the 22nd and every
        // row said the 21st.
        let evening = Utc.with_ymd_and_hms(2026, 9, 21, 23, 30, 0).unwrap();
        assert_eq!(stamp(evening, &taipei(), Stamp::Row), "09-22 07:30");
        assert_eq!(stamp(evening, &Utc, Stamp::Row), "09-21 23:30");
    }

    #[test]
    fn a_zone_behind_utc_rolls_the_other_way() {
        // Not "add eight hours somewhere". New York is five behind, so the same instant is the
        // day before, and a client that only ever shifted forward would be wrong here.
        let newyork = FixedOffset::west_opt(5 * 3600).unwrap();
        let just_after_midnight = Utc.with_ymd_and_hms(2026, 9, 22, 3, 15, 0).unwrap();
        assert_eq!(
            stamp(just_after_midnight, &newyork, Stamp::Row),
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
        assert_eq!(listed(morning(), now(), &zone), "09:02");
        // Yesterday evening in Taipei, which is a different calendar day and so not a time.
        let yesterday = Utc.with_ymd_and_hms(2026, 9, 21, 12, 0, 0).unwrap();
        assert_eq!(listed(yesterday, now(), &zone), "Mon");
        // Three weeks: past the weekday window, inside the year.
        let weeks_ago = Utc.with_ymd_and_hms(2026, 9, 1, 2, 0, 0).unwrap();
        assert_eq!(listed(weeks_ago, now(), &zone), "Sep 01");
        // Last year, where the year is the only part still worth printing.
        let old = Utc.with_ymd_and_hms(2024, 11, 14, 22, 13, 0).unwrap();
        assert_eq!(listed(old, now(), &zone), "2024-11-15");
    }

    #[test]
    fn today_is_a_calendar_day_not_the_last_twenty_four_hours() {
        // 00:10 this morning is today although it is fourteen hours ago; 23:50 last night is
        // not, although it is fourteen hours ago too. Elapsed-hours arithmetic gets both wrong.
        let zone = taipei();
        let just_after_midnight = Utc.with_ymd_and_hms(2026, 9, 21, 16, 10, 0).unwrap();
        assert_eq!(listed(just_after_midnight, now(), &zone), "00:10");
        let late_last_night = Utc.with_ymd_and_hms(2026, 9, 21, 15, 50, 0).unwrap();
        assert_eq!(listed(late_last_night, now(), &zone), "Mon");
    }

    #[test]
    fn the_list_stamp_is_in_the_readers_zone_too() {
        // The same instant is today in Taipei and yesterday in New York, so the two disagree
        // about which shape to use at all — not just about the digits.
        let newyork = chrono::FixedOffset::west_opt(5 * 3600).unwrap();
        assert_eq!(listed(morning(), now(), &taipei()), "09:02");
        assert_eq!(listed(morning(), now(), &newyork), "Mon");
    }

    #[test]
    fn utc_is_left_alone() {
        // Someone whose machine is on UTC must see what was stored — this is the case that
        // made the bug invisible, because it is the one every test machine is in.
        assert_eq!(stamp(morning(), &Utc, Stamp::Full), "2026-09-22 01:02");
    }
}
