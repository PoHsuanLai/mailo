//! What a reminder to follow up means, apart from the store that keeps it: the times it offers, when
//! a typed one comes due, which conversations a list narrowed to a scope holds, and how a returned
//! reminder reads. Sweeping the reminders is `mail_core::follow_up`.

use crate::snooze::TimeError;
use chrono::{DateTime, TimeDelta, TimeZone, Utc};
use mail_domain::{Filter, FollowUp, MatchCtx, Op, ThreadId, ThreadSummary};

/// The times a reminder offers by name, as `(key, what it says)`. A typed time is the rest.
pub const CHOICES: [(&str, &str); 3] = [
    ("tomorrow", "Tomorrow morning"),
    ("three-days", "In 3 days"),
    ("next-week", "Next week"),
];

/// The instant a reminder chosen as `choice` comes due, counted from `from` in `zone`: one of
/// [`CHOICES`]' keys, or a time typed the way snooze reads one (`fri 17:00`, `+2h`,
/// `2026-10-02`). A time that is not after `from` is refused, in words (its hour as `clock`
/// writes it): a reminder for a moment already gone would come back before anyone could answer.
pub fn due<Tz: TimeZone>(
    choice: &str,
    from: DateTime<Utc>,
    zone: &Tz,
    clock: crate::when::Clock,
) -> Result<DateTime<Utc>, TimeError>
where
    Tz::Offset: std::fmt::Display,
{
    let phrase = match choice {
        "tomorrow" => "tomorrow".to_owned(),
        // Three mornings on: the day someone can be expected to have answered by.
        "three-days" => {
            let day = from.with_timezone(zone).date_naive() + TimeDelta::days(3);
            day.format("%Y-%m-%d").to_string()
        }
        "next-week" => "monday".to_owned(),
        typed => typed.trim().to_owned(),
    };
    if phrase.is_empty() {
        return Err(TimeError::Blank);
    }
    let at = crate::snooze::snooze_until(&phrase, from, zone)?;
    if at <= from {
        return Err(TimeError::Passed {
            stamp: crate::when::stamp(at, zone, crate::when::Stamp::Full, clock),
        });
    }
    Ok(at)
}

/// Whether a conversation belongs where the list is narrowed to.
pub fn in_scope(summary: &ThreadSummary, scope: Option<&Filter>, now: DateTime<Utc>) -> bool {
    scope.is_none_or(|filter| {
        filter.fit(&MatchCtx {
            summary,
            corpus: None,
            folders: &[],
            sizes: &[],
            cc: &[],
            vips: &[],
            now,
        })
    })
}

/// `top` above `rows`, each conversation once.
pub fn on_top(top: Vec<ThreadSummary>, rows: Vec<ThreadSummary>) -> Vec<ThreadSummary> {
    let ids: Vec<ThreadId> = top.iter().map(|summary| summary.id).collect();
    top.into_iter()
        .chain(
            rows.into_iter()
                .filter(|summary| !ids.contains(&summary.id)),
        )
        .collect()
}

/// What a row says about its reminder, when it says anything: the one words a returned
/// conversation shows, beside nothing for a reminder still waiting.
pub fn returned_words(follow_up: &FollowUp) -> Option<&'static str> {
    match follow_up {
        FollowUp::Returned { .. } => Some("No reply yet"),
        FollowUp::Until { .. } | FollowUp::Inactive => None,
    }
}

/// The op a menu applies to set a reminder at `at`, asked for at `set`.
pub fn remind(at: DateTime<Utc>, set: DateTime<Utc>) -> Op {
    Op::SetFollowUp(FollowUp::Until { at, set })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Noon on a Monday, 2026-09-28, and whole days from it.
    fn day(n: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 28, 12, 0, 0).unwrap() + TimeDelta::days(n)
    }

    #[test]
    fn the_named_times_and_a_typed_one_resolve_from_when_it_leaves() {
        // Monday noon in UTC.
        let from = day(0);
        let cases: [(&str, Result<DateTime<Utc>, ()>); 7] = [
            (
                "tomorrow",
                Ok(Utc.with_ymd_and_hms(2026, 9, 29, 9, 0, 0).unwrap()),
            ),
            (
                "three-days",
                Ok(Utc.with_ymd_and_hms(2026, 10, 1, 9, 0, 0).unwrap()),
            ),
            (
                "next-week",
                Ok(Utc.with_ymd_and_hms(2026, 10, 5, 9, 0, 0).unwrap()),
            ),
            ("+2h", Ok(from + TimeDelta::hours(2))),
            (
                "2026-10-02 17:30",
                Ok(Utc.with_ymd_and_hms(2026, 10, 2, 17, 30, 0).unwrap()),
            ),
            // Gone already, and not a time at all.
            ("2026-09-01", Err(())),
            ("", Err(())),
        ];
        for (choice, want) in cases {
            let got = due(choice, from, &Utc, crate::when::Clock::H24);
            match want {
                Ok(at) => assert_eq!(got, Ok(at), "{choice:?}"),
                Err(()) => assert!(got.is_err(), "{choice:?} should be refused: {got:?}"),
            }
        }
    }
}
