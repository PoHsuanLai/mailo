//! The calendar mailo puts invitations in, and where it comes from.
//!
//! The same two questions as [`crate::mail_handler`]: *what* the window says about it is the
//! UI's; *where* the calendar is comes from a seam ([`Desk`]), which is the platform's in the
//! launched window ([`platform`]) and a fake in the tests, which must never reach a real
//! calendar. The calendar itself is [`porter_calendar::CalendarStore`], the trait every porter
//! client shares; [`Desk`] is the same four calls made object-safe so one can be a root context.
//!
//! What each platform does:
//! - macOS: EventKit, through `mail-appkit` (the one crate that may use `unsafe`). The system
//!   asks the person for access the first time a calendar is read or written.
//! - Elsewhere: [`handoff::Handoff`], which writes the event as an `.ics` file and hands it to
//!   the default calendar application. It cannot list calendars or find or remove an event.

#[cfg(test)]
pub(crate) mod fake;
pub mod flow;
pub mod handoff;

use porter_calendar::{CalendarError, CalendarRef, CalendarStore, Event, Stored};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// A future the seam hands back: boxed, so [`Desk`] can be a trait object.
pub type Boxed<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// [`CalendarStore`], object-safe. Every store is one: implement the porter trait, and this
/// comes with it.
pub trait Desk: Send + Sync {
    /// Every calendar the store holds.
    fn calendars(&self) -> Boxed<'_, Result<Vec<CalendarRef>, CalendarError>>;
    /// The event with `uid` in `calendar`.
    fn find<'a>(
        &'a self,
        calendar: &'a str,
        uid: &'a str,
    ) -> Boxed<'a, Result<Option<Stored>, CalendarError>>;
    /// Put `event` into `calendar`.
    fn put<'a>(
        &'a self,
        calendar: &'a str,
        event: &'a Event,
    ) -> Boxed<'a, Result<Stored, CalendarError>>;
    /// Take the event with `uid` out of `calendar`.
    fn remove<'a>(
        &'a self,
        calendar: &'a str,
        uid: &'a str,
    ) -> Boxed<'a, Result<bool, CalendarError>>;
}

impl<S: CalendarStore> Desk for S {
    fn calendars(&self) -> Boxed<'_, Result<Vec<CalendarRef>, CalendarError>> {
        Box::pin(CalendarStore::calendars(self))
    }

    fn find<'a>(
        &'a self,
        calendar: &'a str,
        uid: &'a str,
    ) -> Boxed<'a, Result<Option<Stored>, CalendarError>> {
        Box::pin(CalendarStore::find(self, calendar, uid))
    }

    fn put<'a>(
        &'a self,
        calendar: &'a str,
        event: &'a Event,
    ) -> Boxed<'a, Result<Stored, CalendarError>> {
        Box::pin(CalendarStore::put(self, calendar, event))
    }

    fn remove<'a>(
        &'a self,
        calendar: &'a str,
        uid: &'a str,
    ) -> Boxed<'a, Result<bool, CalendarError>> {
        Box::pin(CalendarStore::remove(self, calendar, uid))
    }
}

/// The window's calendar, a root context. A window without one (every test that does not hand it
/// a fake) offers no Add to Calendar: nothing reaches a calendar by accident.
#[derive(Clone)]
pub struct Calendars(pub Arc<dyn Desk>);

impl std::fmt::Debug for Calendars {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Calendars(..)")
    }
}

/// This platform's calendar: EventKit on macOS, the default calendar application elsewhere.
pub fn platform() -> Option<Calendars> {
    #[cfg(target_os = "macos")]
    {
        Some(Calendars(Arc::new(mail_appkit::EventKit::new())))
    }
    #[cfg(not(target_os = "macos"))]
    {
        Some(Calendars(Arc::new(handoff::Handoff::new(
            handoff::default_application(),
        ))))
    }
}

impl CalendarStore for Calendars {
    async fn calendars(&self) -> Result<Vec<CalendarRef>, CalendarError> {
        self.0.calendars().await
    }

    async fn find(&self, calendar: &str, uid: &str) -> Result<Option<Stored>, CalendarError> {
        self.0.find(calendar, uid).await
    }

    async fn put(&self, calendar: &str, event: &Event) -> Result<Stored, CalendarError> {
        self.0.put(calendar, event).await
    }

    async fn remove(&self, calendar: &str, uid: &str) -> Result<bool, CalendarError> {
        self.0.remove(calendar, uid).await
    }
}
