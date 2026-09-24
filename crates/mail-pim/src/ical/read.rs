//! Reading a `VCALENDAR`, as calendar programs and mail servers write them into invitations.

use super::{
    Attendee, Calendar, End, Event, EventStatus, Method, Moment, Observance, ObservanceKind,
    PartStat, Party, Role, Rsvp, ZoneRules,
};
use crate::PimError;
use crate::line::{self, ContentLine, unescape};
use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, TimeDelta, Utc};

/// The largest calendar object read, in bytes. An invitation for a meeting with a few hundred
/// people and a long agenda is tens of kilobytes; this is room for a hundred of those, and a
/// bound on what a hostile one can make the reader do.
pub const MAX_CALENDAR_BYTES: usize = 1 << 20;

/// Components nested deeper than this are passed over unread. Real ones go three deep
/// (`VCALENDAR` → `VTIMEZONE` → `STANDARD`, or `VEVENT` → `VALARM`).
const MAX_DEPTH: usize = 16;

/// The calendar in `text`.
///
/// Lenient, like [`crate::vcard::parse`], because the text came from a stranger's program: a
/// line that does not parse is skipped, an event with no `UID` is passed over (nothing could
/// answer it or recognise it again), and one cut off by the end of the text is kept with what it
/// had. [`PimError::TooLarge`] past [`MAX_CALENDAR_BYTES`]; [`PimError::Unexpected`] when there
/// is no calendar or event in it at all.
pub fn parse(text: &str) -> Result<Calendar, PimError> {
    if text.len() > MAX_CALENDAR_BYTES {
        return Err(PimError::TooLarge {
            limit: MAX_CALENDAR_BYTES,
        });
    }
    let mut reader = Reader::default();
    for line in line::lines(text) {
        reader.feed(line);
    }
    reader.finish()
}

#[derive(Default)]
struct Reader {
    method: Option<Method>,
    stack: Vec<String>,
    /// `BEGIN`s past [`MAX_DEPTH`], still to be matched by an `END`.
    overflow: usize,
    seen_calendar: bool,
    event: Option<Event>,
    zone: Option<ZoneRules>,
    observance: Option<(ObservanceKind, Vec<ContentLine>)>,
    events: Vec<Event>,
    zones: Vec<ZoneRules>,
}

impl Reader {
    fn feed(&mut self, line: ContentLine) {
        if let Some(zone) = self.zone.as_mut() {
            zone.lines.push(line.clone());
        }
        match line.name.as_str() {
            "BEGIN" => self.begin(line.value.trim().to_ascii_uppercase()),
            "END" => self.end(line.value.trim()),
            _ => self.property(line),
        }
    }

    fn begin(&mut self, name: String) {
        if self.stack.len() >= MAX_DEPTH {
            self.overflow += 1;
            return;
        }
        let top = self.stack.last().map(String::as_str);
        match (top, name.as_str()) {
            (_, "VCALENDAR") => self.seen_calendar = true,
            (Some("VCALENDAR") | None, "VEVENT") if self.event.is_none() => {
                self.event = Some(empty_event());
            }
            (Some("VCALENDAR") | None, "VTIMEZONE") if self.zone.is_none() => {
                self.zone = Some(ZoneRules {
                    tzid: String::new(),
                    observances: Vec::new(),
                    lines: vec![ContentLine::new("BEGIN", "VTIMEZONE")],
                });
            }
            (Some("VTIMEZONE"), "STANDARD") => {
                self.observance = Some((ObservanceKind::Standard, Vec::new()));
            }
            (Some("VTIMEZONE"), "DAYLIGHT") => {
                self.observance = Some((ObservanceKind::Daylight, Vec::new()));
            }
            _ => {}
        }
        self.stack.push(name);
    }

    fn end(&mut self, name: &str) {
        if self.overflow > 0 {
            self.overflow -= 1;
            return;
        }
        // An `END` closes the innermost open component of its name, and any left open inside
        // it: a missing `END:VALARM` must not keep the rest of the event from being read.
        let Some(at) = self
            .stack
            .iter()
            .rposition(|open| open.eq_ignore_ascii_case(name))
        else {
            return;
        };
        while self.stack.len() > at {
            if let Some(closed) = self.stack.pop() {
                self.close(&closed);
            }
        }
    }

