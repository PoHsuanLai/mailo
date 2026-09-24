//! `RRULE`s, in words.
//!
//! An invitation is shown, not expanded, so a rule only has to be said. The common ones are
//! said exactly — every day, every two weeks on Monday and Thursday, ten times, until a date —
//! and everything else (monthly by position, yearly, by hour, by set position) is said to repeat,
//! with the details left to the invitation, rather than said approximately and wrongly.

use chrono::{NaiveDate, Weekday};

/// What is said of a rule this module will not put into words.
pub const REPEATS_OTHERWISE: &str = "repeats (details in the invitation)";

/// `rule` in words: "every week on Monday and Wednesday, 10 times".
pub fn describe(rule: &str) -> String {
    said(rule).unwrap_or_else(|| REPEATS_OTHERWISE.to_owned())
}

fn said(rule: &str) -> Option<String> {
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
    let mut out = match (freq?.as_str(), interval) {
        ("DAILY", 1) if days.is_empty() => "every day".to_owned(),
        ("DAILY", n) if days.is_empty() => format!("every {n} days"),
        ("WEEKLY", 1) => "every week".to_owned(),
        ("WEEKLY", n) => format!("every {n} weeks"),
        _ => return None,
    };
    if !days.is_empty() {
        let names: Vec<&str> = days.iter().map(|d| day_name(*d)).collect();
        out.push_str(" on ");
        out.push_str(&list(&names));
    }
    match (count, until) {
        (Some(1), None) => out.push_str(", once"),
        (Some(n), None) => out.push_str(&format!(", {n} times")),
        (None, Some(day)) => out.push_str(&format!(", until {}", day.format("%a %-d %b %Y"))),
        (None, None) => {}
        // Both is forbidden (RFC 5545 §3.3.10); saying either would be a guess.
        (Some(_), Some(_)) => return None,
    }
    Some(out)
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

/// The day an `UNTIL` falls on, as written: a date, or the date part of a date-time.
fn until_day(value: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(value.get(..8)?, "%Y%m%d").ok()
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

    #[test]
    fn common_rules_are_said_in_words_and_others_are_not_guessed() {
        const CASES: &[(&str, &str)] = &[
            ("FREQ=DAILY", "every day"),
            ("FREQ=DAILY;INTERVAL=3", "every 3 days"),
            ("FREQ=DAILY;COUNT=5", "every day, 5 times"),
            ("FREQ=DAILY;COUNT=1", "every day, once"),
            ("FREQ=WEEKLY", "every week"),
            ("FREQ=WEEKLY;BYDAY=MO", "every week on Monday"),
            (
                "FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,TH;WKST=SU",
                "every 2 weeks on Monday and Thursday",
            ),
            (
                "FREQ=WEEKLY;BYDAY=MO,WE,FR;UNTIL=20261130T235959Z",
                "every week on Monday, Wednesday and Friday, until Mon 30 Nov 2026",
            ),
            (
                "freq=weekly;byday=tu;count=10",
                "every week on Tuesday, 10 times",
            ),
            ("FREQ=MONTHLY;BYDAY=2MO", REPEATS_OTHERWISE),
            ("FREQ=YEARLY", REPEATS_OTHERWISE),
            ("FREQ=WEEKLY;BYDAY=1MO", REPEATS_OTHERWISE),
            ("FREQ=DAILY;BYHOUR=9,17", REPEATS_OTHERWISE),
            ("FREQ=DAILY;BYDAY=MO", REPEATS_OTHERWISE),
            ("FREQ=DAILY;COUNT=3;UNTIL=20261130", REPEATS_OTHERWISE),
            ("FREQ=DAILY;INTERVAL=0", REPEATS_OTHERWISE),
            ("", REPEATS_OTHERWISE),
            ("garbage", REPEATS_OTHERWISE),
        ];
        for (rule, expected) in CASES {
            assert_eq!(describe(rule), *expected, "{rule}");
        }
    }
}
