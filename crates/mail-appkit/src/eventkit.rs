//! The Mac's calendars, as a [`CalendarStore`]: EventKit's `EKEventStore`, what Calendar.app and
//! every account added in System Settings keep their events in.
//!
//! EventKit takes no iCalendar text, so an event is written from the fields beside it (title,
//! time, place, notes), and the iCalendar the caller holds is not kept. EventKit gives no way to
//! set an event's `UID`, either, so the event carries it in its URL as `urn:uid:<uid>` (RFC
//! 9253's form), and is found by that or, for an event a server put there, by the external
//! identifier EventKit reads the `UID` into. What it cannot say — the attendees, the repetition —
//! is left out rather than written wrong.
//!
//! Each call makes its own `EKEventStore` on a thread of its own and hands the answer back
//! through a oneshot: an `EKEventStore` is used from the thread that made it, and the call that
//! asks the person for access waits on them for as long as they take. Access is asked for by the
//! calls that must have it (listing the calendars and putting an event); finding or removing an
//! event first looks at what was already decided, so answering an invitation never raises the
//! question for someone who has never used their calendar from here.
#![allow(unsafe_code)]

use block2::RcBlock;
use chrono::{Local, NaiveDate, NaiveTime, TimeZone};
use objc2::rc::Retained;
use objc2::runtime::{Bool, NSObjectProtocol};
use objc2::sel;
use objc2_event_kit::{
    EKAuthorizationStatus, EKCalendar, EKEntityType, EKEvent, EKEventStore, EKSpan,
};
use objc2_foundation::{NSArray, NSDate, NSError, NSString, NSURL};
use porter_calendar::{CalendarError, CalendarRef, CalendarStore, Date, Event, Stored, When};
use std::sync::mpsc;

/// How far back and forward [`find`](CalendarStore::find) looks. EventKit answers over a span of
/// at most four years, so an event further off than this is not found, and is added again.
const LOOKBACK_DAYS: i64 = 365;
const LOOKAHEAD_DAYS: i64 = 3 * 365;

/// The calendars of this Mac. Holds nothing: it is cheap to make, and `Send` and `Sync` by it.
#[derive(Debug, Clone, Copy, Default)]
pub struct EventKit;

impl EventKit {
    /// The Mac's calendar store. Asks for nothing until a call needs it.
    pub fn new() -> Self {
        Self
    }
}

/// The URL that names `uid` on an event.
pub(crate) fn urn(uid: &str) -> String {
    format!("urn:uid:{uid}")
}

/// Seconds since the epoch of local midnight on `date`, or of its last second.
pub(crate) fn day_edge(date: Date, last_second: bool) -> Option<i64> {
    let day = NaiveDate::from_ymd_opt(date.year, u32::from(date.month), u32::from(date.day))?;
    let time = if last_second {
        NaiveTime::from_hms_opt(23, 59, 59)?
    } else {
        NaiveTime::MIN
    };
    Local
        .from_local_datetime(&day.and_time(time))
        .earliest()
        .map(|at| at.timestamp())
}

/// The kinds of failure EventKit's answers come to.
fn failed(what: &str, error: &NSError) -> CalendarError {
    CalendarError::Bus(format!("{what}: {}", error.localizedDescription()))
}

/// Run `work` on a thread of its own, with a fresh event store, and await what it returns.
async fn on_a_thread<T: Send + 'static>(
    work: impl FnOnce(&EKEventStore) -> Result<T, CalendarError> + Send + 'static,
) -> Result<T, CalendarError> {
    let (tell, heard) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        // SAFETY: `new` makes an event store, which this thread alone uses and drops.
        let store = unsafe { EKEventStore::new() };
        // A listener that is gone has no use for the answer.
        let _ = tell.send(work(&store));
    });
    heard
        .await
        .unwrap_or_else(|_| Err(CalendarError::Bus("the calendar thread ended".to_owned())))
}

/// What the person has decided about this app reaching their events.
fn status() -> EKAuthorizationStatus {
    // SAFETY: a class method that reads the system's record and touches no object of ours.
    unsafe { EKEventStore::authorizationStatusForEntityType(EKEntityType::Event) }
}

