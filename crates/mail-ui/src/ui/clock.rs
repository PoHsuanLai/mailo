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
use mail_logic::when::Clock;
use std::sync::{Arc, OnceLock};

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
        Some(live) => clock_from(live.read().hours(time_locale())),
        None => Clock::default(),
    }
}

/// The locale name the session writes times in: `LC_ALL`, then `LC_TIME`, then `LANG`, the first
/// set and not empty, else `C` (`ds_settings::time_locale`). Read from the process once; a
/// session does not change its locale under a running program.
pub fn time_locale() -> &'static str {
    static LOCALE: OnceLock<String> = OnceLock::new();
    LOCALE.get_or_init(|| {
        let get = |name: &str| std::env::var(name).ok();
        let (all, time, lang) = (get("LC_ALL"), get("LC_TIME"), get("LANG"));
        ds_settings::time_locale(all.as_deref(), time.as_deref(), lang.as_deref()).to_owned()
    })
}

/// The desktop's settled clock as `mail-core`'s, which knows nothing of the desktop.
pub fn clock_from(hours: ds_settings::ResolvedHours) -> Clock {
    match hours {
        ds_settings::ResolvedHours::H12 => Clock::H12,
        ds_settings::ResolvedHours::H24 => Clock::H24,
    }
}
