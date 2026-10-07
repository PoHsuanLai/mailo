//! The window every lane drives: the real `mail_app::ui::native::root` over a store seeded in a
//! `TempDir`, on the harness's virtual clock, with every seam that would reach the desktop
//! replaced by a recorder.
//!
//! - the print dialog records each PDF it is handed and answers Cancelled;
//! - the file dialog answers what the lane put in [`Window::answer`] and records each ask;
//! - files the window saves land in a scratch directory ([`Window::saves`]);
//! - a link clicked in a message is recorded rather than opened, and the frames fetch nothing;
//! - desktop notifications are recorded;
//! - the window's wall clock follows the harness's, so a reminder comes due in one `advance`.
//!
//! Nothing here reads or touches the real store, config, keyring, downloads or network.

use ds::prelude::*;
use ds_blitz::{FocusFallback, PrintOutcome, RootContexts};
use ds_harness::{Clock, Driver, Harness, HarnessConfig, Query, Viewport};
use mail_app::ui::appearance::WindowDirs;
use mail_app::ui::native::{Browse, DialogAsk, Dialogs, Fetch, Original, SaveDir, WallClock};
use mail_app::ui::native::{Configured, Revisions};
use mail_core::notify::{Notification, Notifier};
use mail_store::SqliteStore;
use std::path::PathBuf;

pub use super::look::{panel_row, panel_settled, parsed, queued, row_of};
pub use super::seed::{INBOX, account, deliver, hours_ago};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::Instant;

use super::drive::{Drive, Key};
use super::hands;
pub use super::hands::ms;
use super::look::row;
use super::seed::seeded;

const VIEW: Viewport = Viewport {
    width: 1280,
    height: 860,
    scale_percent: 100,
};

/// A lock that a panicking lane cannot leave unusable for the asserts after it.
fn held<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Every PDF the print dialog was handed: its bytes and its title.
pub type Printed = Arc<Mutex<Vec<(Vec<u8>, String)>>>;

/// The file dialog: what each ask is answered with, and every ask it was handed.
#[derive(Clone, Default)]
pub struct Picker {
    answer: Arc<Mutex<Vec<PathBuf>>>,
    asked: Arc<Mutex<Vec<DialogAsk>>>,
}

impl Picker {
    fn dialogs(&self) -> Dialogs {
        let answer = Arc::clone(&self.answer);
        let asked = Arc::clone(&self.asked);
        Dialogs::with_answer(move |ask, _start| {
            held(&asked).push(ask);
            held(&answer).clone()
        })
    }
}

/// Every link the window opened in the browser.
#[derive(Default)]
pub struct Opened(Mutex<Vec<String>>);

impl Browse for Opened {
    fn open(&self, url: &str) {
        held(&self.0).push(url.to_owned());
    }
}

