//! An invitation in a calendar: the event a message holds as a calendar store takes it, and what
//! an answer or a cancellation does to the one already stored.
//!
//! The store is [`porter_calendar::CalendarStore`], whatever the platform's is (EventKit,
//! Evolution Data Server, a CalDAV or Graph account); this module is generic over it and knows
//! none of them, so the window's tests hand it a fake. The iCalendar text is `mail-pim`'s, from
//! the message; the store keeps it whole where it can, and takes the few fields beside it where
//! it cannot.
//!
//! Nothing here runs by itself. Opening a message touches no calendar: a calendar may ask the
//! person for access the first time it is used, and reading mail is no reason to ask. Each call
//! is the answer to a press of a button.

use super::{addresses, product, raw_of};
use crate::error::{CoreError, Logged};
use chrono::{Datelike, LocalResult, TimeZone};
use mail_domain::{Attendance, MessageId};
use mail_pim::ical::{self, PartStat, Standing};
use mail_pim::{Me, When};
use mail_store::{SqliteStore, Store};
use porter_calendar::{CalendarRef, CalendarStore, Date, Event, Instant, Stored};

/// The event `message` holds, as a store takes it. With `attendance`, the reader's own line in
/// it says that answer; without, the invitation is as it was sent.
///
/// A time written without a zone is placed in `zone`, the reader's: a store keeps instants.
/// [`CoreError::NoTimeToKeep`] when the event has no time at all.
pub fn event_of<Z: TimeZone>(
    store: &SqliteStore,
    message: MessageId,
    attendance: Option<Attendance>,
    zone: &Z,
) -> Result<Event, CoreError> {
    let original = store.message(message)?;
    let bytes = raw_of(store, &original)?.ok_or(CoreError::HeadersOnly)?;
    let mine = addresses(store, original.account);
    let me: Vec<&str> = mine.iter().map(|(_, address)| address.as_str()).collect();
    event_in(&bytes, &me, attendance, zone)
}

/// [`event_of`] over a message's bytes: pure, for the window to call on bytes it holds.
pub fn event_in<Z: TimeZone>(
    raw: &[u8],
    me: &[&str],
    attendance: Option<Attendance>,
    zone: &Z,
) -> Result<Event, CoreError> {
    let part = mail_mime::calendar_part(raw).ok_or(CoreError::NoCalendar)?;
    let calendar =
        ical::parse(&part.text).map_err(|e| CoreError::context("the calendar object", e))?;
    let invite = mail_pim::summarise(&calendar, me).ok_or(CoreError::NoInvitation)?;
    let main = calendar.main_event().ok_or(CoreError::NoInvitation)?;
    let standing = match (&invite.me, attendance) {
        (Me::Invited { address, .. }, Some(attendance)) => Some(Standing {
            address,
            answer: PartStat::from(attendance),
        }),
        _ => None,
    };
    Ok(Event {
        uid: invite.uid.clone(),
        ics: ical::stored(&calendar, main, standing, &product()),
        // An event with no title is put in with none: the store shows its own word for it.
        summary: invite.title.clone().unwrap_or_default(),
        when: placed(&invite.when, zone)?,
        location: invite.location.clone(),
        description: invite.description.clone(),
    })
}

/// A store's time for `when`.
fn placed<Z: TimeZone>(when: &When, zone: &Z) -> Result<porter_calendar::When, CoreError> {
    let date = |day: chrono::NaiveDate| Date {
        year: day.year(),
        month: day.month() as u8,
        day: day.day() as u8,
    };
    let local = |at: chrono::NaiveDateTime| match zone.from_local_datetime(&at) {
        // The earlier of two readings a clock change repeats; a reading it skips has none.
        LocalResult::Single(at) | LocalResult::Ambiguous(at, _) => Some(Instant(at.timestamp())),
        LocalResult::None => None,
    };
    Ok(match when {
        When::Unstated => return Err(CoreError::NoTimeToKeep),
        When::AllDay { first, last } => porter_calendar::When::AllDay {
            first: date(*first),
            last: date(*last),
        },
        When::Timed { start, end, .. } => {
            let start = Instant(start.timestamp());
            porter_calendar::When::Timed {
                start,
                end: end.map_or(start, |end| Instant(end.timestamp())),
            }
        }
        When::Floating { start, end, .. } => {
            let first = local(*start).ok_or(CoreError::NoTimeToKeep)?;
            porter_calendar::When::Timed {
                start: first,
                end: end.and_then(local).unwrap_or(first),
            }
        }
    })
}