    fn close(&mut self, name: &str) {
        match name {
            "VEVENT" if self.stack.last().is_none_or(|top| top == "VCALENDAR") => {
                if let Some(event) = self.event.take().filter(|e| !e.uid.is_empty()) {
                    self.events.push(event);
                }
            }
            "VTIMEZONE" => {
                if let Some(mut zone) = self.zone.take().filter(|z| !z.tzid.is_empty()) {
                    // Cut off before its `END`, it is still copied whole into an answer.
                    if zone.lines.last().is_none_or(|l| l.name != "END") {
                        zone.lines.push(ContentLine::new("END", "VTIMEZONE"));
                    }
                    self.zones.push(zone);
                }
            }
            "STANDARD" | "DAYLIGHT" => {
                if let Some((kind, lines)) = self.observance.take()
                    && let Some(observance) = observance(kind, &lines)
                    && let Some(zone) = self.zone.as_mut()
                {
                    zone.observances.push(observance);
                }
            }
            _ => {}
        }
    }

    fn property(&mut self, line: ContentLine) {
        match self.stack.last().map(String::as_str) {
            Some("VCALENDAR") if line.name == "METHOD" => {
                self.method = Some(Method::from_token(&line.value));
            }
            Some("VEVENT") => {
                if let Some(event) = self.event.as_mut() {
                    read_event_property(event, line);
                }
            }
            Some("VTIMEZONE") if line.name == "TZID" => {
                if let Some(zone) = self.zone.as_mut() {
                    zone.tzid = line.value.trim().to_owned();
                }
            }
            Some("STANDARD" | "DAYLIGHT") => {
                if let Some((_, lines)) = self.observance.as_mut() {
                    lines.push(line);
                }
            }
            _ => {}
        }
    }

    fn finish(mut self) -> Result<Calendar, PimError> {
        // Whatever the text left open is closed where it stopped.
        while let Some(open) = self.stack.pop() {
            self.close(&open);
        }
        if !self.seen_calendar && self.events.is_empty() {
            return Err(PimError::Unexpected {
                expected: "calendar",
            });
        }
        Ok(Calendar {
            method: self.method.unwrap_or(Method::Unstated),
            events: self.events,
            zones: self.zones,
        })
    }
}

fn empty_event() -> Event {
    Event {
        uid: String::new(),
        sequence: 0,
        stamp: None,
        start: None,
        end: None,
        summary: None,
        location: None,
        description: None,
        organizer: None,
        attendees: Vec::new(),
        rrule: None,
        status: None,
        recurrence_id: None,
        comment: None,
        lines: Vec::new(),
    }
}

fn read_event_property(event: &mut Event, line: ContentLine) {
    match line.name.as_str() {
        "UID" => event.uid = line.value.trim().to_owned(),
        "SEQUENCE" => event.sequence = line.value.trim().parse().unwrap_or(0),
        "DTSTAMP" => {
            event.stamp = match moment(&line) {
                Some(Moment::Utc(at)) => Some(at),
                _ => None,
            }
        }
        "DTSTART" => event.start = moment(&line),
        "DTEND" => event.end = moment(&line).map(End::At),
        // `DTEND` wins when an event carries both, which RFC 5545 forbids and some do anyway.
        "DURATION" if !matches!(event.end, Some(End::At(_))) => {
            event.end = duration(&line.value).map(End::Lasts);
        }
        "SUMMARY" => event.summary = text(&line.value),
        "LOCATION" => event.location = text(&line.value),
        "DESCRIPTION" => event.description = text(&line.value),
        "COMMENT" => event.comment = text(&line.value),
        "ORGANIZER" => event.organizer = party(&line),
        "ATTENDEE" => {
            if let Some(attendee) = attendee(&line) {
                event.attendees.push(attendee);
            }
        }
        "RRULE" => event.rrule = Some(line.value.trim().to_owned()),
        "STATUS" => event.status = status(&line.value),
        "RECURRENCE-ID" => event.recurrence_id = moment(&line),
        _ => {}
    }
    event.lines.push(line);
}