/// Every saved file the window opened, or showed in its folder: `"open"` or `"reveal"`, and
/// the file.
pub type Launched = Arc<Mutex<Vec<(&'static str, PathBuf)>>>;

fn file_opener(launched: &Launched) -> mail_app::ui::native::FileOpener {
    let opened = Arc::clone(launched);
    let revealed = Arc::clone(launched);
    mail_app::ui::native::FileOpener::new(
        move |path| {
            held(&opened).push(("open", path.to_owned()));
            Ok(())
        },
        move |path| {
            held(&revealed).push(("reveal", path.to_owned()));
            Ok(())
        },
    )
}

/// The frames' network: fetches nothing, as a person who never consented to remote images.
struct Offline;

impl Fetch for Offline {
    fn get(&self, _: String, _: Box<dyn FnOnce(Vec<u8>) + Send>) {}
}

/// What the window asked the desktop to say.
#[derive(Default)]
pub struct Notices(Mutex<Vec<Notification>>);

impl Notifier for Notices {
    fn show(&self, notification: &Notification) {
        held(&self.0).push(notification.clone());
    }
}

/// What every window of the lane's app is handed alike: the store, its directories, the
/// revision they share, the file dialog and where files are saved.
#[derive(Clone)]
struct Shared {
    store: Arc<SqliteStore>,
    dirs: WindowDirs,
    revisions: (Revisions, Configured),
    dialogs: Dialogs,
    saves: SaveDir,
}

impl Shared {
    fn contexts(&self) -> RootContexts {
        mail_app::ui::native::contexts(
            Arc::clone(&self.store),
            mail_app::ui::view::Appearance::default(),
            mail_app::ui::space::Spaces::default(),
            Some(self.dirs.clone()),
            mail_app::ui::Start::Inbox,
        )
        .with(self.revisions.0.clone())
        .with(self.revisions.1.clone())
        .with(self.dialogs.clone())
        .with(self.saves.clone())
    }
}

/// The window and everything a lane reads back.
pub struct Window {
    pub harness: Harness,
    pub store: Arc<SqliteStore>,
    shared: Shared,
    picker: Picker,
    opened: Arc<Opened>,
    launched: Launched,
    printed: Printed,
    notices: Arc<Notices>,
    clock: WallClock,
    dir: tempfile::TempDir,
}

impl Window {
    /// The window over the seeded store, which `seed` adds to first, first rows drawn.
    pub fn open(seed: impl FnOnce(&SqliteStore)) -> Window {
        let dir = tempfile::tempdir().expect("a temp dir");
        let store = seeded(dir.path());
        seed(&store);
        std::fs::create_dir_all(dir.path().join("saves")).expect("a saves directory");
        std::fs::create_dir_all(dir.path().join("files")).expect("a files directory");
        let picker = Picker::default();
        let opened = Arc::new(Opened::default());
        let printed = Printed::default();
        let launched = Launched::default();
        let notices = Arc::new(Notices::default());
        let base = chrono::Utc::now();
        // Read first on the harness's thread, where quire's clock is the virtual one.
        let origin: Arc<OnceLock<Instant>> = Arc::new(OnceLock::new());
        let clock = WallClock::new(move || {
            let since =
                ds::base::time::clock::since(*origin.get_or_init(ds::base::time::clock::now));
            base + chrono::TimeDelta::from_std(since).unwrap_or_default()
        });
        let printer = mail_app::ui::native::Printer::with_dialog({
            let printed = Arc::clone(&printed);
            move |pdf, title| {
                held(&printed).push((pdf.to_vec(), title.to_owned()));
                Ok(PrintOutcome::Cancelled)
            }
        });
        let original = Original::new(Arc::new(Offline), opened.clone());
        let dirs = WindowDirs {
            config: dir.path().join("config"),
            state: dir.path().join("state"),
        };
        let shared = Shared {
            store: Arc::clone(&store),
            dirs,
            revisions: (Revisions::new(), Configured::default()),
            dialogs: picker.dialogs(),
            saves: SaveDir(dir.path().join("saves")),
        };
        let contexts = shared
            .contexts()
            .with(printer)
            .with(clock.clone())
            .with(file_opener(&launched))
            .with(mail_app::ui::native::Notices(notices.clone()));
        let config = HarnessConfig::new(VIEW)
            .with_focus_fallback(FocusFallback::Ancestor)
            .with_clock(Clock::Virtual)
            .with_contexts(contexts)
            .with_net(original.net())
            .with_frame_links(original.links())
            .with_contexts(original.contexts());
        let mut harness = Harness::new(mail_app::ui::native::root, config);
        harness.advance(ms(300));
        let mut window = Window {
            harness,
            store,
            shared,
            picker,
            opened,
            launched,
            printed,
            notices,
            clock,
            dir,
        };
        window.until("the inbox's rows are drawn", |h| {
            h.count(".list .ds-thread") > 0
        });
        window
    }

    /// Open Settings on `page` beside this window and let `steps` drive it, on a thread of its
    /// own as every window has one (a window's host is per thread). The main window catches up
    /// with what Settings changed once it is advanced again.
    pub fn in_settings(&mut self, page: &str, steps: impl FnOnce(&mut Harness) + Send) {
        let shared = self.shared.clone();
        std::thread::scope(|scope| {
            scope
                .spawn(move || {
                    let mut settings = super::settings::open_at(shared.contexts(), page);
                    steps(&mut settings);
                })
                .join()
                .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
        });
        self.harness.advance(ms(100));
    }

    /// Wait for `done`, as [`hands::until`].
    pub fn until(&mut self, what: &str, done: impl Fn(&Harness) -> bool) {
        hands::until(&mut self.harness, what, done);
    }

    /// The window's own now, which follows the harness's clock.
    pub fn now(&self) -> chrono::DateTime<chrono::Utc> {
        self.clock.now()
    }

    /// Answer the file dialog's next asks with `paths`.
    pub fn answer(&self, paths: Vec<PathBuf>) {
        *held(&self.picker.answer) = paths;
    }

    /// Every ask the file dialog was handed.
    pub fn asked(&self) -> Vec<DialogAsk> {
        held(&self.picker.asked).clone()
    }

    /// Every link opened in the browser.
    pub fn opened(&self) -> Vec<String> {
        held(&self.opened.0).clone()
    }

    /// Every saved file opened, or shown in its folder, from Downloads.
    pub fn launched(&self) -> Vec<(&'static str, PathBuf)> {
        held(&self.launched).clone()
    }

    /// The window's state directory, where the Downloads list is stored.
    pub fn state(&self) -> PathBuf {
        self.shared.dirs.state.clone()
    }

    /// Every PDF handed to the print dialog.
    pub fn printed(&self) -> Vec<(Vec<u8>, String)> {
        held(&self.printed).clone()
    }

    /// Every notification shown.
    pub fn notices(&self) -> Vec<Notification> {
        held(&self.notices.0).clone()
    }

    /// Where the window saves files.
    pub fn saves(&self) -> PathBuf {
        self.dir.path().join("saves")
    }

    /// Deliver `raw` into the Inbox from another connection to the store, as `mailo watch`
    /// does in its own process: the window hears of it by the store's data version.
    pub fn arrives(&self, uidl: &str, raw: &str) {
        let dir = self.dir.path();
        let watch = SqliteStore::open(dir.join("mail.db"), dir.join("blobs"))
            .expect("a second connection to the store");
        deliver(&watch, uidl, raw);
    }

    /// Write `bytes` as `name` in a scratch directory a dialog can answer with.
    pub fn file(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.dir.path().join("files").join(name);
        std::fs::write(&path, bytes).expect("a scratch file");
        path
    }

    // --- Reading what the window shows ------------------------------------------------------

    /// Wait, as [`Window::until`] does, for the store to say `done`.
    pub fn until_stored(&mut self, what: &str, done: impl Fn(&SqliteStore) -> bool) {
        let store = Arc::clone(&self.store);
        hands::until(&mut self.harness, what, move |_| done(&store));
    }

    /// Wait for the print dialog to have been handed `count` printouts.
    pub fn until_printed(&mut self, what: &str, count: usize) {
        let printed = Arc::clone(&self.printed);
        hands::until(&mut self.harness, what, move |_| {
            held(&printed).len() >= count
        });
    }

    /// Wait for Downloads to have opened or shown `count` files in all.
    pub fn until_launched(&mut self, what: &str, count: usize) {
        let launched = Arc::clone(&self.launched);
        hands::until(&mut self.harness, what, move |_| {
            held(&launched).len() >= count
        });
    }

    /// The centre of `selector`, which must be drawn.
    pub fn centre(&self, selector: &str) -> Point {
        hands::centre(&self.harness, selector)
    }

    /// The text of `selector`, or nothing.
    pub fn text(&self, selector: &str) -> String {
        hands::text(&self.harness, selector)
    }

    /// The subjects of the list's rows, top to bottom.
    pub fn subjects(&self) -> Vec<String> {
        (1..=self.harness.count(".list .ds-list-item"))
            .map(|n| self.text(&format!("{} .ds-thread-sub", row(n))))
            .filter(|subject| !subject.is_empty())
            .collect()
    }

    // --- Doing what a person does -----------------------------------------------------------

    /// Click `selector` once it is drawn where it stands.
    pub fn click(&mut self, selector: &str) {
        hands::click(&mut self.harness, selector);
    }

    /// Type `text`, a newline as Enter.
    pub fn type_text(&mut self, text: &str) {
        hands::type_text(&mut self.harness, text);
    }

    /// Press `key` `times` times with `held` down.
    pub fn press(&mut self, held: &[Key], key: Key, times: usize) {
        hands::press(&mut self.harness, held, key, times);
    }

    /// Open the row whose subject is `subject`, clicking the start of its subject line.
    pub fn open_subject(&mut self, subject: &str) {
        let what = format!("a row reads {subject:?}");
        self.until(&what, |h| row_of(h, subject).is_some());
        let n = row_of(&self.harness, subject).unwrap_or_default();
        let line = format!("{} .ds-thread-sub", row(n));
        let rect = self
            .harness
            .rect(&line)
            .unwrap_or_else(|| panic!("{line} is not drawn"));
        self.harness.click(Point {
            x: Px(rect.origin.x.0 + 24.0),
            y: Px(rect.origin.y.0 + rect.size.height.0 / 2.0),
        });
        let what = format!("the reader shows {subject:?}");
        self.until(&what, |h| {
            h.text_of(".reader")
                .is_some_and(|text| text.contains(subject))
        });
    }

    /// Pick `name` in the open menu once it stands where it is drawn.
    pub fn menu_item(&mut self, name: &str) {
        hands::menu_item(&mut self.harness, name);
    }
}