/// Which of `calendars` an event goes into: the one named `wanted` while it is there and takes
/// events, else the first that does. `None` when none does.
pub fn pick<'a>(calendars: &'a [CalendarRef], wanted: &str) -> Option<&'a CalendarRef> {
    calendars
        .iter()
        .find(|calendar| calendar.writable && calendar.id == wanted)
        .or_else(|| calendars.iter().find(|calendar| calendar.writable))
}

/// What an answer did to the calendar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Followed {
    /// The event is in the calendar, as the store now holds it.
    Kept(Stored),
    /// A declined event that was in the calendar is gone from it.
    Removed,
    /// A declined event that was never in the calendar: nothing to do.
    Absent,
}

/// Put `event` into `calendar`: a new event, or the one with its `UID` replaced.
pub async fn add<S: CalendarStore>(
    store: &S,
    calendar: &str,
    event: &Event,
) -> Result<Stored, CoreError> {
    store
        .put(calendar, event)
        .await
        .map_err(|e| CoreError::context("the calendar", e))
}

/// Bring the calendar in step with an answer.
///
/// Accepting or saying maybe puts `event` (written with that answer) in the calendar, adding it
/// or updating the stored one. Declining takes the event out if it is there, and does not add
/// it if it is not: a store keeps no attendee status of its own that a declined event could
/// carry, and a calendar full of what the person declined is no calendar.
pub async fn follow_answer<S: CalendarStore>(
    store: &S,
    calendar: &str,
    event: &Event,
    attendance: Attendance,
) -> Result<Followed, CoreError> {
    match attendance {
        Attendance::Accepted | Attendance::Tentative => {
            add(store, calendar, event).await.map(Followed::Kept)
        }
        Attendance::Declined => {
            let gone = store
                .remove(calendar, &event.uid)
                .await
                .map_err(|e| CoreError::context("the calendar", e))?;
            Ok(if gone {
                Followed::Removed
            } else {
                Followed::Absent
            })
        }
    }
}

/// Take the event a cancellation names out of `calendar`. `true` when it was there.
pub async fn follow_cancellation<S: CalendarStore>(
    store: &S,
    calendar: &str,
    uid: &str,
) -> Result<bool, CoreError> {
    store
        .remove(calendar, uid)
        .await
        .map_err(|e| CoreError::context("the calendar", e))
}

/// A calendar store's calendars, or none when it cannot say: a store that cannot be reached
/// offers nothing to choose, and the reason is logged for whoever looks.
pub async fn calendars<S: CalendarStore>(store: &S) -> Vec<CalendarRef> {
    store
        .calendars()
        .await
        .or_log_default("the calendar store's calendars could not be listed")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn calendar(id: &str, writable: bool) -> CalendarRef {
        CalendarRef {
            id: id.to_owned(),
            name: id.to_owned(),
            color: None,
            writable,
        }
    }

    #[test]
    fn the_chosen_calendar_is_used_while_it_takes_events_else_the_first_that_does() {
        let calendars = [
            calendar("holidays", false),
            calendar("work", true),
            calendar("home", true),
        ];
        // (wanted, the calendar picked)
        let cases = [
            ("home", Some("home")),
            ("", Some("work")),
            ("gone", Some("work")),
            ("holidays", Some("work")),
        ];
        for (wanted, picked) in cases {
            assert_eq!(
                pick(&calendars, wanted).map(|c| c.id.as_str()),
                picked,
                "{wanted:?}"
            );
        }
        assert!(pick(&[calendar("holidays", false)], "").is_none());
        assert!(pick(&[], "").is_none());
    }
}
