//! Which application opens `mailto:` links, and asking the system to make that mailo.
//!
//! Two questions, as in [`crate::launcher`]: *what* the window says about it (the alert at
//! launch, the row in Settings › General) is the UI's; *where* the answer comes from is a seam
//! ([`MailHandler`]): the platform's in the launched window ([`platform`]), a fake in the tests,
//! which must never change what the real system opens.
//!
//! What each platform does:
//! - macOS: `NSWorkspace` says which application opens `mailto:` and, on request, asks the
//!   person (the system's own confirmation, "use mailo or keep Mail") to make it mailo
//!   (`workspace`). Only an installed `mailo.app` can be chosen; a `mailo` run from a terminal is
//!   no bundle, and gets no handler, so nothing is offered.
//! - Elsewhere: nothing yet. Linux chooses through the desktop entry (README), Windows through
//!   Default apps.

#[cfg(target_os = "macos")]
mod workspace;

use std::sync::Arc;

/// Which application opens `mailto:` links now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Handler {
    /// mailo does.
    Mailo,
    /// Another application, by the name the system shows for it ("Mail").
    Other(String),
    /// None at all.
    Nobody,
}

/// How a request to become the handler ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Made {
    /// The system made mailo the handler.
    Made,
    /// It did not: the person kept the other application, or the system refused.
    Kept,
}

/// What hears how a request ended. Called once, on any thread.
pub type Done = Box<dyn FnOnce(Made) + Send>;

/// Where the handler is read and changed: the platform's, or a fake in the tests.
pub trait MailHandler: Send + Sync {
    /// The application that opens `mailto:` links now.
    fn current(&self) -> Handler;
    /// Ask the system to make mailo the handler; `done` hears how it ended.
    fn make_mailo(&self, done: Done);
}

/// The window's handler seam, a root context. A window without one (every test that does not
/// hand it a fake) offers nothing: no alert, no row.
#[derive(Clone)]
pub struct MailHandlers(pub Arc<dyn MailHandler>);

impl std::fmt::Debug for MailHandlers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MailHandlers(..)")
    }
}

/// This platform's handler seam, or `None` where mailo cannot be made the handler from inside:
/// off macOS, and on macOS when this process is not an installed `mailo.app`.
pub fn platform() -> Option<MailHandlers> {
    #[cfg(target_os = "macos")]
    {
        workspace::Workspace::of_this_app().map(|found| MailHandlers(Arc::new(found)))
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}
