//! Where a time written in an invitation falls: `TZID` to a zone, and a clock reading in it to
//! an instant.
//!
//! Most senders name an IANA zone (`Europe/Berlin`). Outlook and Exchange name Windows zones
//! (`W. Europe Standard Time`), mapped through CLDR's table in [`super::windows`]; some programs
//! prefix an IANA name with a path of their own (`/example.org/20260101_1/Europe/Berlin`). A
//! name none of those recognise is looked up among the calendar's own `VTIMEZONE`s, whose
//! `STANDARD` and `DAYLIGHT` rules are evaluated here — which is what a sender's invented name
//! (`Customized Time Zone`) relies on. A zone found nowhere leaves the time as a clock reading,
//! said to be one, rather than guessed at.

use super::rrule::{parts, weekday};
use super::windows::WINDOWS_ZONES;
use super::{Moment, Observance, ZoneRules};
use chrono::{
    DateTime, Datelike, FixedOffset, LocalResult, NaiveDate, NaiveDateTime, TimeDelta, TimeZone,
    Utc, Weekday,
};
use chrono_tz::Tz;

/// The zone an event's times were written in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventZone {
    /// Written in UTC, which says nothing about where the organiser is.
    Utc,
    /// An IANA zone, found by name.
    Iana(Tz),
    /// A zone known only by the calendar's own rules for it, and the offset they give at the
    /// time in question.
    Rules { tzid: String, offset: FixedOffset },
}

/// A [`Moment`] placed on the time line, as far as it can be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Placed {
    /// A whole day, in no zone.
    Day(NaiveDate),
    At {
        utc: DateTime<Utc>,
        zone: EventZone,
    },
    /// A clock reading meant to be read in whatever zone the reader is in.
    Floating(NaiveDateTime),
    /// A clock reading in a zone nothing could identify.
    UnknownZone {
        local: NaiveDateTime,
        tzid: String,
    },
}

/// Place `moment`, using the calendar's `zones` for any `TZID` that is not a known name.
pub fn place(moment: &Moment, zones: &[ZoneRules]) -> Placed {
    match moment {
        Moment::Date(day) => Placed::Day(*day),
        Moment::Utc(utc) => Placed::At {
            utc: *utc,
            zone: EventZone::Utc,
        },
        Moment::Floating(local) => Placed::Floating(*local),
        Moment::Zoned { local, tzid } => {
            if let Some(tz) = resolve_iana(tzid) {
                return Placed::At {
                    utc: from_local(&tz, *local),
                    zone: EventZone::Iana(tz),
                };
            }
            let rules = zones.iter().find(|z| z.tzid == *tzid);
            match rules.and_then(|rules| offset_at(rules, *local)) {
                Some(offset) => Placed::At {
                    utc: (*local - TimeDelta::seconds(i64::from(offset.local_minus_utc())))
                        .and_utc(),
                    zone: EventZone::Rules {
                        tzid: tzid.clone(),
                        offset,
                    },
                },
                None => Placed::UnknownZone {
                    local: *local,
                    tzid: tzid.clone(),
                },
            }
        }
    }
}

/// The IANA zone a `TZID` names: an IANA name, a Windows name, or an IANA name at the end of a
/// sender's path prefix.
pub fn resolve_iana(tzid: &str) -> Option<Tz> {
    let tzid = tzid.trim();
    if tzid.is_empty() || tzid.len() > 200 {
        return None;
    }
    if let Ok(tz) = tzid.parse::<Tz>() {
        return Some(tz);
    }
    if let Some(iana) = windows(tzid) {
        return iana.parse().ok();
    }
    // `/example.org/20260101_1/America/New_York`: the longest tail that is a zone name.
    let segments: Vec<&str> = tzid.split('/').filter(|s| !s.is_empty()).collect();
    (1..segments.len())
        .filter_map(|skip| segments[skip..].join("/").parse::<Tz>().ok())
        .next()
}

fn windows(name: &str) -> Option<&'static str> {
    WINDOWS_ZONES
        .binary_search_by(|(windows, _)| (*windows).cmp(name))
        .ok()
        .map(|i| WINDOWS_ZONES[i].1)
        .or_else(|| {
            WINDOWS_ZONES
                .iter()
                .find(|(windows, _)| windows.eq_ignore_ascii_case(name))
                .map(|(_, iana)| *iana)
        })
}