/// Full access to events, asking the person if they have not been asked. Blocks until they have
/// answered.
fn full_access(store: &EKEventStore) -> Result<(), CalendarError> {
    if status() == EKAuthorizationStatus::FullAccess {
        return Ok(());
    }
    let (tell, heard) = mpsc::channel();
    let block = RcBlock::new(move |granted: Bool, _error: *mut NSError| {
        // A thread that stopped waiting has no use for the answer.
        let _ = tell.send(granted.as_bool());
    });
    // SAFETY: the pointer is a live block for the call; EventKit copies it if it keeps it.
    unsafe {
        // macOS 14 split access in two and added this call; before it there is one request,
        // and the bundle still runs on macOS 11.
        if store.respondsToSelector(sel!(requestFullAccessToEventsWithCompletion:)) {
            store.requestFullAccessToEventsWithCompletion(RcBlock::as_ptr(&block));
        } else {
            #[allow(deprecated)]
            store
                .requestAccessToEntityType_completion(EKEntityType::Event, RcBlock::as_ptr(&block));
        }
    }
    match heard.recv() {
        Ok(true) => Ok(()),
        _ => Err(CalendarError::Unauthorized),
    }
}

/// The events calendar, by its identifier: any when `id` is empty.
fn calendar_in(store: &EKEventStore, id: &str) -> Option<Retained<EKCalendar>> {
    if id.is_empty() {
        return None;
    }
    // SAFETY: a lookup by identifier on a store this thread owns.
    unsafe { store.calendarWithIdentifier(&NSString::from_str(id)) }
}

/// The event with `uid` in `calendar` (or anywhere when `calendar` is empty or unknown).
fn locate(store: &EKEventStore, calendar: &str, uid: &str) -> Option<Retained<EKEvent>> {
    let now = chrono::Utc::now().timestamp();
    let day = 86_400;
    // SAFETY: dates, a predicate and a search on a store this thread owns.
    unsafe {
        let from = NSDate::dateWithTimeIntervalSince1970((now - LOOKBACK_DAYS * day) as f64);
        let to = NSDate::dateWithTimeIntervalSince1970((now + LOOKAHEAD_DAYS * day) as f64);
        let only = calendar_in(store, calendar).map(|c| NSArray::from_retained_slice(&[c]));
        let predicate =
            store.predicateForEventsWithStartDate_endDate_calendars(&from, &to, only.as_deref());
        let wanted = urn(uid);
        store
            .eventsMatchingPredicate(&predicate)
            .to_vec()
            .into_iter()
            .find(|event| {
                let by_url = event
                    .URL()
                    .and_then(|url| url.absoluteString())
                    .is_some_and(|url| url.to_string() == wanted);
                let by_external = event
                    .calendarItemExternalIdentifier()
                    .is_some_and(|external| external.to_string() == uid);
                by_url || by_external
            })
    }
}

/// `event` as a [`Stored`] in `calendar`.
fn stored(calendar: &str, uid: &str, event: &EKEvent) -> Stored {
    // SAFETY: a read of a saved event.
    let id = unsafe { event.eventIdentifier() }.map_or_else(String::new, |id| id.to_string());
    Stored {
        calendar: calendar.to_owned(),
        uid: uid.to_owned(),
        id,
        version: None,
        ics: None,
    }
}

impl CalendarStore for EventKit {
    async fn calendars(&self) -> Result<Vec<CalendarRef>, CalendarError> {
        on_a_thread(|store| {
            full_access(store)?;
            // SAFETY: reads of calendars this thread's store holds.
            let all = unsafe { store.calendarsForEntityType(EKEntityType::Event) };
            Ok(all
                .to_vec()
                .into_iter()
                .map(|calendar| unsafe {
                    CalendarRef {
                        id: calendar.calendarIdentifier().to_string(),
                        name: calendar.title().to_string(),
                        color: None,
                        writable: calendar.allowsContentModifications(),
                    }
                })
                .collect())
        })
        .await
    }

