//! mailo.app on macOS goes on when its last window closes, as Mail does: it keeps fetching, says
//! new mail on the desktop and keeps the Dock's unread count, until ⌘Q. Clicking its Dock icon, or
//! opening it again, opens a window (`handoff::Requests::open_with`).
//!
//! # One scheduler, in the app
//!
//! What fetches with no window open is the scheduler `mailo watch` runs (`SyncOps::watch_until`),
//! on a thread of the app's own, holding the watch's lock for the app's Profile
//! (`ipc::watching`). So the app is, to every window, exactly what a `mailo watch` would be: a
//! window leaves its scheduled fetching to it ([`Schedule::Watch`]) and opens no connection of its
//! own to hear the server push ([`Daemon::Holding`]), as it does beside a watch started by hand.
//! Only the watch announces new mail (a window never does), so nothing is said twice.
//!
//! The lock decides who watches ([`Step`]): taken, the app watches; held by a `mailo watch`
//! already (a terminal's, or the login item's), the app leaves it to that one and asks again now
//! and then, so that the app takes over when that one stops. A window opened meanwhile asks the
//! lock itself each time, and so follows.
//!
//! The Dock's count is the app's alone, kept on a thread of its own whether or not a window is
//! open (`ui::launch::native`): no other process can badge mailo.app's tile, so a `mailo watch`
//! does not try (`session::keep_the_launchers_badge`), and no window keeps one of its own.
//!
//! Only on macOS ([`stays_open`]). On Linux and Windows the app ends with its last window, as it
//! always has, and the session's `mailo watch` (the systemd user unit, the Run key) is what fetches
//! with no window. Only the launched window starts any of this: a test's window (`ui::native::
//! contexts`, the harness) never does.
//!
//! [`Schedule::Watch`]: mail_core::schedule::Schedule::Watch
//! [`Daemon::Holding`]: mail_core::schedule::Daemon::Holding

use ds_blitz::LastWindowClosed;
use mail_core::SqliteStore;
use mail_core::ipc::watching::{Refused, Watching};
use mail_core::notify::{Notification, Notifier};
use mail_core::schedule::End;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// The platform an app runs on, as far as staying open is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    MacOs,
    /// Linux, the BSDs, Windows.
    Other,
}

impl Os {
    /// This build's.
    pub const HERE: Os = if cfg!(target_os = "macos") {
        Os::MacOs
    } else {
        Os::Other
    };
}

/// Whether the app on `os` goes on with no window open.
pub fn stays_open(os: Os) -> bool {
    os == Os::MacOs
}

/// What the app on `os` does when its last window closes: on macOS it stays, for as long as it
/// runs (a linger too long to add to any instant never ends, `ds_blitz::LastWindowClosed`), and
/// ⌘Q or the Dock's Quit ends it; elsewhere it ends.
pub fn last_window(os: Os) -> LastWindowClosed {
    if stays_open(os) {
        LastWindowClosed::StayFor(Duration::MAX)
    } else {
        LastWindowClosed::Exit
    }
}

/// What the app does with what the watch's lock answered.
#[derive(Debug)]
pub enum Step<W> {
    /// The lock is the app's: it watches, from now until it quits.
    Watch(W),
    /// Another process watches, or the lock could not be asked: the app asks again after
    /// [`AGAIN`].
    Wait,
}

/// Choose what to do with `claimed`, the lock's answer.
pub fn step<W>(claimed: Result<W, Refused>) -> Step<W> {
    match claimed {
        Ok(held) => Step::Watch(held),
        Err(Refused::AlreadyWatching) => Step::Wait,
        Err(Refused::Failed(why)) => {
            log::warn!("can't tell whether a mailo watch is running: {why}");
            Step::Wait
        }
    }
}

/// How long the app waits before asking for the lock again, and before starting the watch again
/// after it ended on an error.
pub const AGAIN: Duration = Duration::from_secs(60);

/// How often the accounts are looked at, beside each write to the store.
const ACCOUNTS_EVERY: Duration = Duration::from_secs(30);

/// Set once the app's own watch holds the lock: a window of this process asks it before it
/// knocks on the lock's door, so it leaves fetching to the app even where the door could not be
/// opened (`Watching::doorless`).
static WATCHING_HERE: AtomicBool = AtomicBool::new(false);

/// Whether this process is the one watching.
fn watching_here() -> bool {
    WATCHING_HERE.load(Ordering::Acquire)
}

/// Whether the app's Profile has a watch fetching for it, this process's own or another's: what
/// a window asks before it fetches or opens a connection of its own.
pub fn watched() -> bool {
    watching_here() || mail_core::ipc::watching::running(crate::edge::profile().id())
}

