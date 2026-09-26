//! Printing the open conversation from the window: Print, and Save for printing.
//!
//! The document is [`crate::print`]'s, the one `mailo print` writes, so the window and the command
//! cannot disagree about what a printout holds. Building it reads every message's stored bytes,
//! so it is done on a blocking thread started by the click or the key that asked for it, and never
//! from a component body (F140).
//!
//! There is no webview anywhere. The document becomes a PDF through quire (`ds_native::pdf`,
//! [`paper`]), and Print hands that PDF to the system's print dialog (`ds_native::print_dialog`,
//! through a [`Printer`], which a test replaces). Save for printing writes the same PDF into the
//! downloads directory, beside where an attachment goes, and says where. A remote image prints
//! only when the reader's consent covers its message as the PDF is made; it is then fetched
//! through the Reader view's own fetcher, on the same blocking thread, and drawn from the bytes
//! as a `data:` URI ([`Sources`]). Without that consent nothing is fetched, and the image is
//! named where it stood.

mod tool;
// The printout as a PDF.
mod paper;

pub(super) use tool::PrintTool;

use super::motion::{motion, tell_through};
#[cfg(test)]
use crate::print::Printed;
use chrono::{DateTime, TimeZone, Utc};
use dioxus::prelude::*;
use mail_domain::ThreadId;
use mail_mime::Pages;
use mail_store::SqliteStore;
use std::path::Path;
use std::sync::Arc;

pub use native_print::Printer;
pub(in crate::ui) use native_print::Sources;
use native_print::{print_on_paper, saved_bytes};

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
///
/// The document `mailo print` writes. The window's printout is
/// [`paper::printed`], the same document with its faces named for paper.
#[cfg(test)]
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

/// The file Save for printing writes ends in this.
pub(in crate::ui) const SAVED_AS: &str = "pdf";

