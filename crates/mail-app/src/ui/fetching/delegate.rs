//! Who schedules fetching, as the window asks it.
//!
//! Where a `mailo watch` runs it is the session's sync engine, and the window leaves scheduled
//! fetching to it and keeps the requests a person makes (`mail_core::schedule::Schedule` has
//! the rule and why). Asking whether one runs is the window's: [`Delegate`].
//!
//! The mail the watch stores is not the window's doing: [`super::external`] tells the window that
//! something it did not write has landed.

use mail_core::schedule::Schedule;
use std::sync::Arc;

/// What reports who schedules. The real one unless a test provided its own, as
/// [`super::Passer`] is.
#[derive(Clone)]
pub(in crate::ui) struct Delegate(pub Arc<dyn Fn() -> Schedule + Send + Sync>);

impl Delegate {
    /// Asks the watch's lock ([`mail_core::ipc::watching::running`]), every time it is asked: a
    /// watch may start or stop while the window is open.
    #[cfg(not(test))]
    pub(in crate::ui) fn server() -> Self {
        Self(Arc::new(|| {
            if mail_core::ipc::watching::running() {
                Schedule::Watch
            } else {
                Schedule::Window
            }
        }))
    }

    /// Not in a test build: the person's real watch, if there is one, must not change what a
    /// test sees.
    #[cfg(test)]
    pub(in crate::ui) fn server() -> Self {
        Self(Arc::new(|| Schedule::Window))
    }

    /// Who schedules now.
    pub(in crate::ui) fn schedule(&self) -> Schedule {
        (self.0)()
    }
}

#[cfg(test)]
mod tests;
