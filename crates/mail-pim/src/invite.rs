//! An invitation as the reader shows it: what the calendar object in a message says, reduced to
//! what a person reading mail needs to decide on it.
//!
//! [`summarise`] picks the event a message is about, places its time, puts its repetition into
//! words and finds the reader among the people it names. [`show_when`] then says the time in the
//! reader's zone, and in the organiser's when that reads differently.

use crate::ical::{
    Attendee, Calendar, End, EventStatus, EventZone, Method, Moment, PartStat, Party, Placed,
    describe_rule, place,
};
use chrono::{DateTime, NaiveDate, NaiveDateTime, Offset, TimeDelta, TimeZone, Utc};

/// What an invitation says, for the reader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invite {
    pub kind: Kind,
    /// The event's `UID`: what an answer and any later update are matched by.
    pub uid: String,
    pub sequence: u32,
    /// Which occurrence of a recurring event this is about, when it is about only one.
    pub recurrence_id: Option<Moment>,
    pub title: Option<String>,
    pub when: When,
    /// The event's repetition in words, when it repeats.
    pub repeats: Option<String>,
    pub location: Option<String>,
    pub description: Option<String>,
    pub organiser: Option<Party>,
    /// Everyone asked, with what each has answered so far as far as this message knows.
    pub attendees: Vec<Attendee>,
    /// Where the reader stands in it.
    pub me: Me,
    /// The note an attendee sent with their answer, on a [`Kind::Reply`].
    pub comment: Option<String>,
}

/// What the message asks of the reader.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// `METHOD:REQUEST`: an invitation, to be answered.
    Request(Revision),
    /// `METHOD:CANCEL`, or an event whose `STATUS` is `CANCELLED`: nothing to answer.
    Cancelled,
    /// `METHOD:REPLY`: someone answering an invitation the reader sent. The answers are in
    /// [`Invite::attendees`].
    Reply,
    /// `METHOD:PUBLISH`, or none: an event to add to a calendar, with no answer expected.
    Published,
}

/// Whether an invitation is the first for its event or a change to one already sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Revision {
    First,
    /// `SEQUENCE` above zero: the organiser changed something and asks again.
    Update {
        sequence: u32,
    },
}

/// The reader's place in an invitation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Me {
    /// One of the reader's addresses organised it.
    Organiser,
    /// One of the reader's addresses is among the attendees, with the answer the message
    /// records for it.
    Invited { address: String, answer: PartStat },
    /// None of the reader's addresses is named: forwarded, or sent to a list.
    NotListed,
}

/// When the event is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum When {
    /// No `DTSTART`.
    Unstated,
    /// Whole days, `last` included.
    AllDay { first: NaiveDate, last: NaiveDate },
    Timed {
        start: DateTime<Utc>,
        end: Option<DateTime<Utc>>,
        /// The zone the organiser wrote the time in.
        zone: EventZone,
    },
    /// A clock reading that could not be placed on the time line.
    Floating {
        start: NaiveDateTime,
        end: Option<NaiveDateTime>,
        why: Unplaced,
    },
}

/// Why a time is a clock reading rather than an instant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unplaced {
    /// Written without a zone, to be read in the reader's.
    AsWritten,
    /// Written in a zone nothing could identify, named here.
    UnknownZone(String),
}

/// A time said for the reader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhenShown {
    /// In the reader's zone.
    pub yours: String,
    /// In the organiser's zone, named, when it reads differently from `yours`.
    pub theirs: Option<String>,
}

/// The invitation in `calendar`, for a reader whose addresses are `me`.
///
/// `None` when it holds no event, or asks for something that is not an invitation to show
/// (`REFRESH`, `COUNTER`, `DECLINECOUNTER`). Addresses are compared case-insensitively.
pub fn summarise(calendar: &Calendar, me: &[&str]) -> Option<Invite> {
    let event = calendar.main_event()?;
    let cancelled = event.status == Some(EventStatus::Cancelled);
    let kind = match calendar.method {
        Method::Cancel => Kind::Cancelled,
        _ if cancelled => Kind::Cancelled,
        Method::Request => Kind::Request(match event.sequence {
            0 => Revision::First,
            sequence => Revision::Update { sequence },
        }),
        Method::Reply => Kind::Reply,
        Method::Publish | Method::Add | Method::Unstated => Kind::Published,
        Method::Refresh | Method::Counter | Method::DeclineCounter => return None,
    };
    let mine = |address: &str| me.iter().any(|m| m.trim().eq_ignore_ascii_case(address));
    let me = if event.organizer.as_ref().is_some_and(|o| mine(&o.email)) {
        Me::Organiser
    } else {
        event
            .attendees
            .iter()
            .find(|a| mine(&a.party.email))
            .map_or(Me::NotListed, |a| Me::Invited {
                address: a.party.email.clone(),
                answer: a.answer,
            })
    };
    Some(Invite {
        kind,
        uid: event.uid.clone(),
        sequence: event.sequence,
        recurrence_id: event.recurrence_id.clone(),
        title: event.summary.clone(),
        when: when(event.start.as_ref(), event.end.as_ref(), calendar),
        repeats: event.rrule.as_deref().map(describe_rule),
        location: event.location.clone(),
        description: event.description.clone(),
        organiser: event.organizer.clone(),
        attendees: event.attendees.clone(),
        me,
        comment: event.comment.clone(),
    })
}

fn when(start: Option<&Moment>, end: Option<&End>, calendar: &Calendar) -> When {
    let Some(start) = start else {
        return When::Unstated;
    };
    let end = end.map(|end| match end {
        End::At(moment) => Ok(place(moment, &calendar.zones)),
        End::Lasts(delta) => Err(*delta),
    });
    match place(start, &calendar.zones) {
        Placed::Day(first) => {
            let last = match end {
                Some(Ok(Placed::Day(end))) => end.pred_opt().unwrap_or(end),
                Some(Err(delta)) => first
                    .checked_add_signed(TimeDelta::days(delta.num_days() - 1))
                    .unwrap_or(first),
                _ => first,
            };
            When::AllDay {
                first,
                last: last.max(first),
            }
        }
        Placed::At { utc, zone } => {
            let end = match end {
                Some(Ok(Placed::At { utc: end, .. })) => Some(end),
                Some(Err(delta)) => utc.checked_add_signed(delta),
                _ => None,
            };
            When::Timed {
                start: utc,
                end: end.filter(|end| *end >= utc),
                zone,
            }
        }
        Placed::Floating(local) => floating(local, end, Unplaced::AsWritten),
        Placed::UnknownZone { local, tzid } => floating(local, end, Unplaced::UnknownZone(tzid)),
    }
}

fn floating(start: NaiveDateTime, end: Option<Result<Placed, TimeDelta>>, why: Unplaced) -> When {
    let end = match end {
        Some(Ok(Placed::Floating(end) | Placed::UnknownZone { local: end, .. })) => Some(end),
        Some(Err(delta)) => start.checked_add_signed(delta),
        _ => None,
    };
    When::Floating {
        start,
        end: end.filter(|end| *end >= start),
        why,
    }
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