/// A text value, unescaped, with line breaks as `\n`; `None` when blank.
fn text(value: &str) -> Option<String> {
    let text = unescape(value).replace("\r\n", "\n");
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

/// A `DATE` or `DATE-TIME` property's one value, with its `TZID`.
pub(super) fn moment(line: &ContentLine) -> Option<Moment> {
    let tzid = line
        .values("TZID")
        .next()
        .map(str::trim)
        .filter(|t| !t.is_empty());
    // Of a list (`RDATE`, `EXDATE`), the first.
    let value = line.value.split(',').next()?.trim();
    moment_value(value, line.has("VALUE", "DATE"), tzid)
}

pub(super) fn moment_value(value: &str, date_only: bool, tzid: Option<&str>) -> Option<Moment> {
    if date_only || value.len() == 8 {
        return NaiveDate::parse_from_str(value.get(..8)?, "%Y%m%d")
            .ok()
            .map(Moment::Date);
    }
    let (clock, utc) = match value.strip_suffix(['Z', 'z']) {
        Some(clock) => (clock, true),
        None => (value, false),
    };
    let local = NaiveDateTime::parse_from_str(clock, "%Y%m%dT%H%M%S").ok()?;
    Some(match (utc, tzid) {
        (true, _) => Moment::Utc(DateTime::from_naive_utc_and_offset(local, Utc)),
        (false, Some(tzid)) => Moment::Zoned {
            local,
            tzid: tzid.to_owned(),
        },
        (false, None) => Moment::Floating(local),
    })
}

/// `DURATION` (RFC 5545 §3.3.6): `P3W`, `P1DT2H`, `PT15M`. `None` for a negative or
/// malformed one, or one too long to be an event.
pub(super) fn duration(value: &str) -> Option<TimeDelta> {
    let value = value.trim();
    let value = value.strip_prefix('+').unwrap_or(value);
    let rest = value.strip_prefix(['P', 'p'])?;
    let mut total = TimeDelta::zero();
    let mut number = String::new();
    let mut any = false;
    for ch in rest.chars() {
        match ch.to_ascii_uppercase() {
            '0'..='9' if number.len() < 9 => number.push(ch),
            'T' if number.is_empty() => {}
            unit @ ('W' | 'D' | 'H' | 'M' | 'S') => {
                let n: i64 = number.parse().ok()?;
                number.clear();
                let part = match unit {
                    'W' => TimeDelta::try_weeks(n)?,
                    'D' => TimeDelta::try_days(n)?,
                    'H' => TimeDelta::try_hours(n)?,
                    'M' => TimeDelta::try_minutes(n)?,
                    _ => TimeDelta::try_seconds(n)?,
                };
                total = total.checked_add(&part)?;
                any = true;
            }
            _ => return None,
        }
    }
    (any && number.is_empty()).then_some(total)
}

/// A `UTC-OFFSET` value: `+0800`, `-0500`, `+053000`.
fn offset(value: &str) -> Option<FixedOffset> {
    let value = value.trim();
    let (sign, digits) = match value.split_at_checked(1)? {
        ("+", digits) => (1, digits),
        ("-", digits) => (-1, digits),
        _ => return None,
    };
    if !(digits.len() == 4 || digits.len() == 6) || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let field = |range: std::ops::Range<usize>| digits.get(range)?.parse::<i32>().ok();
    let seconds = field(0..2)? * 3600 + field(2..4)? * 60 + field(4..6).unwrap_or(0);
    FixedOffset::east_opt(sign * seconds)
}

/// An `ORGANIZER` or `ATTENDEE`: `mailto:` stripped, or the `EMAIL` parameter when the value
/// is some other kind of address (a `urn:uuid:`, as some servers write).
fn party(line: &ContentLine) -> Option<Party> {
    let value = line.value.trim();
    let email = match value.get(..7) {
        Some(scheme) if scheme.eq_ignore_ascii_case("mailto:") => value[7..].trim().to_owned(),
        _ => line
            .values("EMAIL")
            .next()
            .map(|e| e.trim().to_owned())
            .unwrap_or_default(),
    };
    if !email.contains('@') || email.chars().any(char::is_control) {
        return None;
    }
    let name = line
        .values("CN")
        .next()
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(str::to_owned);
    Some(Party { email, name })
}

fn attendee(line: &ContentLine) -> Option<Attendee> {
    let party = party(line)?;
    let param = |name| line.values(name).next().map(str::trim).unwrap_or("");
    let answer = match param("PARTSTAT").to_ascii_uppercase().as_str() {
        "ACCEPTED" => PartStat::Accepted,
        "DECLINED" => PartStat::Declined,
        "TENTATIVE" => PartStat::Tentative,
        "DELEGATED" => PartStat::Delegated,
        _ => PartStat::NeedsAction,
    };
    let role = match param("ROLE").to_ascii_uppercase().as_str() {
        "CHAIR" => Role::Chair,
        "OPT-PARTICIPANT" => Role::Optional,
        "NON-PARTICIPANT" => Role::NonParticipant,
        _ => Role::Required,
    };
    let rsvp = if param("RSVP").eq_ignore_ascii_case("TRUE") {
        Rsvp::Requested
    } else {
        Rsvp::NotRequested
    };
    Some(Attendee {
        party,
        answer,
        role,
        rsvp,
    })
}

fn status(value: &str) -> Option<EventStatus> {
    match value.trim().to_ascii_uppercase().as_str() {
        "TENTATIVE" => Some(EventStatus::Tentative),
        "CONFIRMED" => Some(EventStatus::Confirmed),
        "CANCELLED" => Some(EventStatus::Cancelled),
        _ => None,
    }
}

/// A `STANDARD` or `DAYLIGHT` period, or `None` when it lacks a start or either offset.
fn observance(kind: ObservanceKind, lines: &[ContentLine]) -> Option<Observance> {
    let find = |name: &str| lines.iter().find(|l| l.name == name);
    let start = match moment(find("DTSTART")?)? {
        Moment::Floating(local) | Moment::Zoned { local, .. } => local,
        Moment::Utc(at) => at.naive_utc(),
        Moment::Date(day) => day.and_hms_opt(0, 0, 0)?,
    };
    let dates = lines
        .iter()
        .filter(|l| l.name == "RDATE")
        .flat_map(|l| l.value.split(','))
        .filter_map(|v| match moment_value(v.trim(), false, None)? {
            Moment::Floating(local) => Some(local),
            _ => None,
        })
        .collect();
    Some(Observance {
        kind,
        start,
        offset_from: offset(&find("TZOFFSETFROM")?.value)?,
        offset_to: offset(&find("TZOFFSETTO")?.value)?,
        rule: find("RRULE").map(|l| l.value.trim().to_owned()),
        dates,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_read_as_rfc_5545_writes_them() {
        const CASES: &[(&str, Option<i64>)] = &[
            ("PT1H", Some(3600)),
            ("PT1H30M", Some(5400)),
            ("P1D", Some(86_400)),
            ("P1DT2H", Some(93_600)),
            ("P2W", Some(1_209_600)),
            ("+PT15M", Some(900)),
            ("PT0S", Some(0)),
            ("-PT15M", None),
            ("P", None),
            ("PT", None),
            ("1H", None),
            ("PT99999999999H", None),
        ];
        for (input, seconds) in CASES {
            assert_eq!(
                duration(input).map(|d| d.num_seconds()),
                *seconds,
                "{input}"
            );
        }
    }

    #[test]
    fn utc_offsets_read_with_and_without_seconds() {
        const CASES: &[(&str, Option<i32>)] = &[
            ("+0800", Some(28_800)),
            ("-0500", Some(-18_000)),
            ("+0530", Some(19_800)),
            ("+053015", Some(19_815)),
            ("0800", None),
            ("+08", None),
            ("+ab00", None),
        ];
        for (input, seconds) in CASES {
            assert_eq!(
                offset(input).map(|o| o.local_minus_utc()),
                *seconds,
                "{input}"
            );
        }
    }

    #[test]
    fn a_moment_is_a_date_a_utc_time_a_zoned_time_or_floating() {
        let at = |s| NaiveDateTime::parse_from_str(s, "%Y%m%dT%H%M%S").unwrap();
        let line = |s: &str| line::parse(s).unwrap();
        assert_eq!(
            moment(&line("DTSTART;VALUE=DATE:20261001")),
            Some(Moment::Date(NaiveDate::from_ymd_opt(2026, 10, 1).unwrap()))
        );
        assert_eq!(
            moment(&line("DTSTART:20261001T090000Z")),
            Some(Moment::Utc(at("20261001T090000").and_utc()))
        );
        assert_eq!(
            moment(&line(
                "DTSTART;TZID=\"Taipei Standard Time\":20261001T090000"
            )),
            Some(Moment::Zoned {
                local: at("20261001T090000"),
                tzid: "Taipei Standard Time".into()
            })
        );
        assert_eq!(
            moment(&line("DTSTART:20261001T090000")),
            Some(Moment::Floating(at("20261001T090000")))
        );
        assert_eq!(moment(&line("DTSTART:tomorrow")), None);
        assert_eq!(moment(&line("DTSTART:20261341T090000")), None);
    }
}