/// Start the app's own watch over `store`, the Profile `profile`'s, on a thread of its own: take
/// the lock when it is free, and watch for as long as the process runs. Once, from the launched
/// window, and only where [`stays_open`].
///
/// The lock is asked here, before the window opens, so that the window's first look at who
/// schedules already finds the app watching.
pub fn start(store: Arc<SqliteStore>, profile: mail_core::profile::ProfileId) {
    let first = claimed(&profile);
    if first.is_some() {
        WATCHING_HERE.store(true, Ordering::Release);
    }
    let started = std::thread::Builder::new()
        .name("mailo-background".to_owned())
        .spawn(move || {
            let mut next = first;
            loop {
                match next.take() {
                    Some(held) => watch(&store, held),
                    None => std::thread::sleep(AGAIN),
                }
                next = claimed(&profile);
            }
        });
    if let Err(why) = started {
        log::warn!("mail is not checked with no window open: {why}");
    }
}

/// The lock for `profile`'s watch, when this process may have it now.
fn claimed(profile: &mail_core::profile::ProfileId) -> Option<Watching> {
    match step(mail_core::ipc::watching::claim(profile)) {
        Step::Watch(held) => Some(held),
        Step::Wait => None,
    }
}

/// Watch `store` while holding the lock, for as long as the process runs. The lock stays with
/// the process once taken, so a watch that ended on an error is started again after [`AGAIN`].
fn watch(store: &Arc<SqliteStore>, held: Watching) -> ! {
    WATCHING_HERE.store(true, Ordering::Release);
    if let Some(why) = held.doorless() {
        log::warn!("Sync in another process will not reach mailo: {why}");
    }
    let accounts = follow_accounts(store);
    let notifier = AsSet {
        desktop: mail_core::notify::desktop::Desktop::connect_in_window(),
        root: crate::settings::person_root(),
    };
    let tell = |watched: mail_core::sync::report::Watched| told(&watched);
    loop {
        let mail = crate::edge::mail(store);
        let ran = crate::edge::block_on(mail.sync().watch_until(
            Some(&notifier),
            &tell,
            Some(held.asked()),
            Some(accounts.clone()),
            End::Never,
        ));
        if let Err(why) = ran {
            log::warn!(
                "checking for mail stopped: {}",
                crate::cli::remedy::error(why)
            );
        }
        std::thread::sleep(AGAIN);
    }
}

/// What the app's watch says of itself: only what went wrong, in the log, since no one reads it
/// as it goes the way a terminal reads `mailo watch`.
fn told(watched: &mail_core::sync::report::Watched) {
    use mail_core::sync::report::Watched;
    match watched {
        Watched::Pass(_) => {}
        Watched::WaitFailed { address, why } => log::info!("{address}: {why}"),
        Watched::RunFailed { why } | Watched::RemindersFailed { why } => log::warn!("{why}"),
        Watched::AnnounceFailed { address, why } => log::warn!("{address}: {why}"),
    }
}

/// A receiver that moves whenever the accounts that have a server, or how often each is
/// fetched, have changed: read again after each write to the store, and every
/// [`ACCOUNTS_EVERY`], by a thread of its own.
fn follow_accounts(store: &Arc<SqliteStore>) -> mail_core::ipc::watching::Asked {
    let (moved, accounts) = tokio::sync::watch::channel(0u64);
    let store = Arc::clone(store);
    let started = std::thread::Builder::new()
        .name("mailo-accounts".to_owned())
        .spawn(move || {
            let (tx, written) = std::sync::mpsc::channel();
            // Gone only with this thread, which is not a write to say anything about.
            let _watching = store.file().and_then(|file| {
                crate::ui::on_write(file, move || {
                    // Nobody left to hear it once this thread has ended.
                    let _ = tx.send(());
                })
            });
            let mut known = mail_core::sync::due::intervals(&store);
            loop {
                // A write and a timeout are both a reason to look. A store that cannot be watched
                // (its sender went with the watch that was never made) is looked at every
                // ACCOUNTS_EVERY all the same.
                if let Err(std::sync::mpsc::RecvTimeoutError::Disconnected) =
                    written.recv_timeout(ACCOUNTS_EVERY)
                {
                    std::thread::sleep(ACCOUNTS_EVERY);
                }
                while written.try_recv().is_ok() {}
                let now = mail_core::sync::due::intervals(&store);
                if now != known {
                    known = now;
                    moved.send_modify(|count| *count = count.wrapping_add(1));
                }
            }
        });
    if let Err(why) = started {
        log::warn!("an account added now is fetched from mailo's next start: {why}");
    }
    accounts
}

/// The desktop's notifications while `notifications.new_mail` is on, read as each one is said:
/// the app runs for days, and the switch in Settings is followed at once, as a window's is.
struct AsSet {
    desktop: mail_core::notify::desktop::Desktop,
    root: Option<ds_settings::ConfigRoot>,
}

impl Notifier for AsSet {
    fn show(&self, notification: &Notification) {
        if says_new_mail(self.root.as_ref()) {
            self.desktop.show(notification);
        }
    }
}

/// Whether the settings under `root` have new mail said; on, as they ship, where there are none.
fn says_new_mail(root: Option<&ds_settings::ConfigRoot>) -> bool {
    root.map_or(crate::settings::NewMail::default(), |root| {
        crate::settings::load(root).notifications.new_mail
    }) == crate::settings::NewMail::On
}

#[cfg(test)]
mod tests;
