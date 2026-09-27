//! The wall clock the window measures reminders against.
//!
//! A root context rather than `Utc::now()` where it is read, so a test can hand the window a
//! clock that follows quire's virtual one and see a reminder come due without waiting a day. The
//! launched window has none and reads the system's.

use chrono::{DateTime, Utc};
use dioxus::prelude::*;
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
