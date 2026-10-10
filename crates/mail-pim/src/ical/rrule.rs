//! `RRULE`s, reduced to the shapes a reader can be told.
//!
//! An invitation is shown, not expanded, so a rule only has to be understood well enough to be
//! said. The common ones are read exactly — every day, every two weeks on Monday and Thursday,
//! ten times, until a date — and everything else (monthly by position, yearly, by hour, by set
//! position) is [`Repeats::Otherwise`], with the details left to the invitation, rather than
//! read approximately and wrongly. The words for either are the caller's.

use chrono::{NaiveDate, Weekday};

/// What an `RRULE` comes to, for the reader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Repeats {
    /// One of the common shapes, read exactly.
    Every(Recurrence),
    /// It repeats, in a way this does not put into words: anything else, and anything
    /// malformed or contradictory.
    Otherwise,
}

/// A rule of the common shapes: every `interval` days, or weeks on certain days.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recurrence {
    pub unit: RepeatUnit,
    /// Always at least 1.
    pub interval: u32,
    /// `BYDAY`, in the order written. Empty for a daily rule, which has none.
    pub days: Vec<Weekday>,
    pub end: RepeatEnd,
}

/// What a rule counts in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepeatUnit {
    Day,
    Week,
}

/// When a rule stops.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepeatEnd {
    /// Neither `COUNT` nor `UNTIL`.
    Never,
    /// `COUNT`: how many times in all.
    Count(u32),
    /// `UNTIL`, as the day it falls on.
    Until(NaiveDate),
}

impl Repeats {
    /// Read `rule`, the value of an `RRULE`.
    pub fn read(rule: &str) -> Repeats {
        recurrence(rule).map_or(Repeats::Otherwise, Repeats::Every)
    }
}

fn recurrence(rule: &str) -> Option<Recurrence> {
    let parts = parts(rule);
    let mut freq = None;
    let mut interval = 1u32;
    let mut days = Vec::new();
    let mut count = None;
    let mut until = None;
    for (key, value) in &parts {
        match key.as_str() {
            "FREQ" => freq = Some(value.to_ascii_uppercase()),
            "INTERVAL" => interval = value.parse().ok().filter(|n| *n > 0)?,
            "BYDAY" => {
                for code in value.split(',') {
                    // An ordinal (`2MO`) belongs to a monthly or yearly rule.
                    days.push(weekday(code.trim())?);
                }
            }
            "COUNT" => count = Some(value.parse::<u32>().ok()?),
            "UNTIL" => until = Some(until_day(value)?),
            // The week start changes nothing about a weekly rule with at most one week between.
            "WKST" => {}
            _ => return None,
        }
    }
    let unit = match freq?.as_str() {
        "DAILY" if days.is_empty() => RepeatUnit::Day,
        "WEEKLY" => RepeatUnit::Week,
        _ => return None,
    };
    let end = match (count, until) {
        (Some(n), None) => RepeatEnd::Count(n),
        (None, Some(day)) => RepeatEnd::Until(day),
        (None, None) => RepeatEnd::Never,
        // Both is forbidden (RFC 5545 §3.3.10); reading either would be a guess.
        (Some(_), Some(_)) => return None,
    };
    Some(Recurrence {
        unit,
        interval,
        days,
        end,
    })
}

/// `KEY=value` pairs of a rule, keys upper-cased, in order.
pub(super) fn parts(rule: &str) -> Vec<(String, String)> {
    rule.split(';')
        .filter_map(|part| part.split_once('='))
        .map(|(k, v)| (k.trim().to_ascii_uppercase(), v.trim().to_owned()))
        .collect()
}

/// A two-letter weekday code.
pub(super) fn weekday(code: &str) -> Option<Weekday> {
    Some(match code.to_ascii_uppercase().as_str() {
        "MO" => Weekday::Mon,
        "TU" => Weekday::Tue,
        "WE" => Weekday::Wed,
        "TH" => Weekday::Thu,
        "FR" => Weekday::Fri,
        "SA" => Weekday::Sat,
        "SU" => Weekday::Sun,
        _ => return None,
    })
}

/// The day an `UNTIL` falls on, as written: a date, or the date part of a date-time.
fn until_day(value: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(value.get(..8)?, "%Y%m%d").ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn every(unit: RepeatUnit, interval: u32, days: &[Weekday], end: RepeatEnd) -> Repeats {
        Repeats::Every(Recurrence {
            unit,
            interval,
            days: days.to_vec(),
            end,
        })
    }

    #[test]
    fn common_rules_are_read_and_others_are_not_guessed() {
        let day = |y, m, d| NaiveDate::from_ymd_opt(y, m, d).unwrap();
        let cases: Vec<(&str, Repeats)> = vec![
            (
                "FREQ=DAILY",
                every(RepeatUnit::Day, 1, &[], RepeatEnd::Never),
            ),
            (
                "FREQ=DAILY;INTERVAL=3",
                every(RepeatUnit::Day, 3, &[], RepeatEnd::Never),
            ),
            (
                "FREQ=DAILY;COUNT=5",
                every(RepeatUnit::Day, 1, &[], RepeatEnd::Count(5)),
            ),
            (
                "FREQ=WEEKLY;BYDAY=MO",
                every(RepeatUnit::Week, 1, &[Weekday::Mon], RepeatEnd::Never),
            ),
            (
                "FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,TH;WKST=SU",
                every(
                    RepeatUnit::Week,
                    2,
                    &[Weekday::Mon, Weekday::Thu],
                    RepeatEnd::Never,
                ),
            ),
            (
                "freq=weekly;byday=tu;count=10",
                every(RepeatUnit::Week, 1, &[Weekday::Tue], RepeatEnd::Count(10)),
            ),
            (
                "FREQ=WEEKLY;INTERVAL=2;BYDAY=TU;UNTIL=20261215",
                every(
                    RepeatUnit::Week,
                    2,
                    &[Weekday::Tue],
                    RepeatEnd::Until(day(2026, 12, 15)),
                ),
            ),
            ("FREQ=MONTHLY;BYDAY=2MO", Repeats::Otherwise),
            ("FREQ=YEARLY", Repeats::Otherwise),
            ("FREQ=WEEKLY;BYDAY=1MO", Repeats::Otherwise),
            ("FREQ=DAILY;BYHOUR=9,17", Repeats::Otherwise),
            ("FREQ=DAILY;BYDAY=MO", Repeats::Otherwise),
            ("FREQ=DAILY;COUNT=3;UNTIL=20261130", Repeats::Otherwise),
            ("FREQ=DAILY;INTERVAL=0", Repeats::Otherwise),
            ("", Repeats::Otherwise),
            ("garbage", Repeats::Otherwise),
        ];
        for (rule, expected) in cases {
            assert_eq!(Repeats::read(rule), expected, "{rule}");
        }
    }
}
