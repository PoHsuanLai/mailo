//! An invitation as a calendar keeps it, and a new time proposed for one.
//!
//! [`stored`] is the object a calendar store is given when an invitation is added, or its
//! answer recorded: the event as the organiser wrote it, without the `METHOD` that made it a
//! message, and with the reader's own `PARTSTAT` set to what they answered. [`counter`] is the
//! iTIP `COUNTER` (RFC 5546 §3.2.7) an attendee sends to propose another time, which the
//! organiser may take or refuse.

use super::{Calendar, Event, PartStat};
use crate::PimError;
use crate::line::{ContentLine, escape};
use chrono::{DateTime, Utc};

/// Where the reader stands, to be written into the event.
#[derive(Debug, Clone, Copy)]
pub struct Standing<'a> {
    /// The reader's address, as the invitation spelled it.
    pub address: &'a str,
    pub answer: PartStat,
}

/// The calendar object for the event `event` belongs to, ready to put in a store: every
/// `VEVENT` of the message with that `UID` (a series and its changed occurrences), the rules of
/// the zones they name, and no `METHOD`. With `standing`, the `ATTENDEE` line of that address
/// carries that `PARTSTAT` and no `RSVP`, since the question has been answered.
pub fn stored(
    calendar: &Calendar,
    event: &Event,
    standing: Option<Standing<'_>>,
    product: &str,
) -> String {
    let events: Vec<Vec<ContentLine>> = calendar
        .events
        .iter()
        .filter(|e| e.uid == event.uid)
        .map(|e| {
            e.lines
                .iter()
                .map(|line| match standing {
                    Some(standing) if line.name == "ATTENDEE" && names(line, standing.address) => {
                        let mut line = line.clone();
                        line.params
                            .retain(|p| p.name != "PARTSTAT" && p.name != "RSVP");
                        line.with("PARTSTAT", &[standing.answer.token()])
                    }
                    _ => line.clone(),
                })
                .collect()
        })
        .collect();
    super::write::object(None, product, &calendar.zones, &events)
}

/// A time proposed for an event, and who proposes it.
#[derive(Debug, Clone, Copy)]
pub struct Proposing<'a> {
    /// The attendee's address, as the invitation spelled it.
    pub attendee: &'a str,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    /// Sent as `COMMENT`: why.
    pub comment: Option<&'a str>,
    /// `DTSTAMP`: when it was proposed.
    pub at: DateTime<Utc>,
    pub product: &'a str,
}

/// The `VCALENDAR` proposing another time for `event`, from one attendee.
///
/// What the organiser's calendar matches the event and the attendee by (`UID`, `SEQUENCE`,
/// `RECURRENCE-ID`, `ORGANIZER` and the attendee's own line) is echoed as read, as in a reply;
/// the proposed time is written in UTC, so no zone has to travel with it.
/// [`PimError::Unanswerable`] when the event names no organiser, or does not list the attendee.
pub fn counter(event: &Event, proposing: &Proposing<'_>) -> Result<String, PimError> {
    if event.organizer.is_none() {
        return Err(PimError::Unanswerable("the invitation names no organiser"));
    }
    let attendee = event
        .lines
        .iter()
        .find(|l| l.name == "ATTENDEE" && names(l, proposing.attendee))
        .ok_or(PimError::Unanswerable(
            "the invitation does not list that address among its attendees",
        ))?;
    let echo = |name: &str| event.lines.iter().find(|l| l.name == name).cloned();
    let utc = |name: &str, at: DateTime<Utc>| {
        ContentLine::new(name, at.format("%Y%m%dT%H%M%SZ").to_string())
    };
    let mut body: Vec<ContentLine> = Vec::new();
    body.extend(echo("UID"));
    body.extend(echo("SEQUENCE"));
    body.extend(echo("RECURRENCE-ID"));
    body.push(utc("DTSTAMP", proposing.at));
    body.push(utc("DTSTART", proposing.start));
    body.push(utc("DTEND", proposing.end));
    body.extend(echo("SUMMARY"));
    body.extend(echo("ORGANIZER"));
    let mut who = attendee.clone();
    who.group = None;
    who.params.retain(|p| p.name != "RSVP");
    body.push(who);
    if let Some(comment) = proposing.comment.map(str::trim).filter(|c| !c.is_empty()) {
        body.push(ContentLine::new("COMMENT", escape(comment)));
    }
    Ok(super::write::object(
        Some("COUNTER"),
        proposing.product,
        &[],
        &[body],
    ))
}

/// Whether an `ATTENDEE` line names `address`, compared as addresses are: case-insensitively.
fn names(line: &ContentLine, address: &str) -> bool {
    super::reply::same_address(line, address)
}