/// Build `job` and write it into `dir` beside anything already there, and say where it went.
/// Blocking. A PDF, with the remote images `sources` consents to.
pub(in crate::ui) fn save_into<Tz>(
    store: &SqliteStore,
    job: Job,
    sources: &Sources,
    dir: &Path,
    zone: &Tz,
    now: DateTime<Utc>,
) -> String
where
    Tz: TimeZone,
    Tz::Offset: std::fmt::Display,
{
    let saved = saved_bytes(store, job, sources, zone, now).and_then(|(subject, bytes)| {
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
    print_on_paper(job);
}

/// Save `job` for printing into the downloads directory, off this thread, and say where.
pub(in crate::ui) fn save(job: Job) {
    let said = motion();
    let store = consume_context::<Arc<SqliteStore>>();
    let dir = crate::attach::downloads_dir();
    let sources = Sources::window();
    dioxus::core::spawn_forever(async move {
        let done = tokio::task::spawn_blocking(move || {
            save_into(&store, job, &sources, &dir, &chrono::Local, Utc::now())
        })
        .await;
        tell_through(
            said,
            done.unwrap_or_else(|error| format!("The save stopped before it finished: {error}")),
        );
    });
}

/// Print: the PDF, the dialog, and the seam between them.
mod native_print {
    use super::super::motion::{motion, tell_through};
    use super::super::original::{Consent, FetchImage, ReaderNet, data_uri};
    use super::Job;
    use super::paper::{self, Paper};
    use crate::print::{Pictures, Printed};
    use chrono::{DateTime, TimeZone, Utc};
    use dioxus::prelude::*;
    use ds_native::{PrintError, PrintOutcome};
    use mail_domain::MessageId;
    use mail_store::SqliteStore;
    use std::collections::BTreeMap;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};

    /// Where a printout's consented remote images come from: the reader's [`Consent`], which
    /// says whose images may be fetched, and the [`ReaderNet`] the Reader view fetches its own
    /// with (no cookies, no `Referer`, its limits). Both are root contexts of the window; a test
    /// builds its own. Without either, a printout fetches nothing.
    #[derive(Clone, Default)]
    pub(in crate::ui) struct Sources {
        consent: Option<Consent>,
        net: Option<ReaderNet>,
    }

    impl std::fmt::Debug for Sources {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("Sources")
                .field("consent", &self.consent)
                .field("net", &self.net.is_some())
                .finish()
        }
    }

    impl Sources {
        /// `consent`, fetching through `net`.
        #[cfg(test)]
        pub(in crate::ui) fn new(consent: Consent, net: ReaderNet) -> Sources {
            Sources {
                consent: Some(consent),
                net: Some(net),
            }
        }

        /// No consent and no fetcher, for a test: nothing is fetched.
        #[cfg(test)]
        pub(in crate::ui) fn none() -> Sources {
            Sources::default()
        }

        /// The window's. From an event handler, where the print was asked for.
        pub(super) fn window() -> Sources {
            Sources {
                consent: try_consume_context::<Consent>(),
                net: try_consume_context::<ReaderNet>(),
            }
        }
    }

    /// How long a printout waits, in all, for its consented images. One that has not come by
    /// then is named, as one that failed is. The fetcher gives up on its own after twenty
    /// seconds.
    const WAIT: Duration = Duration::from_secs(25);

    /// `job`'s printout on `paper`, with the remote images of the messages whose images the
    /// reader consented to, when `sources` can fetch them. Blocking, for as long as the fetches
    /// take (at most [`WAIT`]).
    ///
    /// Asked of the consent here, on the blocking thread, and not when the click came: what is
    /// printed is what stands when the print is made. Only the messages the grant covers are
    /// fetched for, only the images their printout draws (`mail_mime::remote_images`), and if
    /// the grant is taken back while they come, none of them is drawn.
    pub(in crate::ui) fn made<Tz>(
        store: &SqliteStore,
        job: Job,
        paper: &Paper,
        sources: &Sources,
        zone: &Tz,
        now: DateTime<Utc>,
    ) -> Result<Printed, String>
    where
        Tz: TimeZone,
        Tz::Offset: std::fmt::Display,
    {
        let (Some(consent), Some(net)) = (&sources.consent, &sources.net) else {
            return paper::printed(store, job, paper, None, zone, now);
        };
        let Some((ticket, messages)) = consent.thread(job.thread) else {
            return paper::printed(store, job, paper, None, zone, now);
        };
        let consented = |id: MessageId| messages.contains(&id);
        let fetch = |urls: &[String]| {
            let fetched = fetch_all(net.0.as_ref(), urls, WAIT);
            if consent.stands(ticket) {
                fetched
            } else {
                BTreeMap::new()
            }
        };
        let pictures = Pictures {
            consented: &consented,
            fetch: &fetch,
        };
        paper::printed(store, job, paper, Some(&pictures), zone, now)
    }

    /// Fetch `urls` through `net`, all at once, and wait at most `wait` for them: those that
    /// came back a raster image to show, as `data:` URIs ([`data_uri`], as the Reader view draws
    /// them). A fetch that never answers (the fetcher drops it) ends the wait for it early.
    fn fetch_all(
        net: &dyn FetchImage,
        urls: &[String],
        wait: Duration,
    ) -> BTreeMap<String, String> {
        let deadline = Instant::now() + wait;
        let (send, answers) = std::sync::mpsc::channel();
        for url in urls {
            let send = send.clone();
            let asked = url.clone();
            net.get(
                url.clone(),
                Box::new(move |got| {
                    let _ = send.send((asked, got));
                }),
            );
        }
        drop(send);
        let mut fetched = BTreeMap::new();
        for _ in urls {
            let left = deadline.saturating_duration_since(Instant::now());
            let Ok((url, got)) = answers.recv_timeout(left) else {
                break;
            };
            if let Some(uri) = data_uri(&got) {
                fetched.insert(url, uri);
            }
        }
        fetched
    }

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

    /// [`super::print`]: the PDF built and handed to the dialog on a blocking thread
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
        let sources = Sources::window();
        dioxus::core::spawn_forever(async move {
            let done = tokio::task::spawn_blocking(move || {
                let _claim = claim;
                print_through(
                    &store,
                    job,
                    &paper,
                    &printer,
                    &sources,
                    &chrono::Local,
                    Utc::now(),
                )
            })
            .await;
            let words = done.unwrap_or_else(|error| {
                eprintln!("print: {error}");
                format!("The printout stopped before it was made: {error}")
            });
            tell_through(said, words);
        });
    }

    /// Build `job` with the remote images `sources` consents to, make it a PDF on `paper`, hand
    /// it to `printer`'s dialog, and say what became of it. Blocking, for as long as the fetches
    /// and the dialog take.
    pub(in crate::ui) fn print_through<Tz>(
        store: &SqliteStore,
        job: Job,
        paper: &Paper,
        printer: &Printer,
        sources: &Sources,
        zone: &Tz,
        now: DateTime<Utc>,
    ) -> String
    where
        Tz: TimeZone,
        Tz::Offset: std::fmt::Display,
    {
        let made = made(store, job, paper, sources, zone, now).and_then(|printed| {
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

    /// What Save for printing writes: the PDF Print would make, on the locale's paper.
    pub(super) fn saved_bytes<Tz>(
        store: &SqliteStore,
        job: Job,
        sources: &Sources,
        zone: &Tz,
        now: DateTime<Utc>,
    ) -> Result<(String, Vec<u8>), String>
    where
        Tz: TimeZone,
        Tz::Offset: std::fmt::Display,
    {
        let paper = Paper::from_env();
        let printed = made(store, job, &paper, sources, zone, now)?;
        let pdf = paper::pdf(&printed, &paper)?;
        Ok((printed.subject, pdf))
    }
}

#[cfg(test)]
#[path = "pdf_tests.rs"]
mod pdf_tests;
#[cfg(test)]
mod tests;