/// A clock reading in `tz` as an instant. In an autumn overlap the earlier of the two; in a
/// spring gap, the reading an hour on — what a clock that skipped the hour would have shown.
fn from_local(tz: &Tz, local: NaiveDateTime) -> DateTime<Utc> {
    let found = match tz.from_local_datetime(&local) {
        LocalResult::None => tz.from_local_datetime(&(local + TimeDelta::hours(1))),
        other => other,
    };
    match found {
        LocalResult::Single(at) | LocalResult::Ambiguous(at, _) => at.with_timezone(&Utc),
        // Only a zone with a gap longer than an hour at this reading; read it as UTC rather
        // than invent an offset.
        LocalResult::None => local.and_utc(),
    }
}

/// The offset `rules` give at the clock reading `local`: that of the latest onset at or before
/// it. Before every onset, the first observance's offset before it.
pub(super) fn offset_at(rules: &ZoneRules, local: NaiveDateTime) -> Option<FixedOffset> {
    let latest = rules
        .observances
        .iter()
        .flat_map(|o| onsets_near(o, local.year()).map(move |onset| (onset, o.offset_to)))
        .filter(|(onset, _)| *onset <= local)
        .max_by_key(|(onset, _)| *onset);
    match latest {
        Some((_, offset)) => Some(offset),
        None => rules
            .observances
            .iter()
            .min_by_key(|o| o.start)
            .map(|o| o.offset_from),
    }
}

/// The observance's onsets in `year` and the year before: enough to find the latest one before
/// any reading in `year`.
fn onsets_near(o: &Observance, year: i32) -> impl Iterator<Item = NaiveDateTime> + '_ {
    let ruled = o
        .rule
        .as_deref()
        .into_iter()
        .flat_map(move |rule| [year - 1, year].map(|y| yearly_onset(rule, o.start, y)))
        .flatten();
    std::iter::once(o.start)
        .chain(o.dates.iter().copied())
        .chain(ruled)
}

/// Where a `FREQ=YEARLY` onset rule puts the onset in `year`, as VTIMEZONEs write them:
/// `BYMONTH` with an ordinal `BYDAY` (`2SU`, `-1SU`), or a plain `BYDAY` narrowed by
/// `BYMONTHDAY` (the older "first Sunday on or after the 8th"). Other rules give nothing, and
/// the observance's own start and dates stand.
fn yearly_onset(rule: &str, start: NaiveDateTime, year: i32) -> Option<NaiveDateTime> {
    if year < start.year() {
        return None;
    }
    let parts = parts(rule);
    let get = |key: &str| {
        parts
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    };
    if !get("FREQ")?.eq_ignore_ascii_case("YEARLY") {
        return None;
    }
    let month = match get("BYMONTH") {
        Some(m) => m.split(',').next()?.trim().parse().ok()?,
        None => start.month(),
    };
    let day = match get("BYDAY") {
        None => NaiveDate::from_ymd_opt(year, month, start.day())?,
        Some(byday) => {
            let byday = byday.split(',').next()?.trim();
            let split = byday.len().checked_sub(2)?;
            let (ordinal, code) = byday.split_at_checked(split)?;
            let weekday = weekday(code)?;
            match ordinal {
                "" | "+" => {
                    let days: Vec<u32> = get("BYMONTHDAY")?
                        .split(',')
                        .filter_map(|d| d.trim().parse().ok())
                        .collect();
                    days.into_iter()
                        .filter_map(|d| NaiveDate::from_ymd_opt(year, month, d))
                        .find(|d| d.weekday() == weekday)?
                }
                n => nth_weekday(year, month, weekday, n.parse().ok()?)?,
            }
        }
    };
    let onset = day.and_time(start.time());
    let until = get("UNTIL").and_then(|u| {
        super::read::moment_value(u, false, None).and_then(|m| match m {
            Moment::Utc(at) => Some(at.naive_utc()),
            Moment::Floating(local) => Some(local),
            Moment::Date(day) => day.and_hms_opt(23, 59, 59),
            Moment::Zoned { .. } => None,
        })
    });
    (onset >= start && until.is_none_or(|until| onset <= until)).then_some(onset)
}

