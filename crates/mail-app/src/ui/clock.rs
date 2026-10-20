//! The wall clock the window measures reminders against.
//!
//! A root context rather than `Utc::now()` where it is read, so a test can hand the window a
//! clock that follows quire's virtual one and see a reminder come due without waiting a day. The
//! launched window has none and reads the system's.
//!
//! And the clock the person reads, twelve hours or twenty-four, which every time of day the
//! window writes follows: the desktop's `[region] hours`, from the settings every window's root
//! already watches, so a change in the desktop's Settings reaches an open window.

use chrono::{DateTime, Utc};
use dioxus::prelude::*;
use mail_core::when::Clock;
use std::sync::Arc;

/// Now, as the window's reminders read it.
#[derive(Clone)]
pub struct WallClock(Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>);

impl std::fmt::Debug for WallClock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("WallClock(..)")
    }
}

impl WallClock {
    /// A clock that reads `now` each time it is asked.
    pub fn new(now: impl Fn() -> DateTime<Utc> + Send + Sync + 'static) -> Self {
        WallClock(Arc::new(now))
    }

    pub fn now(&self) -> DateTime<Utc> {
        (self.0)()
    }
}

/// Now, from the window's [`WallClock`] when it has one, else the system's.
pub(in crate::ui) fn now() -> DateTime<Utc> {
    try_consume_context::<WallClock>().map_or_else(Utc::now, |clock| clock.now())
}

/// How the person keeps the hours of the day: the live desktop settings' `region.hours`
/// (`ds_settings::use_environment`, which every launched window's root provides), Automatic
/// settled by the session's locale. Read in a component it subscribes it, so the window redraws
/// its times when the setting changes.
///
/// Twenty-four hours in a window without them, which is a test's: a test never reads the real
/// config directory, nor the machine's locale.
pub(in crate::ui) fn hours() -> Clock {
    match try_consume_context::<ReadSignal<ds_settings::Environment>>() {
        Some(live) => crate::edge::clock_from(live.read().hours(crate::edge::time_locale())),
        None => Clock::default(),
    }
}
