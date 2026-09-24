//! iCalendar (RFC 5545) as invitations carry it: events, the people asked to them, and the
//! time zones their times are written in.
//!
//! What a mail client needs from a calendar object is what the invitation says — a title, a
//! time, a place, who organised it and who was asked — and enough of the original to answer it
//! (RFC 5546). So those are fields, and every property of an event is also kept as read in
//! [`Event::lines`], which is what an answer echoes back: an organiser's calendar matches a
//! reply to its event by the exact `UID`, `RECURRENCE-ID` and address it wrote, not by ours.
//!
//! Not a calendar: nothing here expands a recurrence, stores an event or talks to a server.

mod read;
mod reply;
mod rrule;
mod windows;
mod zone;

pub use read::{MAX_CALENDAR_BYTES, parse};
pub use reply::{Answering, reply};
pub use rrule::describe as describe_rule;
pub use zone::{EventZone, Placed, place, resolve_iana};

use crate::line::ContentLine;
use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, TimeDelta, Utc};

/// One `VCALENDAR`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Calendar {
    pub method: Method,
    /// Every `VEVENT`, in order. A recurring event and its changed occurrences share a `UID`;
    /// the occurrences carry a [`Event::recurrence_id`].
    pub events: Vec<Event>,
    /// Every `VTIMEZONE`, by the `TZID` it defines.
    pub zones: Vec<ZoneRules>,
}

/// `METHOD`: what the sender wants done with the object (RFC 5546 §1.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    /// No `METHOD`, or one this module does not know: a calendar file, not a message.
    Unstated,
    Publish,
    Request,
    Reply,
    Add,
    Cancel,
    Refresh,
    Counter,
    DeclineCounter,
}

/// One `VEVENT`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    /// `UID`: the event, across every message about it.
    pub uid: String,
    /// `SEQUENCE`: bumped by the organiser on each change that needs attendees to answer again.
    /// Zero when absent.
    pub sequence: u32,
    /// `DTSTAMP`: when this object was written.
    pub stamp: Option<DateTime<Utc>>,
    /// `DTSTART`.
    pub start: Option<Moment>,
    /// `DTEND` or `DURATION`.
    pub end: Option<End>,
    pub summary: Option<String>,
    pub location: Option<String>,
    pub description: Option<String>,
    pub organizer: Option<Party>,
    pub attendees: Vec<Attendee>,
    /// `RRULE`, as written. See [`describe_rule`].
    pub rrule: Option<String>,
    pub status: Option<EventStatus>,
    /// `RECURRENCE-ID`: which occurrence of a recurring event this object is about, when it is
    /// about one rather than the whole series.
    pub recurrence_id: Option<Moment>,
    /// `COMMENT`, which a reply may carry: what the attendee wrote with their answer.
    pub comment: Option<String>,
    /// Every property of the event as read, in order, to echo back in an answer.
    pub lines: Vec<ContentLine>,
}

/// A date-time or date value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Moment {
    /// `VALUE=DATE`: a day rather than a time, which is how an all-day event is written.
    Date(NaiveDate),
    /// A time ending in `Z`.
    Utc(DateTime<Utc>),
    /// A clock reading in the zone `TZID` names.
    Zoned { local: NaiveDateTime, tzid: String },
    /// A clock reading with no zone: the same reading wherever the reader is (RFC 5545 §3.3.5).
    Floating(NaiveDateTime),
}

/// How an event ends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum End {
    /// `DTEND`: exclusive, so an all-day event on the 1st ends on the 2nd.
    At(Moment),
    /// `DURATION`.
    Lasts(TimeDelta),
}

/// Someone named by a `cal-address`: an organiser or an attendee.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Party {
    /// The address, less `mailto:`, spelled as the invitation spelled it.
    pub email: String,
    /// `CN`.
    pub name: Option<String>,
}

/// One `ATTENDEE`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attendee {
    pub party: Party,
    pub answer: PartStat,
    pub role: Role,
    pub rsvp: Rsvp,
}

/// `PARTSTAT` for an event. Anything unrecognised is read as [`PartStat::NeedsAction`], as
/// RFC 5545 §3.2.12 asks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartStat {
    NeedsAction,
    Accepted,
    Declined,
    Tentative,
    Delegated,
}

/// `ROLE`. Unrecognised, or absent, is [`Role::Required`] (RFC 5545 §3.2.16).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Chair,
    Required,
    Optional,
    NonParticipant,
}

/// `RSVP`: whether the organiser asked for an answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rsvp {
    Requested,
    NotRequested,
}

/// `STATUS` of an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventStatus {
    Tentative,
    Confirmed,
    Cancelled,
}

/// One `VTIMEZONE`: a zone's rules, carried inside the calendar that uses it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZoneRules {
    pub tzid: String,
    pub observances: Vec<Observance>,
    /// The component as read, `BEGIN` to `END`, to copy into an answer that names the zone.
    pub lines: Vec<ContentLine>,
}

/// One `STANDARD` or `DAYLIGHT` period of a [`ZoneRules`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observance {
    pub kind: ObservanceKind,
    /// `DTSTART`: the first onset, as a clock reading in the offset that was in force before it.
    pub start: NaiveDateTime,
    pub offset_from: FixedOffset,
    pub offset_to: FixedOffset,
    /// `RRULE`, as written: when the onset recurs.
    pub rule: Option<String>,
    /// `RDATE`s: further onsets.
    pub dates: Vec<NaiveDateTime>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservanceKind {
    Standard,
    Daylight,
}

impl Calendar {
    /// The event a message is about: the one with no `RECURRENCE-ID` (the series, or the only
    /// event), else the first.
    pub fn main_event(&self) -> Option<&Event> {
        self.events
            .iter()
            .find(|e| e.recurrence_id.is_none())
            .or_else(|| self.events.first())
    }
}

impl Method {
    /// The token as RFC 5546 writes it; `None` for [`Method::Unstated`].
    pub fn token(self) -> Option<&'static str> {
        Some(match self {
            Method::Unstated => return None,
            Method::Publish => "PUBLISH",
            Method::Request => "REQUEST",
            Method::Reply => "REPLY",
            Method::Add => "ADD",
            Method::Cancel => "CANCEL",
            Method::Refresh => "REFRESH",
            Method::Counter => "COUNTER",
            Method::DeclineCounter => "DECLINECOUNTER",
        })
    }

    /// Read a `METHOD` value, or a `method` MIME parameter.
    pub fn from_token(token: &str) -> Self {
        const ALL: [Method; 8] = [
            Method::Publish,
            Method::Request,
            Method::Reply,
            Method::Add,
            Method::Cancel,
            Method::Refresh,
            Method::Counter,
            Method::DeclineCounter,
        ];
        let token = token.trim();
        ALL.into_iter()
            .find(|m| m.token().is_some_and(|t| t.eq_ignore_ascii_case(token)))
            .unwrap_or(Method::Unstated)
    }
}

impl PartStat {
    pub fn token(self) -> &'static str {
        match self {
            PartStat::NeedsAction => "NEEDS-ACTION",
            PartStat::Accepted => "ACCEPTED",
            PartStat::Declined => "DECLINED",
            PartStat::Tentative => "TENTATIVE",
            PartStat::Delegated => "DELEGATED",
        }
    }
}

impl From<mail_domain::Attendance> for PartStat {
    fn from(attendance: mail_domain::Attendance) -> Self {
        match attendance {
            mail_domain::Attendance::Accepted => PartStat::Accepted,
            mail_domain::Attendance::Tentative => PartStat::Tentative,
            mail_domain::Attendance::Declined => PartStat::Declined,
        }
    }
}
