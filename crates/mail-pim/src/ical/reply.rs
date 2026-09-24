//! Answering an invitation: the iTIP `REPLY` (RFC 5546 §3.2.3) that carries one attendee's
//! answer back to the organiser.
//!
//! The organiser's calendar finds the event by `UID` and `RECURRENCE-ID` and the attendee by
//! address, all compared as it wrote them, so those lines are echoed exactly as read rather
//! than rebuilt. Only the answering attendee is named: a reply listing everyone would tell the
//! organiser's calendar that this attendee speaks for them all.

use super::{Calendar, Event};
use crate::PimError;
use crate::line::{self, ContentLine, escape};
use chrono::{DateTime, Utc};
use mail_domain::Attendance;

/// Who answers, and what.
#[derive(Debug, Clone, Copy)]
pub struct Answering<'a> {
    /// The attendee's address, as the invitation spelled it.
    pub attendee: &'a str,
    pub attendance: Attendance,
    /// Sent as `COMMENT`: a note to the organiser with the answer.
    pub comment: Option<&'a str>,
    /// `DTSTAMP`: when the answer was given.
    pub at: DateTime<Utc>,
    /// `PRODID`, e.g. `-//mailo//mailo 0.1.0//EN`.
    pub product: &'a str,
}

/// Properties of the event carried into the reply as they were read, in this order.
///
/// `UID`, `SEQUENCE`, `RECURRENCE-ID` and `ORGANIZER` are what RFC 5546 requires. The time and
/// the title are optional there, and carried because some calendars show a reply by them.
const ECHOED: &[&str] = &[
    "UID",
    "SEQUENCE",
    "RECURRENCE-ID",
    "DTSTART",
    "DTEND",
    "DURATION",
    "SUMMARY",
    "ORGANIZER",
];

/// The `VCALENDAR` answering `event` of `calendar` for one attendee.
///
/// [`PimError::Unanswerable`] when the event names no organiser to answer, or the attendee is
/// not among those it lists.
pub fn reply(
    calendar: &Calendar,
    event: &Event,
    answering: &Answering<'_>,
) -> Result<String, PimError> {
    if event.organizer.is_none() {
        return Err(PimError::Unanswerable("the invitation names no organiser"));
    }
    let attendee = event
        .lines
        .iter()
        .filter(|l| l.name == "ATTENDEE")
        .find(|l| same_address(l, answering.attendee))
        .ok_or(PimError::Unanswerable(
            "the invitation does not list that address among its attendees",
        ))?;

    let mut body: Vec<ContentLine> = Vec::new();
    for name in ECHOED {
        body.extend(
            event
                .lines
                .iter()
                .filter(|l| l.name == *name)
                .take(1)
                .cloned(),
        );
        if *name == "SEQUENCE" {
            body.push(ContentLine::new(
                "DTSTAMP",
                answering.at.format("%Y%m%dT%H%M%SZ").to_string(),
            ));
        }
    }
    let mut answer = attendee.clone();
    answer.group = None;
    answer
        .params
        .retain(|p| p.name != "PARTSTAT" && p.name != "RSVP");
    let partstat = super::PartStat::from(answering.attendance).token();
    body.push(answer.with("PARTSTAT", &[partstat]));
    if let Some(comment) = answering.comment.map(str::trim).filter(|c| !c.is_empty()) {
        body.push(ContentLine::new("COMMENT", escape(comment)));
    }

    let mut out = String::new();
    for head in [
        ContentLine::new("BEGIN", "VCALENDAR"),
        ContentLine::new("PRODID", answering.product),
        ContentLine::new("VERSION", "2.0"),
        ContentLine::new("CALSCALE", "GREGORIAN"),
        ContentLine::new("METHOD", "REPLY"),
    ] {
        out.push_str(&line::write(&head));
    }
    // Every zone a carried time names, so the organiser's calendar can read it (RFC 5545
    // §3.6.5), each once.
    let mut named: Vec<&str> = Vec::new();
    for tzid in body.iter().flat_map(|l| l.values("TZID")) {
        if !named.contains(&tzid) {
            named.push(tzid);
        }
    }
    for zone in named
        .iter()
        .filter_map(|tzid| calendar.zones.iter().find(|z| z.tzid == *tzid))
    {
        for zone_line in &zone.lines {
            out.push_str(&line::write(zone_line));
        }
    }
    out.push_str(&line::write(&ContentLine::new("BEGIN", "VEVENT")));
    for property in &body {
        out.push_str(&line::write(property));
    }
    out.push_str(&line::write(&ContentLine::new("END", "VEVENT")));
    out.push_str(&line::write(&ContentLine::new("END", "VCALENDAR")));
    Ok(out)
}

/// Whether an `ATTENDEE` line names `address`, compared as addresses are: case-insensitively.
fn same_address(line: &ContentLine, address: &str) -> bool {
    let value = line.value.trim();
    let written = match value.get(..7) {
        Some(scheme) if scheme.eq_ignore_ascii_case("mailto:") => value[7..].trim(),
        _ => line.values("EMAIL").next().map(str::trim).unwrap_or(""),
    };
    written.eq_ignore_ascii_case(address.trim())
}
