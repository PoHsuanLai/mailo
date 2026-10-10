//! Saying an invitation's time and repetition in words.
//!
//! `mail-pim` reads a calendar object into values ([`When`], [`Repeats`]); the wording is a
//! front-end's, so it lives here, above the parser, where a translation can replace it.

use chrono::{NaiveDateTime, Offset, TimeZone, Weekday};
use mail_pim::ical::{EventZone, RepeatEnd, RepeatUnit, Repeats};
use mail_pim::{Unplaced, When};

/// A time said for the reader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhenShown {
    /// In the reader's zone.
    pub yours: String,
    /// In the organiser's zone, named, when it reads differently from `yours`.
    pub theirs: Option<String>,
}

const DAY: &str = "%a %-d %b %Y";

/// `when` said in the reader's zone, and in the organiser's when that reads differently.
pub fn show_when<Z: TimeZone>(when: &When, reader: &Z) -> WhenShown {
    match when {
        When::Unstated => WhenShown {
            yours: "no time given".to_owned(),
            theirs: None,
        },
        When::AllDay { first, last } => WhenShown {
            yours: if first == last {
                format!("{}, all day", first.format(DAY))
            } else {
                format!("{} – {}, all day", first.format(DAY), last.format(DAY))
            },
            theirs: None,
        },
        When::Timed { start, end, zone } => {
            let yours = span(
                start.with_timezone(reader).naive_local(),
                end.map(|e| e.with_timezone(reader).naive_local()),
            );
            let reader_offset = start.with_timezone(reader).offset().fix();
            let theirs = match zone {
                EventZone::Utc => None,
                EventZone::Iana(tz) => {
                    let at = start.with_timezone(tz);
                    (at.offset().fix() != reader_offset).then(|| {
                        let text = span(
                            at.naive_local(),
                            end.map(|e| e.with_timezone(tz).naive_local()),
                        );
                        format!("{text} ({})", tz.name())
                    })
                }
                EventZone::Rules { tzid, offset } => (*offset != reader_offset).then(|| {
                    let text = span(
                        start.with_timezone(offset).naive_local(),
                        end.map(|e| e.with_timezone(offset).naive_local()),
                    );
                    format!("{text} ({tzid})")
                }),
            };
            WhenShown { yours, theirs }
        }
        When::Floating { start, end, why } => WhenShown {
            yours: match why {
                Unplaced::AsWritten => {
                    format!("{} (local time, wherever you are)", span(*start, *end))
                }
                Unplaced::UnknownZone(tzid) => format!(
                    "{} (in time zone \"{tzid}\", which could not be identified)",
                    span(*start, *end)
                ),
            },
            theirs: None,
        },
    }
}

/// "Thu 1 Oct 2026, 09:00–10:00", or with both days when it ends on another.
fn span(start: NaiveDateTime, end: Option<NaiveDateTime>) -> String {
    let first = format!("{}, {}", start.format(DAY), start.format("%H:%M"));
    match end {
        None => first,
        Some(end) if end.date() == start.date() => format!("{first}–{}", end.format("%H:%M")),
        Some(end) => format!("{first} – {}, {}", end.format(DAY), end.format("%H:%M")),
    }
}

/// What is said of a rule that was not put into words.
pub const REPEATS_OTHERWISE: &str = "repeats (details in the invitation)";

/// A repetition in words: "every week on Monday and Wednesday, 10 times".
pub fn repeats_words(repeats: &Repeats) -> String {
    let Repeats::Every(rule) = repeats else {
        return REPEATS_OTHERWISE.to_owned();
    };
    let mut out = match (rule.unit, rule.interval) {
        (RepeatUnit::Day, 1) => "every day".to_owned(),
        (RepeatUnit::Day, n) => format!("every {n} days"),
        (RepeatUnit::Week, 1) => "every week".to_owned(),
        (RepeatUnit::Week, n) => format!("every {n} weeks"),
    };
    if !rule.days.is_empty() {
        let names: Vec<&str> = rule.days.iter().map(|d| day_name(*d)).collect();
        out.push_str(" on ");
        out.push_str(&list(&names));
    }
    match rule.end {
        RepeatEnd::Never => {}
        RepeatEnd::Count(1) => out.push_str(", once"),
        RepeatEnd::Count(n) => out.push_str(&format!(", {n} times")),
        RepeatEnd::Until(day) => out.push_str(&format!(", until {}", day.format(DAY))),
    }
    out
}

fn day_name(day: Weekday) -> &'static str {
    match day {
        Weekday::Mon => "Monday",
        Weekday::Tue => "Tuesday",
        Weekday::Wed => "Wednesday",
        Weekday::Thu => "Thursday",
        Weekday::Fri => "Friday",
        Weekday::Sat => "Saturday",
        Weekday::Sun => "Sunday",
    }
}

