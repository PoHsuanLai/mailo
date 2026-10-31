//! Reading a typed time: the words people put off a conversation, a send or a reminder until.
//!
//! Resolved in the reader's zone against a `now` the caller supplies, so a call never depends on
//! when it runs. Putting a conversation off, and bringing it back, is `mail_core::snooze`.

use chrono::{DateTime, Datelike, TimeDelta, TimeZone, Timelike, Utc, Weekday};

/// When a conversation should come back.
///
/// The vocabulary is small on purpose. "Snooze until a quarter past four on the third Tuesday"
/// is a calendar; what a mail client needs is the four or five answers people actually give,
/// plus an exact date for the rest.
///
/// Resolved in the reader's zone and against a `now` the caller supplies — a function that reads
/// the clock decides its own test's answer, and one that assumes UTC sends "tomorrow morning" to
/// the middle of tonight for anyone east of Greenwich.
///
/// | phrase | means |
/// | --- | --- |
/// | `later` | three hours from now |
/// | `tonight` | 19:00 today, or tomorrow if that has passed |
/// | `tomorrow` | 09:00 tomorrow |
/// | `weekend` | 09:00 on the coming Saturday |
/// | `monday` … `sunday` | 09:00 on the next such day |
/// | `today 17:00`, `tomorrow 9`, `fri 17:00` | that day, at that hour |
/// | `+90m`, `+2h`, `+3d` | that much from now |
/// | `2026-09-25` | 09:00 that day |
/// | `2026-09-25 14:30` | exactly that |
pub fn snooze_until<Tz: TimeZone>(
    phrase: &str,
    now: DateTime<Utc>,
    zone: &Tz,
) -> Result<DateTime<Utc>, TimeError>
where
    Tz::Offset: std::fmt::Display,
{
    /// The hour a morning starts, for every phrase that means "a day" rather than "a time".
    const MORNING: u32 = 9;
    /// And an evening.
    const EVENING: u32 = 19;

    let here = now.with_timezone(zone);
    let phrase = phrase.trim().to_ascii_lowercase();
    let at = |day: chrono::NaiveDate, hour: u32| -> Result<DateTime<Utc>, TimeError> {
        let naive = day
            .and_hms_opt(hour, 0, 0)
            .ok_or(TimeError::NotAnHour { hour })?;
        // `earliest`: a local time can be skipped by a daylight-saving jump, in which case the
        // next valid instant is the honest answer rather than an error about clocks.
        zone.from_local_datetime(&naive)
            .earliest()
            .map(|t| t.with_timezone(&Utc))
            .ok_or(TimeError::NoLocalTime)
    };

    if let Some(rest) = phrase.strip_prefix('+') {
        return relative(rest, now);
    }
    let today = here.date_naive();
    match phrase.as_str() {
        "later" => return Ok(now + TimeDelta::try_hours(3).unwrap_or_default()),
        "tonight" | "evening" => {
            // If the evening has already gone, the next one is tomorrow's — not one in the past.
            return if here.hour() < EVENING {
                at(today, EVENING)
            } else {
                at(today.succ_opt().ok_or(TimeError::NoTomorrow)?, EVENING)
            };
        }
        "tomorrow" => return at(today.succ_opt().ok_or(TimeError::NoTomorrow)?, MORNING),
        "weekend" | "saturday" | "sat" => return at(next_weekday(today, Weekday::Sat), MORNING),
        _ => {}
    }
    if let Some(day) = weekday_named(&phrase) {
        return at(next_weekday(today, day), MORNING);
    }
    if let Ok(date) = chrono::NaiveDate::parse_from_str(&phrase, "%Y-%m-%d") {
        return at(date, MORNING);
    }
    if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(&phrase, "%Y-%m-%d %H:%M") {
        return zone
            .from_local_datetime(&naive)
            .earliest()
            .map(|t| t.with_timezone(&Utc))
            .ok_or(TimeError::NoLocalTime);
    }
    if let Some((day, clock)) = phrase.split_once(char::is_whitespace)
        && let Some((hour, minute)) = clock_of(clock.trim())
    {
        let date = match day {
            "today" => Some(today),
            "tomorrow" => today.succ_opt(),
            "weekend" | "saturday" | "sat" => Some(next_weekday(today, Weekday::Sat)),
            _ => weekday_named(day)
                .map(|named| next_weekday(today, named))
                .or_else(|| chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d").ok()),
        };
        if let Some(naive) = date.and_then(|date| date.and_hms_opt(hour, minute, 0)) {
            return zone
                .from_local_datetime(&naive)
                .earliest()
                .map(|t| t.with_timezone(&Utc))
                .ok_or(TimeError::NoLocalTime);
        }
    }
    Err(TimeError::Unknown { phrase })
}

/// `9`, `17`, `9:30`, `17:00`: an hour of the day, with its minutes when they are written.
fn clock_of(text: &str) -> Option<(u32, u32)> {
    let (hour, minute) = match text.split_once(':') {
        Some((hour, minute)) if minute.len() == 2 => (hour, minute),
        Some(_) => return None,
        None => (text, "00"),
    };
    let all_digits = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
    if !all_digits(hour) || hour.len() > 2 || !all_digits(minute) {
        return None;
    }
    let (hour, minute) = (hour.parse().ok()?, minute.parse().ok()?);
    (hour < 24 && minute < 60).then_some((hour, minute))
}

/// `90m`, `2h`, `3d` — the part after a `+`.
fn relative(rest: &str, now: DateTime<Utc>) -> Result<DateTime<Utc>, TimeError> {
    let rest = rest.trim();
    let (digits, unit) = rest.split_at(rest.len().saturating_sub(1));
    let count: i64 = digits.parse().map_err(|_| TimeError::NotACount {
        rest: rest.to_owned(),
    })?;
    if count <= 0 {
        return Err(TimeError::Backwards);
    }
    let delta = match unit {
        "m" => TimeDelta::try_minutes(count),
        "h" => TimeDelta::try_hours(count),
        "d" => TimeDelta::try_days(count),
        _ => {
            return Err(TimeError::BadUnit {
                unit: unit.to_owned(),
            });
        }
    }
    .ok_or_else(|| TimeError::TooLong {
        rest: rest.to_owned(),
    })?;
    Ok(now + delta)
}

fn weekday_named(name: &str) -> Option<Weekday> {
    Some(match name {
        "monday" | "mon" => Weekday::Mon,
        "tuesday" | "tue" => Weekday::Tue,
        "wednesday" | "wed" => Weekday::Wed,
        "thursday" | "thu" => Weekday::Thu,
        "friday" | "fri" => Weekday::Fri,
        "sunday" | "sun" => Weekday::Sun,
        _ => return None,
    })
}

/// The next `want` strictly after `from`. Never today: "monday" said on a Monday means the one
/// coming, not the hour that has already passed.
fn next_weekday(from: chrono::NaiveDate, want: Weekday) -> chrono::NaiveDate {
    let mut day = from;
    for _ in 0..7 {
        day = day.succ_opt().unwrap_or(day);
        if day.weekday() == want {
            return day;
        }
    }
    day
}

/// When a conversation comes back.

/// A time that was typed and could not be read, or cannot be used.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TimeError {
    #[error("{hour}:00 is not a time")]
    NotAnHour { hour: u32 },
    #[error("no tomorrow")]
    NoTomorrow,
    /// A daylight-saving jump skipped it, or the zone has no such instant.
    #[error("that local time does not exist")]
    NoLocalTime,
    #[error(
        "{phrase:?} is not a time I know. Try: later, tonight, tomorrow, tomorrow 9, weekend, \
         monday…sunday, fri 17:00, +2h, +3d, 2026-09-25, or \"2026-09-25 14:30\""
    )]
    Unknown { phrase: String },
    #[error("{rest:?} is not a number of minutes, hours or days")]
    NotACount { rest: String },
    #[error("a snooze goes forwards")]
    Backwards,
    #[error("{unit:?} is not m, h or d")]
    BadUnit { unit: String },
    #[error("{rest:?} is longer than a mail client can wait")]
    TooLong { rest: String },
    /// A reminder was asked for with no time given.
    #[error("Type a time: tomorrow 9, fri 17:00, +3d, 2026-10-02")]
    Blank,
    #[error("{stamp} has already passed. Pick a later time.")]
    Passed { stamp: String },
    #[error("{text:?} is not a date: write 2026-10-08 or 2026-10-08T09:00")]
    NotADate { text: String },
    #[error("{text} does not exist in this time zone")]
    NoSuchInstant { text: String },
}