    async fn find(&self, calendar: &str, uid: &str) -> Result<Option<Stored>, CalendarError> {
        let (calendar, uid) = (calendar.to_owned(), uid.to_owned());
        on_a_thread(move |store| {
            match status() {
                EKAuthorizationStatus::FullAccess => {}
                // Never asked: nothing of ours can be there, and asking is for a put.
                EKAuthorizationStatus::NotDetermined => return Ok(None),
                _ => return Err(CalendarError::Unauthorized),
            }
            Ok(locate(store, &calendar, &uid).map(|event| stored(&calendar, &uid, &event)))
        })
        .await
    }

    async fn put(&self, calendar: &str, event: &Event) -> Result<Stored, CalendarError> {
        let (calendar, event) = (calendar.to_owned(), event.clone());
        on_a_thread(move |store| {
            full_access(store)?;
            let (start, end, all_day) = match event.when {
                When::Timed { start, end } => (start.0, end.0.max(start.0), false),
                When::AllDay { first, last } => (
                    day_edge(first, false).ok_or_else(date_error)?,
                    day_edge(last, true).ok_or_else(date_error)?,
                    true,
                ),
            };
            // SAFETY: every call below is on an event and a store this thread owns.
            unsafe {
                let target = match locate(store, &calendar, &event.uid) {
                    Some(found) => found,
                    None => EKEvent::eventWithEventStore(store),
                };
                // A new event goes in the calendar asked for; one found stays where it is.
                if target.calendar().is_none() {
                    let home = calendar_in(store, &calendar)
                        .or_else(|| store.defaultCalendarForNewEvents())
                        .ok_or_else(|| {
                            CalendarError::Bus("there is no calendar to add to".to_owned())
                        })?;
                    target.setCalendar(Some(&home));
                }
                target.setTitle(Some(&NSString::from_str(&event.summary)));
                target.setAllDay(all_day);
                target.setStartDate(Some(&NSDate::dateWithTimeIntervalSince1970(start as f64)));
                target.setEndDate(Some(&NSDate::dateWithTimeIntervalSince1970(end as f64)));
                target.setLocation(event.location.as_deref().map(NSString::from_str).as_deref());
                target.setNotes(
                    event
                        .description
                        .as_deref()
                        .map(NSString::from_str)
                        .as_deref(),
                );
                target
                    .setURL(NSURL::URLWithString(&NSString::from_str(&urn(&event.uid))).as_deref());
                store
                    .saveEvent_span_commit_error(&target, EKSpan::ThisEvent, true)
                    .map_err(|e| failed("the event could not be saved", &e))?;
                Ok(stored(&calendar, &event.uid, &target))
            }
        })
        .await
    }

    async fn remove(&self, calendar: &str, uid: &str) -> Result<bool, CalendarError> {
        let (calendar, uid) = (calendar.to_owned(), uid.to_owned());
        on_a_thread(move |store| {
            match status() {
                EKAuthorizationStatus::FullAccess => {}
                EKAuthorizationStatus::NotDetermined => return Ok(false),
                _ => return Err(CalendarError::Unauthorized),
            }
            let Some(found) = locate(store, &calendar, &uid) else {
                return Ok(false);
            };
            // SAFETY: removes an event this thread's store returned.
            unsafe {
                store
                    .removeEvent_span_commit_error(&found, EKSpan::ThisEvent, true)
                    .map_err(|e| failed("the event could not be removed", &e))?;
            }
            Ok(true)
        })
        .await
    }
}

fn date_error() -> CalendarError {
    CalendarError::Malformed("an all-day event on a day that does not exist".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_event_is_named_by_its_uid_as_a_urn() {
        assert_eq!(urn("a@b.test"), "urn:uid:a@b.test");
    }

    #[test]
    fn a_day_runs_from_local_midnight_to_its_last_second() {
        let date = Date {
            year: 2026,
            month: 10,
            day: 5,
        };
        let first = day_edge(date, false).unwrap();
        let last = day_edge(date, true).unwrap();
        // Not 86 399 on a day a clock change shortens or lengthens; within an hour of it.
        assert!((last - first - 86_399).abs() <= 3_600);
        let nonsense = Date {
            year: 2026,
            month: 2,
            day: 30,
        };
        assert_eq!(day_edge(nonsense, false), None);
    }
}