/// "a", "a and b", "a, b and c".
fn list(items: &[&str]) -> String {
    match items {
        [] => String::new(),
        [one] => (*one).to_owned(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, FixedOffset, NaiveDate, Utc};
    use mail_pim::ical::{Recurrence, resolve_iana};

    fn at(text: &str) -> DateTime<Utc> {
        text.parse().unwrap()
    }

    fn iana(name: &str) -> mail_pim::ical::EventZone {
        EventZone::Iana(resolve_iana(name).unwrap())
    }

    fn timed(zone: EventZone) -> When {
        When::Timed {
            start: at("2026-10-01T07:00:00Z"),
            end: Some(at("2026-10-01T08:00:00Z")),
            zone,
        }
    }

    #[test]
    fn a_time_written_in_utc_has_one_reading() {
        let when = When::Timed {
            start: at("2026-10-02T13:00:00Z"),
            end: Some(at("2026-10-02T14:00:00Z")),
            zone: EventZone::Utc,
        };
        let taipei = resolve_iana("Asia/Taipei").unwrap();
        let shown = show_when(&when, &taipei);
        assert_eq!(shown.yours, "Fri 2 Oct 2026, 21:00–22:00");
        assert_eq!(shown.theirs, None);
    }

    #[test]
    fn the_organisers_zone_is_named_when_it_reads_differently() {
        let when = timed(iana("Asia/Taipei"));
        let london = resolve_iana("Europe/London").unwrap();
        let shown = show_when(&when, &london);
        assert_eq!(shown.yours, "Thu 1 Oct 2026, 08:00–09:00");
        assert_eq!(
            shown.theirs.as_deref(),
            Some("Thu 1 Oct 2026, 15:00–16:00 (Asia/Taipei)")
        );
        // A reader in the organiser's zone is told once.
        let taipei = resolve_iana("Asia/Taipei").unwrap();
        let shown = show_when(&when, &taipei);
        assert_eq!(shown.yours, "Thu 1 Oct 2026, 15:00–16:00");
        assert_eq!(shown.theirs, None);
    }

    #[test]
    fn a_zone_known_by_its_rules_is_named_by_the_calendars_own_name() {
        let when = When::Timed {
            start: at("2026-07-10T16:00:00Z"),
            end: Some(at("2026-07-10T17:30:00Z")),
            zone: EventZone::Rules {
                tzid: "Customized Time Zone".to_owned(),
                offset: FixedOffset::west_opt(7 * 3600).unwrap(),
            },
        };
        let shown = show_when(&when, &Utc);
        assert_eq!(shown.yours, "Fri 10 Jul 2026, 16:00–17:30");
        assert_eq!(
            shown.theirs.as_deref(),
            Some("Fri 10 Jul 2026, 09:00–10:30 (Customized Time Zone)")
        );
    }

    #[test]
    fn all_day_events_and_missing_times_are_said_plainly() {
        let day = |d| NaiveDate::from_ymd_opt(2026, 10, d).unwrap();
        let two = When::AllDay {
            first: day(1),
            last: day(2),
        };
        assert_eq!(
            show_when(&two, &Utc).yours,
            "Thu 1 Oct 2026 – Fri 2 Oct 2026, all day"
        );
        let one = When::AllDay {
            first: day(1),
            last: day(1),
        };
        assert_eq!(show_when(&one, &Utc).yours, "Thu 1 Oct 2026, all day");
        assert_eq!(show_when(&When::Unstated, &Utc).yours, "no time given");
    }

    #[test]
    fn a_clock_reading_says_why_it_could_not_be_placed() {
        let start = NaiveDate::from_ymd_opt(2026, 10, 1)
            .unwrap()
            .and_hms_opt(9, 0, 0)
            .unwrap();
        let end = start + chrono::TimeDelta::hours(1);
        let as_written = When::Floating {
            start,
            end: Some(end),
            why: Unplaced::AsWritten,
        };
        assert_eq!(
            show_when(&as_written, &Utc).yours,
            "Thu 1 Oct 2026, 09:00–10:00 (local time, wherever you are)"
        );
        let unknown = When::Floating {
            start,
            end: None,
            why: Unplaced::UnknownZone("Nowhere/Bogus".to_owned()),
        };
        assert!(
            show_when(&unknown, &Utc)
                .yours
                .contains("which could not be identified")
        );
    }

    #[test]
    fn repetitions_are_said_in_words_and_others_are_not_guessed() {
        let day = NaiveDate::from_ymd_opt(2026, 11, 30).unwrap();
        let rule = |unit, interval, days: &[Weekday], end| {
            Repeats::Every(Recurrence {
                unit,
                interval,
                days: days.to_vec(),
                end,
            })
        };
        let cases = [
            (rule(RepeatUnit::Day, 1, &[], RepeatEnd::Never), "every day"),
            (
                rule(RepeatUnit::Day, 3, &[], RepeatEnd::Never),
                "every 3 days",
            ),
            (
                rule(RepeatUnit::Day, 1, &[], RepeatEnd::Count(1)),
                "every day, once",
            ),
            (
                rule(RepeatUnit::Day, 1, &[], RepeatEnd::Count(5)),
                "every day, 5 times",
            ),
            (
                rule(RepeatUnit::Week, 1, &[Weekday::Mon], RepeatEnd::Never),
                "every week on Monday",
            ),
            (
                rule(
                    RepeatUnit::Week,
                    2,
                    &[Weekday::Mon, Weekday::Thu],
                    RepeatEnd::Never,
                ),
                "every 2 weeks on Monday and Thursday",
            ),
            (
                rule(
                    RepeatUnit::Week,
                    1,
                    &[Weekday::Mon, Weekday::Wed, Weekday::Fri],
                    RepeatEnd::Until(day),
                ),
                "every week on Monday, Wednesday and Friday, until Mon 30 Nov 2026",
            ),
            (Repeats::Otherwise, REPEATS_OTHERWISE),
        ];
        for (repeats, expected) in cases {
            assert_eq!(repeats_words(&repeats), expected, "{repeats:?}");
        }
    }
}
