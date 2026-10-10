//! Putting a conversation off until later, and bringing it back.
//!
//! `Op::SetSnooze` has been in the domain since phase 1 and `Filter::Snoozed`/`SnoozeDue` have
//! been answerable by the store for as long — and nothing could set one. `op_for` returns `None`
//! for `OpKind::Snooze` because the op needs a payload, and no surface supplied one, so the
//! whole apparatus sat there unused.
//!
//! Nothing runs when a snooze expires and nothing needs to. `SnoozeDue` is resolved against
//! `now` at query time, so a conversation returns to the inbox on the stroke whether the client
//! was running or not — which is the right design for something that may be asleep for a week.

use crate::error::{CoreError, TimeError};
use chrono::{DateTime, Datelike, Local, TimeDelta, TimeZone, Timelike, Utc, Weekday};
use mail_domain::{
    AccountCaps, ArchiveMeans, ChangeId, Condstore, ExpungeMeans, FolderRoles, Message, MoveExt,
    Op, ServerLabels, ServerThreads, Snooze, Supported, Target, ThreadId, WatchMode,
};
use mail_store::{SqliteStore, Store};

/// Put a conversation off until `phrase` names, and say when that is.
pub fn snooze(
    store: &SqliteStore,
    thread: ThreadId,
    phrase: &str,
    now: DateTime<Utc>,
) -> Result<DateTime<Utc>, CoreError> {
    let at = snooze_until(phrase, now, &Local)?;
    set(store, thread, Snooze::Until(at), now)?;
    Ok(at)
}

/// Bring one back now.
pub fn wake(store: &SqliteStore, thread: ThreadId, now: DateTime<Utc>) -> Result<(), CoreError> {
    set(store, thread, Snooze::Inactive, now)
}

/// Pin a conversation, or unpin it if it is already pinned. Says whether it is pinned now.
///
/// Here rather than in a module of its own because pin and snooze are the same shape: thread
/// level, local by construction, and needing a payload that `op_for` cannot supply.
pub fn pin(store: &SqliteStore, thread: ThreadId, now: DateTime<Utc>) -> Result<bool, CoreError> {
    let loaded = store.thread(thread)?;
    let op = crate::place::pin_op(&loaded.summary, now);
    let pinned = matches!(op, Op::SetPin(mail_domain::Pin::Rank(_)));
    apply(store, thread, op, now)?;
    Ok(pinned)
}

/// Apply `Op::SetSnooze` to a thread.
///
/// Through `Op::apply` and `Store::apply` like every other operation, so the change is recorded
/// with its inverse and can be undone. Snooze has no server representation anywhere — it is
/// local by construction, which is why the capabilities below are the safe defaults rather than
/// anything read from an account.
fn set(
    store: &SqliteStore,
    thread: ThreadId,
    snooze: Snooze,
    now: DateTime<Utc>,
) -> Result<(), CoreError> {
    apply(store, thread, Op::SetSnooze(snooze), now)
}

/// Apply a thread-level op through `Op::apply` and `Store::apply`, like every other operation,
/// so the change is recorded with its inverse and can be undone.
pub(crate) fn apply(
    store: &SqliteStore,
    thread: ThreadId,
    op: Op,
    now: DateTime<Utc>,
) -> Result<(), CoreError> {
    let loaded = store.thread(thread)?;
    let messages: Vec<Message> = loaded
        .messages
        .iter()
        .filter_map(|id| store.message(*id).ok())
        .collect();
    let account = messages
        .first()
        .map(|m| m.account.clone())
        .ok_or(CoreError::EmptyConversation)?;

    let applied = op.apply(
        &Target::Threads(vec![thread]),
        &loaded,
        &messages,
        &local_only(now),
        now,
    );
    store.apply(account, &applied.forward)?;
    let _ = ChangeId::generate();
    Ok(())
}

/// Capabilities for an operation that never leaves this machine.
///
/// Snooze is local by construction: no protocol represents it, so what a server supports cannot
/// change the answer. Spelled out rather than read from the account because reading them would
/// suggest they matter.
pub(crate) fn local_only(now: DateTime<Utc>) -> AccountCaps {
    AccountCaps {
        labels: ServerLabels::LocalOnly,
        threads: ServerThreads::Jwz,
        watch: WatchMode::Poll {
            every: std::time::Duration::from_secs(300),
        },
        archive: ArchiveMeans::LocalOnly,
        folders: FolderRoles::default(),
        condstore: Condstore::Absent,
        move_ext: MoveExt::Absent,
        expunge: ExpungeMeans::Forbidden,
        top: Supported::Absent,
        pipelining: Supported::Absent,
        connections: Default::default(),
        observed_at: now,
    }
}

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
        // `Filter::SnoozeDue` would make due immediately — a button that appears to do nothing.
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
