//! What the invitation card's buttons and the answer do to the calendar, with no window in it:
//! a [`Calendars`] and the store in, an outcome or a sentence of why not out. The window draws
//! the words; this decides what happens, so a test can run it against a fake calendar.
//!
//! Nothing here runs unless a person pressed a button: reading a message never reaches a
//! calendar, which may ask them for access the first time it is reached.

use super::{Calendars, handoff};
use chrono::Local;
use mail_core::SqliteStore;
use mail_core::invite::{self, Followed, InviteState};
use mail_domain::{Attendance, MessageId};
use mail_pim::Kind;
use porter_calendar::{CalendarRef, CalendarStore};
use std::sync::Arc;

/// Where Add to Calendar put the event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Added {
    /// In this calendar, by the name the store gives it.
    In(String),
    /// With the default calendar application, which does the adding.
    HandedOver,
}

/// What an answer did to the calendar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kept {
    /// The event that was in the calendar says the new answer.
    Updated(String),
    /// The declined event is out of the calendar.
    Removed(String),
    /// The event was not in the calendar, and an answer does not add it.
    NotThere,
}

/// Read the message on a blocking thread: a store read is not for the thread that draws.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, mail_core::CoreError> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| e.to_string())?
        .map_err(String::from)
}

/// The calendar to add to: the one `wanted`, else the first that takes events.
async fn target(calendars: &Calendars, wanted: &str) -> Result<CalendarRef, String> {
    let listed = invite::calendars(calendars).await;
    invite::pick(&listed, wanted)
        .cloned()
        .ok_or_else(|| crate::ui::copy::errors::NO_CALENDAR_TO_ADD_TO.to_owned())
}

/// The calendar (of those that take events) that holds the event `uid`, with its name.
async fn holder(calendars: &Calendars, uid: &str) -> Option<CalendarRef> {
    for calendar in invite::calendars(calendars).await {
        if !calendar.writable {
            continue;
        }
        match calendars.find(&calendar.id, uid).await {
            Ok(Some(_)) => return Some(calendar),
            Ok(None) => {}
            Err(why) => {
                log::warn!(
                    "the calendar {} could not be searched: {why}",
                    calendar.name
                );
            }
        }
    }
    None
}

/// Add to Calendar: put `message`'s event in the calendar `wanted` names (or the first there is),
/// as the reader answered it if they have.
pub async fn add(
    calendars: &Calendars,
    store: &Arc<SqliteStore>,
    message: MessageId,
    wanted: &str,
) -> Result<Added, String> {
    let read = store.clone();
    let event = blocking(move || {
        let answered = match invite::state(&read, &mail_core::Store::message(&*read, message)?)? {
            InviteState::Shown { invite, answered } => answered
                .filter(|answer| answer.sequence >= invite.sequence)
                .map(|answer| answer.attendance),
            _ => None,
        };
        invite::event_of(&read, message, answered, &Local)
    })
    .await?;
    let calendar = target(calendars, wanted).await?;
    invite::add(calendars, &calendar.id, &event)
        .await
        .map_err(String::from)?;
    Ok(if calendar.id == handoff::APPLICATION {
        Added::HandedOver
    } else {
        Added::In(calendar.name)
    })
}

/// Bring the calendar in step with the answer just given to `message`'s invitation: an event
/// already in it says the answer, or goes if it was declined. One that is not there stays out.
pub async fn follow(
    calendars: &Calendars,
    store: &Arc<SqliteStore>,
    message: MessageId,
    attendance: Attendance,
) -> Result<Kept, String> {
    let read = store.clone();
    let event =
        blocking(move || invite::event_of(&read, message, Some(attendance), &Local)).await?;
    let Some(calendar) = holder(calendars, &event.uid).await else {
        return Ok(Kept::NotThere);
    };
    Ok(
        match invite::follow_answer(calendars, &calendar.id, &event, attendance)
            .await
            .map_err(String::from)?
        {
            Followed::Kept(_) => Kept::Updated(calendar.name),
            Followed::Removed => Kept::Removed(calendar.name),
            Followed::Absent => Kept::NotThere,
        },
    )
}

/// Remove from Calendar: take the event a cancellation names out of the calendar that has it.
/// The name of that calendar, or `None` when none has it.
pub async fn cancel(
    calendars: &Calendars,
    store: &Arc<SqliteStore>,
    message: MessageId,
) -> Result<Option<String>, String> {
    let read = store.clone();
    let uid = blocking(move || {
        match invite::state(&read, &mail_core::Store::message(&*read, message)?)? {
            InviteState::Shown { invite, .. } if invite.kind == Kind::Cancelled => Ok(invite.uid),
            _ => Err(mail_core::CoreError::NoInvitation),
        }
    })
    .await?;
    let Some(calendar) = holder(calendars, &uid).await else {
        return Ok(None);
    };
    invite::follow_cancellation(calendars, &calendar.id, &uid)
        .await
        .map_err(String::from)?;
    Ok(Some(calendar.name))
}

/// A time the person typed, `2026-11-02 14:30`, in their own zone, as an instant. `None` for
/// anything else, and for a clock reading their zone skips or the date does not have.
pub fn local_time(text: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    use chrono::TimeZone;
    let at = chrono::NaiveDateTime::parse_from_str(text.trim(), "%Y-%m-%d %H:%M").ok()?;
    Local
        .from_local_datetime(&at)
        .earliest()
        .map(|at| at.with_timezone(&chrono::Utc))
}
