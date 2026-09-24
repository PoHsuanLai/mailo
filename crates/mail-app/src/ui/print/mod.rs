//! Printing the open conversation from the window: Print, and Save for printing.
//!
//! The document is [`crate::print`]'s, the one `mailo print` writes, so the window and the command
//! cannot disagree about what a printout holds. Building it reads every message's stored bytes,
//! so it is done on a blocking thread started by the click or the key that asked for it, and never
//! from a component body (F140).
//!
//! Print shows the document in a window of its own and puts the print dialog over it
//! ([`window`]); the application's own page is never printed and never holds the mail. Save for
//! printing writes the same document into the downloads directory, beside where an attachment
//! goes, and says where.

mod tool;
mod window;

pub(super) use tool::PrintTool;

use super::motion::{Motion, motion, tell_through};
use crate::print::Printed;
use chrono::{DateTime, TimeZone, Utc};
use dioxus::prelude::*;
use mail_domain::ThreadId;
use mail_mime::Pages;
use mail_store::SqliteStore;
use std::path::Path;
use std::sync::Arc;

/// What to print: a conversation, and whether each message starts a page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) struct Job {
    pub thread: ThreadId,
    pub pages: Pages,
}

/// What Ctrl P, or the menu's Print, prints: the open conversation as one flow. With nothing
/// open there is nothing to print, and nothing is said.
pub(in crate::ui) fn job_for(open: Option<ThreadId>) -> Option<Job> {
    open.map(|thread| Job {
        thread,
        pages: Pages::Flow,
    })
}

/// The printable document for `job`, dates in `zone`. Blocking: it reads the stored mail.
pub(in crate::ui) fn build<Tz>(
    store: &SqliteStore,
    job: Job,
    zone: &Tz,
    now: DateTime<Utc>,
) -> Result<Printed, String>
where
    Tz: TimeZone,
    Tz::Offset: std::fmt::Display,
{
    crate::print::document(store, *job.thread.as_uuid(), zone, now, job.pages)
}

/// Build `job` and write it into `dir` beside anything already there, and say where it went.
/// Blocking.
pub(in crate::ui) fn save_into<Tz>(
    store: &SqliteStore,
    job: Job,
    dir: &Path,
    zone: &Tz,
    now: DateTime<Utc>,
) -> String
where
    Tz: TimeZone,
    Tz::Offset: std::fmt::Display,
{
    match build(store, job, zone, now).and_then(|printed| crate::print::write_into(dir, &printed)) {
        Ok(path) => format!("Saved for printing to {}", path.display()),
        Err(why) => format!("Could not save for printing: {why}"),
    }
}

#[cfg(test)]
thread_local! {
    static STARTED: std::cell::RefCell<Vec<Job>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Every print started on this thread, in order. A test's own window has no desktop to open a
/// print window on, so this is where it looks.
#[cfg(test)]
pub(in crate::ui) fn started() -> Vec<Job> {
    STARTED.with(|started| started.borrow().clone())
}

/// Print `job`: build it off this thread, then open it in its own window with the dialog over it.
///
/// Call from an event handler. The work is a task of the window's rather than of the component
/// that asked, so leaving the conversation does not strand a print window half made.
pub(in crate::ui) fn print(job: Job) {
    #[cfg(test)]
    STARTED.with(|started| started.borrow_mut().push(job));
    let said = motion();
    if window::busy() {
        tell_through(said, "A printout is already open.".to_owned());
        return;
    }
    let store = consume_context::<Arc<SqliteStore>>();
    let desktop = try_consume_context::<dioxus::desktop::DesktopContext>();
    dioxus::core::spawn_forever(async move {
        let Some(printed) = built(said, store, job).await else {
            return;
        };
        let Some(desktop) = desktop else {
            tell_through(
                said,
                "There is no window to print from here; Save for printing works anywhere."
                    .to_owned(),
            );
            return;
        };
        let title = format!("Print · {}", printed.subject);
        if let Err(why) = window::print(desktop, printed.html, title).await {
            tell_through(said, why);
        }
    });
}

/// [`build`] on a blocking thread, or `None` once the reason it failed has been said.
async fn built(said: Option<Motion>, store: Arc<SqliteStore>, job: Job) -> Option<Printed> {
    let done =
        tokio::task::spawn_blocking(move || build(&store, job, &chrono::Local, Utc::now())).await;
    match done {
        Ok(Ok(printed)) => Some(printed),
        Ok(Err(why)) => {
            tell_through(said, format!("Could not print: {why}"));
            None
        }
        Err(error) => {
            tell_through(
                said,
                format!("The printout stopped before it was made: {error}"),
            );
            None
        }
    }
}

/// Save `job` for printing into the downloads directory, off this thread, and say where.
pub(in crate::ui) fn save(job: Job) {
    let said = motion();
    let store = consume_context::<Arc<SqliteStore>>();
    let dir = crate::attach::downloads_dir();
    dioxus::core::spawn_forever(async move {
        let done = tokio::task::spawn_blocking(move || {
            save_into(&store, job, &dir, &chrono::Local, Utc::now())
        })
        .await;
        tell_through(
            said,
            done.unwrap_or_else(|error| format!("The save stopped before it finished: {error}")),
        );
    });
}

#[cfg(test)]
mod tests;