/// The `n`th `weekday` of a month, counting from the end when `n` is negative.
fn nth_weekday(year: i32, month: u32, weekday: Weekday, n: i32) -> Option<NaiveDate> {
    match n {
        1..=5 => NaiveDate::from_weekday_of_month_opt(year, month, weekday, n as u8),
        -5..=-1 => {
            let next = if month == 12 {
                NaiveDate::from_ymd_opt(year + 1, 1, 1)?
            } else {
                NaiveDate::from_ymd_opt(year, month + 1, 1)?
            };
            let last = next.pred_opt()?;
            let back =
                (last.weekday().num_days_from_monday() + 7 - weekday.num_days_from_monday()) % 7;
            let found = last - TimeDelta::days(i64::from(back) + 7 * i64::from(-n - 1));
            (found.month() == month).then_some(found)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ical::ObservanceKind;

    fn at(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y%m%dT%H%M%S").unwrap()
    }

    fn offset(hours: i32) -> FixedOffset {
        FixedOffset::east_opt(hours * 3600).unwrap()
    }

    /// US Pacific as Outlook writes it: onsets since 1601, by rule.
    fn pacific() -> ZoneRules {
        ZoneRules {
            tzid: "Custom Pacific".into(),
            observances: vec![
                Observance {
                    kind: ObservanceKind::Standard,
                    start: at("16010101T020000"),
                    offset_from: offset(-7),
                    offset_to: offset(-8),
                    rule: Some("FREQ=YEARLY;INTERVAL=1;BYDAY=1SU;BYMONTH=11".into()),
                    dates: vec![],
                },
                Observance {
                    kind: ObservanceKind::Daylight,
                    start: at("16010101T020000"),
                    offset_from: offset(-8),
                    offset_to: offset(-7),
                    rule: Some("FREQ=YEARLY;INTERVAL=1;BYDAY=2SU;BYMONTH=3".into()),
                    dates: vec![],
                },
            ],
            lines: vec![],
        }
    }

    #[test]
    fn vtimezone_rules_give_the_offset_in_force() {
        const CASES: &[(&str, i32)] = &[
            ("20260115T090000", -8),
            ("20260307T090000", -8),
            // 8 March 2026 is the second Sunday; the onset is 02:00.
            ("20260308T010000", -8),
            ("20260308T030000", -7),
            ("20260701T120000", -7),
            // 1 November 2026 is the first Sunday.
            ("20261101T030000", -8),
            ("20261231T235959", -8),
        ];
        let rules = pacific();
        for (local, hours) in CASES {
            assert_eq!(
                offset_at(&rules, at(local)),
                Some(offset(*hours)),
                "{local}"
            );
        }
    }

    #[test]
    fn the_last_weekday_of_a_month_counts_from_its_end() {
        /// Year, month, which Sunday, and the (month, day) it falls on.
        type Case = (i32, u32, i32, Option<(u32, u32)>);
        const CASES: &[Case] = &[
            (2026, 3, -1, Some((3, 29))),
            (2026, 10, -1, Some((10, 25))),
            (2026, 12, -1, Some((12, 27))),
            (2026, 3, -2, Some((3, 22))),
            (2026, 3, 2, Some((3, 8))),
            (2026, 2, 5, None),
            (2026, 3, 0, None),
        ];
        for (year, month, n, expected) in CASES {
            let found = nth_weekday(*year, *month, Weekday::Sun, *n).map(|d| (d.month(), d.day()));
            assert_eq!(found, *expected, "{year}-{month} {n}");
        }
    }

    #[test]
    fn a_tzid_is_found_by_iana_name_windows_name_or_path_tail() {
        const CASES: &[(&str, Option<&str>)] = &[
            ("Europe/Berlin", Some("Europe/Berlin")),
            ("Taipei Standard Time", Some("Asia/Taipei")),
            ("taipei standard time", Some("Asia/Taipei")),
            ("W. Europe Standard Time", Some("Europe/Berlin")),
            ("India Standard Time", Some("Asia/Kolkata")),
            (
                "/example.org/20260101_1/America/New_York",
                Some("America/New_York"),
            ),
            ("Customized Time Zone", None),
            ("", None),
            ("Mars/Olympus_Mons", None),
        ];
        for (tzid, expected) in CASES {
            let found = resolve_iana(tzid).map(|tz| tz.name().to_owned());
            // CLDR's representative for India is the older spelling, a link to the same zone.
            let found = found.map(|n| {
                if n == "Asia/Calcutta" {
                    "Asia/Kolkata".into()
                } else {
                    n
                }
            });
            assert_eq!(found.as_deref(), *expected, "{tzid:?}");
        }
    }

    #[test]
    fn every_windows_name_maps_to_a_zone_this_build_knows() {
        for (windows, iana) in WINDOWS_ZONES {
            assert!(iana.parse::<Tz>().is_ok(), "{windows} -> {iana}");
        }
        assert!(WINDOWS_ZONES.windows(2).all(|w| w[0].0 < w[1].0), "sorted");
    }

    #[test]
    fn a_reading_in_a_spring_gap_moves_on_an_hour() {
        let tz: Tz = "Europe/Berlin".parse().unwrap();
        // 29 March 2026, 02:30 does not exist in Berlin; 03:30 CEST is 01:30 UTC.
        assert_eq!(
            from_local(&tz, at("20260329T023000")),
            at("20260329T013000").and_utc()
        );
    }
}
