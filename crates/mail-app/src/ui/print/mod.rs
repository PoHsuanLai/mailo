//! Printing the open conversation from the window: Print, and Save for printing.
//!
//! The document is [`crate::print`]'s, the one `mailo print` writes, so the window and the command
//! cannot disagree about what a printout holds. Building it reads every message's stored bytes,
//! so it is done on a blocking thread started by the click or the key that asked for it, and never
//! from a component body (F140).
//!
//! On `webview`, Print shows the document in a window of its own and puts the print dialog over it
//! ([`window`]); the application's own page is never printed and never holds the mail. Save for
//! printing writes the same document, as HTML, into the downloads directory, beside where an
//! attachment goes, and says where.
//!
//! On `native` there is no webview anywhere. The document becomes a PDF through quire
//! (`ds_native::pdf`, [`paper`]), and Print hands that PDF to the system's print dialog
//! (`ds_native::print_dialog`, through a [`Printer`], which a test replaces). Save for printing
//! writes the same PDF.

mod tool;
// The print window is a WebKitGTK webview of its own, and the print operation is WebKit's: the
// `webview` frontend's alone. It goes when the webview does.
#[cfg(feature = "webview")]
mod window;
// The printout as a PDF, for `native`.
#[cfg(feature = "native")]
mod paper;

pub(super) use tool::PrintTool;

#[cfg(feature = "webview")]
use super::motion::Motion;
use super::motion::{motion, tell_through};
use crate::print::Printed;
use chrono::{DateTime, TimeZone, Utc};
use dioxus::prelude::*;
use mail_domain::ThreadId;
use mail_mime::Pages;
use mail_store::SqliteStore;
use std::path::Path;
use std::sync::Arc;

#[cfg(feature = "native")]
pub use native_print::Printer;
#[cfg(feature = "native")]
use native_print::{print_on_paper, printed_bytes};

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

/// What Save for printing writes: HTML on `webview`, which prints it from a browser.
#[cfg(feature = "webview")]
fn printed_bytes(printed: Printed) -> Result<(String, Vec<u8>), String> {
    Ok((printed.subject, printed.html.into_bytes()))
}

/// The file Save for printing writes ends in this.
#[cfg(feature = "webview")]
pub(in crate::ui) const SAVED_AS: &str = "html";
#[cfg(feature = "native")]
pub(in crate::ui) const SAVED_AS: &str = "pdf";

