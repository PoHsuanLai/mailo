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
//! The watch announces what its passes fetch and a window only what its own passes fetch (a Fetch
//! it is asked for), each message on the pass that first stores it, so little is said twice.
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

use mail_cli::exec::AGAIN;
use mail_core::SqliteStore;
use mail_core::ipc::watching::{Refused, Watching};
use mail_core::schedule::End;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

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

/// How long the watch is left down after its first panic, and the most it is left down for as
/// each one after it doubles the wait.
const REVIVE: Duration = Duration::from_secs(5);
const REVIVE_AT_MOST: Duration = Duration::from_secs(300);

/// How long to leave the watch down after a panic that follows one that took `after`.
fn revive_after(after: Duration) -> Duration {
    (after * 2).clamp(REVIVE, REVIVE_AT_MOST)
}

/// How often the accounts are looked at, beside each write to the store.
const ACCOUNTS_EVERY: Duration = Duration::from_secs(30);

/// How soon after one look at the accounts the next may be. A sync writes to the store without
/// stopping, each write saying so, and a look reads every account and its settings.
const ACCOUNTS_GAP: Duration = Duration::from_secs(2);

/// Set once the app's own watch holds the lock: a window of this process asks it before it
/// knocks on the lock's door, so it leaves fetching to the app even where the door could not be
/// opened (`Watching::doorless`).
static WATCHING_HERE: AtomicBool = AtomicBool::new(false);

/// Whether this process is the one watching.
pub fn watching_here() -> bool {
    WATCHING_HERE.load(Ordering::Acquire)
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
    // The app runs for days: what is said, whose mail, in which mailboxes, with which sound, and
    // how the junk filter behaves are each read as they are used, so Settings is followed from
    // the next pass and not from the next start.
    let desktop = mail_os::desktop::connect_in_window();
    let notifier = mail_ui_model::settings::Notices::new(
        Box::new(move |notification, sound| desktop.show_with(notification, sound)),
        mail_ui_model::settings::person_root(),
    );
    let tell = |watched: mail_core::sync::report::Watched| told(&watched);
    let mut down = Duration::ZERO;
    loop {
        let began = std::time::Instant::now();
        // A panic in the watch ends this run of it, not the thread: the lock is the process's
        // and stays held, so the same claim is used again and nothing is claimed twice.
        let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mail = mail_edge::mail(store);
            mail_edge::block_on(mail.sync().watch_until(
                Some(&notifier),
                mail_core::sync::watcher::Live {
                    whom: &|| notifier.whom(),
                    junk: &mail_ui_model::settings::junk_policy,
                },
                &tell,
                Some(held.asked()),
                Some(accounts.clone()),
                End::Never,
            ))
        }));
        match ran {
            Ok(Ok(_)) => std::thread::sleep(AGAIN),
            Ok(Err(why)) => {
                log::warn!(
                    "checking for mail stopped: {}",
                    mail_cli::remedy::error(why)
                );
                std::thread::sleep(AGAIN);
            }
            Err(panicked) => {
                let what = panicked
                    .downcast_ref::<&str>()
                    .map(|text| (*text).to_owned())
                    .or_else(|| panicked.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "the watch panicked".to_owned());
                // A watch that ran a while before it fell starts the waits again from the
                // shortest, so only one that keeps falling at once is left down for long.
                down = if began.elapsed() > REVIVE_AT_MOST {
                    REVIVE
                } else {
                    revive_after(down)
                };
                log::error!("checking for mail panicked, trying again in {down:?}: {what}");
                // While it is down the window schedules for itself, as it does with no watch.
                WATCHING_HERE.store(false, Ordering::Release);
                std::thread::sleep(down);
                WATCHING_HERE.store(true, Ordering::Release);
            }
        }
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

/// Wait until the accounts are worth looking at again: a write heard on `written`, or `every`
/// with none, but never sooner than `gap` after `looked`, when the last look was. Every write
/// heard by then is taken as one, so a burst of them is a single look, and the one that ends it
/// is not lost for having come early. A store that cannot be watched (its sender went with the
/// watch that was never made) is looked at every `every` all the same.
///
/// Not by `data_version`: a commit through the store's own connection, which is where an account
/// edited in this process is written, does not change it.
fn wait_to_look(
    written: &std::sync::mpsc::Receiver<()>,
    looked: Option<Instant>,
    every: Duration,
    gap: Duration,
) {
    if let Err(std::sync::mpsc::RecvTimeoutError::Disconnected) = written.recv_timeout(every) {
        std::thread::sleep(every);
    }
    if let Some(at) = looked {
        std::thread::sleep(gap.saturating_sub(at.elapsed()));
    }
    while written.try_recv().is_ok() {}
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
                mail_client::on_write(file, move || {
                    // Nobody left to hear it once this thread has ended.
                    let _ = tx.send(());
                })
            });
            let mut known = mail_core::sync::due::intervals(&store);
            let mut looked = Some(Instant::now());
            loop {
                wait_to_look(&written, looked, ACCOUNTS_EVERY, ACCOUNTS_GAP);
                looked = Some(Instant::now());
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

#[cfg(test)]
mod tests;