impl From<TimeError> for String {
    fn from(error: TimeError) -> Self {
        error.to_string()
    }
}

#[cfg(test)]
mod snoozing {
    use super::*;

    fn taipei() -> chrono::FixedOffset {
        chrono::FixedOffset::east_opt(8 * 3600).unwrap()
    }

    /// Tuesday 2026-09-22, 14:00 in Taipei (06:00 UTC).
    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 22, 6, 0, 0).unwrap()
    }

    /// What a phrase resolves to, written in the reader's zone.
    fn until(phrase: &str) -> String {
        snooze_until(phrase, now(), &taipei())
            .map(|t| {
                t.with_timezone(&taipei())
                    .format("%a %Y-%m-%d %H:%M")
                    .to_string()
            })
            .unwrap_or_else(|e| format!("error: {e}"))
    }

    #[test]
    fn the_words_people_actually_use() {
        assert_eq!(until("later"), "Tue 2026-09-22 17:00");
        assert_eq!(until("tonight"), "Tue 2026-09-22 19:00");
        assert_eq!(until("tomorrow"), "Wed 2026-09-23 09:00");
        assert_eq!(until("weekend"), "Sat 2026-09-26 09:00");
        assert_eq!(until("friday"), "Fri 2026-09-25 09:00");
        // Case and surrounding space are not part of what was meant.
        assert_eq!(until("  ToMorrow "), "Wed 2026-09-23 09:00");
    }

    #[test]
    fn a_day_named_today_means_the_next_one_not_an_hour_that_has_passed() {
        // Said on a Tuesday afternoon, "tuesday" cannot mean this morning.
        assert_eq!(until("tuesday"), "Tue 2026-09-29 09:00");
    }

    #[test]
    fn tonight_after_the_evening_is_tomorrow_evening() {
        // 22:00 in Taipei, which is 14:00 UTC. The alternative is a snooze into the past, which
        // `Filter::SnoozeDue` would make due immediately — a snooze that appears to do nothing.
        let late = Utc.with_ymd_and_hms(2026, 9, 22, 14, 0, 0).unwrap();
        let got = snooze_until("tonight", late, &taipei()).unwrap();
        assert_eq!(
            got.with_timezone(&taipei())
                .format("%a %Y-%m-%d %H:%M")
                .to_string(),
            "Wed 2026-09-23 19:00"
        );
        assert!(got > late, "a snooze must be in the future");
    }

    #[test]
    fn offsets_and_dates() {
        assert_eq!(until("+90m"), "Tue 2026-09-22 15:30");
        assert_eq!(until("+2h"), "Tue 2026-09-22 16:00");
        assert_eq!(until("+3d"), "Fri 2026-09-25 14:00");
        assert_eq!(until("2026-12-25"), "Fri 2026-12-25 09:00");
        assert_eq!(until("2026-12-25 14:30"), "Fri 2026-12-25 14:30");
    }

    #[test]
    fn a_date_is_read_in_the_readers_zone_not_in_utc() {
        // 09:00 on Christmas morning in Taipei is 01:00 UTC. Read as UTC it would land at
        // 17:00 local — the afternoon of a day the user said "morning" about.
        let at = snooze_until("2026-12-25", now(), &taipei()).unwrap();
        assert_eq!(at.format("%Y-%m-%d %H:%M").to_string(), "2026-12-25 01:00");
        let in_utc = snooze_until("2026-12-25", now(), &Utc).unwrap();
        assert_eq!(
            in_utc.format("%Y-%m-%d %H:%M").to_string(),
            "2026-12-25 09:00"
        );
    }

    #[test]
    fn everything_it_returns_is_in_the_future() {
        // The property that matters: a snooze into the past is due the instant it is made, so
        // the conversation never leaves the inbox and the feature silently does nothing.
        for phrase in [
            "later",
            "tonight",
            "tomorrow",
            "weekend",
            "monday",
            "tuesday",
            "wednesday",
            "thursday",
            "friday",
            "saturday",
            "sunday",
            "+1m",
            "+1h",
            "+1d",
        ] {
            let at =
                snooze_until(phrase, now(), &taipei()).unwrap_or_else(|e| panic!("{phrase}: {e}"));
            assert!(at > now(), "{phrase} resolved to {at}, which is not later");
        }
    }

    #[test]
    fn a_phrase_it_does_not_know_says_what_it_does() {
        let why = snooze_until("next fortnight", now(), &taipei())
            .unwrap_err()
            .to_string();
        assert!(why.contains("tomorrow") && why.contains("+2h"), "{why}");
        // Nonsense that looks like an offset, too.
        assert!(snooze_until("+", now(), &taipei()).is_err());
        assert!(snooze_until("+2y", now(), &taipei()).is_err());
        assert!(
            snooze_until("+0h", now(), &taipei()).is_err(),
            "zero is not forwards"
        );
        assert!(
            snooze_until("-2h", now(), &taipei()).is_err(),
            "nor is backwards"
        );
        assert!(snooze_until("", now(), &taipei()).is_err());
    }
}
