//! A calendar that lives in memory, for the tests: every call is recorded and nothing leaves the
//! process, so no test reaches a real calendar.

use porter_calendar::{CalendarError, CalendarRef, CalendarStore, Event, Stored};
use std::sync::Mutex;

/// An in-memory [`CalendarStore`].
#[derive(Debug)]
pub(crate) struct Fake {
    pub calendars: Vec<CalendarRef>,
    /// Each event with the calendar it is in, in the order put.
    pub events: Mutex<Vec<(String, Event)>>,
}

impl Fake {
    /// Two calendars: a read-only one first, then a writable "Work".
    pub fn new() -> Self {
        let calendar = |id: &str, name: &str, writable| CalendarRef {
            id: id.to_owned(),
            name: name.to_owned(),
            color: None,
            writable,
        };
        Self {
            calendars: vec![
                calendar("holidays", "Holidays", false),
                calendar("work", "Work", true),
                calendar("home", "Home", true),
            ],
            events: Mutex::new(Vec::new()),
        }
    }

    /// The events held, as `(calendar, uid)`.
    pub fn held(&self) -> Vec<(String, String)> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .map(|(calendar, event)| (calendar.clone(), event.uid.clone()))
            .collect()
    }

    /// The event with `uid`.
    pub fn event(&self, uid: &str) -> Option<Event> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .find(|(_, event)| event.uid == uid)
            .map(|(_, event)| event.clone())
    }
}

fn stored(calendar: &str, event: &Event) -> Stored {
    Stored {
        calendar: calendar.to_owned(),
        uid: event.uid.clone(),
        id: event.uid.clone(),
        version: None,
        ics: Some(event.ics.clone()),
    }
}

impl CalendarStore for Fake {
    async fn calendars(&self) -> Result<Vec<CalendarRef>, CalendarError> {
        Ok(self.calendars.clone())
    }

    async fn find(&self, calendar: &str, uid: &str) -> Result<Option<Stored>, CalendarError> {
        Ok(self
            .events
            .lock()
            .unwrap()
            .iter()
            .find(|(held, event)| held == calendar && event.uid == uid)
            .map(|(held, event)| stored(held, event)))
    }

    async fn put(&self, calendar: &str, event: &Event) -> Result<Stored, CalendarError> {
        let mut events = self.events.lock().unwrap();
        match events
            .iter_mut()
            .find(|(held, old)| held == calendar && old.uid == event.uid)
        {
            Some(slot) => slot.1 = event.clone(),
            None => events.push((calendar.to_owned(), event.clone())),
        }
        Ok(stored(calendar, event))
    }

    async fn remove(&self, calendar: &str, uid: &str) -> Result<bool, CalendarError> {
        let mut events = self.events.lock().unwrap();
        let before = events.len();
        events.retain(|(held, event)| !(held == calendar && event.uid == uid));
        Ok(events.len() != before)
    }
}