/// Build `job` and write it into `dir` beside anything already there, and say where it went.
/// Blocking. A PDF on `native`, the HTML document on `webview`.
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
    let saved = build(store, job, zone, now)
        .and_then(printed_bytes)
        .and_then(|(subject, bytes)| {
            crate::print::write_file_into(dir, &subject, SAVED_AS, &bytes)
        });
    match saved {
        Ok(path) => format!("Saved for printing to {}", path.display()),
        Err(why) => {
            eprintln!("save for printing: {why}");
            format!("Could not save for printing: {why}")
        }
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

/// Print `job`: build it off this thread, then hand it to the print dialog.
///
/// Call from an event handler. The work is a task of the window's rather than of the component
/// that asked, so leaving the conversation does not strand a print half made.
pub(in crate::ui) fn print(job: Job) {
    #[cfg(test)]
    STARTED.with(|started| started.borrow_mut().push(job));
    #[cfg(feature = "webview")]
    print_in_window(job);
    #[cfg(feature = "native")]
    print_on_paper(job);
}

/// [`print`] on the webview: the document in a print window of its own.
#[cfg(feature = "webview")]
fn print_in_window(job: Job) {
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
#[cfg(feature = "webview")]
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

/// Print on `native`: the PDF, the dialog, and the seam between them.
#[cfg(feature = "native")]
mod native_print {
    use super::super::motion::{motion, tell_through};
    use super::paper::{self, Paper};
    use super::{Job, build};
    use crate::print::Printed;
    use chrono::{DateTime, TimeZone, Utc};
    use dioxus::prelude::*;
    use ds_native::{PrintError, PrintOutcome};
    use mail_store::SqliteStore;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// The call that puts a PDF in front of the person: `(pdf, title)`.
    type Dialog = dyn Fn(&[u8], &str) -> Result<PrintOutcome, PrintError> + Send + Sync;

    /// How a printout reaches paper: the system's print dialog (`ds_native::print_dialog`), or,
    /// in a test, whatever the test puts in its place. A root context: the window without one
    /// uses the system's.
    ///
    /// It also holds whether a print is under way, so a second Print while the first one's
    /// dialog is up says so and starts nothing.
    #[derive(Clone)]
    pub struct Printer {
        dialog: Arc<Dialog>,
        busy: Arc<AtomicBool>,
    }

    impl std::fmt::Debug for Printer {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("Printer")
                .field("busy", &self.busy.load(Ordering::Relaxed))
                .finish_non_exhaustive()
        }
    }

    impl Printer {
        /// A printer whose dialog is `dialog`, called with the PDF and its title off the UI
        /// thread, and answering as `ds_native::print_dialog` would. For tests: no real dialog.
        pub fn with_dialog(
            dialog: impl Fn(&[u8], &str) -> Result<PrintOutcome, PrintError> + Send + Sync + 'static,
        ) -> Printer {
            Printer {
                dialog: Arc::new(dialog),
                busy: Arc::new(AtomicBool::new(false)),
            }
        }

        /// The system's print dialog, one print at a time across the process. In this crate's
        /// tests, a dialog that refuses: a window drawn by a test never opens a real one.
        fn system() -> Printer {
            static BUSY: std::sync::LazyLock<Arc<AtomicBool>> =
                std::sync::LazyLock::new(|| Arc::new(AtomicBool::new(false)));
            if cfg!(test) {
                return Printer::with_dialog(|_, _| {
                    Err(PrintError::NoViewer(
                        "there is no print dialog in tests".to_owned(),
                    ))
                });
            }
            Printer {
                dialog: Arc::new(ds_native::print_dialog),
                busy: Arc::clone(&BUSY),
            }
        }

        /// Claim this printer for one print, or `None` while another holds it.
        pub(in crate::ui) fn claim(&self) -> Option<Claim> {
            (!self.busy.swap(true, Ordering::AcqRel)).then(|| Claim(Arc::clone(&self.busy)))
        }
    }

    /// A print under way. Dropping it, however the print ended (a panic included), frees the
    /// printer.
    pub(in crate::ui) struct Claim(Arc<AtomicBool>);

    impl Drop for Claim {
        fn drop(&mut self) {
            self.0.store(false, Ordering::Release);
        }
    }

    /// What a second Print says while the first is under way.
    pub(in crate::ui) const BUSY: &str =
        "A printout is already being made; finish with its print dialog first.";

    /// [`super::print`] on `native`: the PDF built and handed to the dialog on a blocking thread
    /// (the layout is slow next to a click, and the dialog blocks until it is answered), and
    /// what became of it said in a toast.
    pub(super) fn print_on_paper(job: Job) {
        let said = motion();
        let printer = try_consume_context::<Printer>().unwrap_or_else(Printer::system);
        let Some(claim) = printer.claim() else {
            tell_through(said, BUSY.to_owned());
            return;
        };
        let store = consume_context::<Arc<SqliteStore>>();
        let paper = Paper::from_env();
        dioxus::core::spawn_forever(async move {
            let done = tokio::task::spawn_blocking(move || {
                let _claim = claim;
                print_through(&store, job, &paper, &printer, &chrono::Local, Utc::now())
            })
            .await;
            let words = done.unwrap_or_else(|error| {
                eprintln!("print: {error}");
                format!("The printout stopped before it was made: {error}")
            });
            tell_through(said, words);
        });
    }

    /// Build `job`, make it a PDF on `paper`, hand it to `printer`'s dialog, and say what became
    /// of it. Blocking, for as long as the dialog is up.
    pub(in crate::ui) fn print_through<Tz>(
        store: &SqliteStore,
        job: Job,
        paper: &Paper,
        printer: &Printer,
        zone: &Tz,
        now: DateTime<Utc>,
    ) -> String
    where
        Tz: TimeZone,
        Tz::Offset: std::fmt::Display,
    {
        let made = build(store, job, zone, now).and_then(|printed| {
            let pdf = paper::pdf(&printed, paper)?;
            Ok((paper::title(&printed), pdf))
        });
        match made {
            Ok((title, pdf)) => said((printer.dialog)(&pdf, &title)),
            Err(why) => {
                eprintln!("print: {why}");
                format!("Could not print: {why}")
            }
        }
    }

    /// What the toast says of how a print ended.
    pub(in crate::ui) fn said(outcome: Result<PrintOutcome, PrintError>) -> String {
        match outcome {
            Ok(PrintOutcome::Printed) => "Sent to the printer.".to_owned(),
            Ok(PrintOutcome::Cancelled) => "Printing cancelled; nothing was printed.".to_owned(),
            Ok(PrintOutcome::Opened(path)) => format!(
                "There is no print dialog here, so the printout opened in your PDF viewer \
                 to print from there: {}",
                path.display()
            ),
            Err(why) => {
                eprintln!("print: {why}");
                format!("Could not print: {why}")
            }
        }
    }

    /// What Save for printing writes on `native`: the PDF, on the locale's paper.
    pub(super) fn printed_bytes(printed: Printed) -> Result<(String, Vec<u8>), String> {
        let pdf = paper::pdf(&printed, &Paper::from_env())?;
        Ok((printed.subject, pdf))
    }
}

#[cfg(all(test, feature = "native"))]
#[path = "pdf_tests.rs"]
mod pdf_tests;
#[cfg(test)]
mod tests;
