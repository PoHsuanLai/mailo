//! Starting the one-shot adoption of the keyring entries earlier builds kept
//! ([`mail_runtime::adopt`]): once per process, off the thread that draws, in the processes that
//! live long enough to be worth it, the window and `mailo watch`.
//!
//! Until it has run, [`mail_runtime::PlatformSecrets`] reads the old entries behind porter's
//! store, so nothing waits on this: a failure is logged and the application starts regardless,
//! and the next process tries again. A second process (the window beside a watch) reads the
//! store's `unadopted_accounts` and finds what the first left, or nothing; two at once are safe
//! (FINDINGS F200).
//!
//! The work runs on a plain thread of its own, driven by [`mail_runtime::block_on`]: no runtime
//! is started and none is blocked, and the blocking keyring calls it makes for the old entries
//! have their own threads ([`mail_runtime::adopt`]).

use crate::cli::Command;
use mail_runtime::adopt::Report;
use mail_store::{SqliteStore, StoreError};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

/// What adoption is, as the start-up path takes it: a seam, so a test hands in a runner that
/// records the call and the application hands in [`platform`].
pub trait Runner: FnOnce(Arc<SqliteStore>) -> Result<Report, StoreError> + Send + 'static {}
impl<F> Runner for F where F: FnOnce(Arc<SqliteStore>) -> Result<Report, StoreError> + Send + 'static
{}

/// Start adoption for the process that will run `command` (`None` is the window), if it is one
/// that lives long: the window and `watch`. Any other command is over in a moment, and finds the
/// old entries through the platform store's fallback while it lasts.
///
/// Returns the thread, which the application never joins (a test does).
pub fn for_command(
    command: Option<&Command>,
    store: &Arc<SqliteStore>,
    run: impl Runner,
) -> Option<JoinHandle<()>> {
    match command {
        None | Some(Command::Watch { .. }) => Some(spawn(store.clone(), run)),
        Some(_) => None,
    }
}

/// Run `run` over `store` on a thread of its own, and log what it came to.
fn spawn(store: Arc<SqliteStore>, run: impl Runner) -> JoinHandle<()> {
    std::thread::spawn(move || match run(store) {
        Ok(report) => {
            if report.moved > 0 || !report.unfinished.is_empty() {
                eprintln!(
                    "account secrets: {} moved, {} account(s) finished, {} left for the next start",
                    report.moved,
                    report.finished.len(),
                    report.unfinished.len()
                );
            }
        }
        Err(why) => {
            eprintln!("could not adopt the account secrets kept by an earlier build: {why}")
        }
    })
}

/// The application's runner: [`mail_runtime::adopt::run`] on the platform's stores, the first
/// time it is asked in this process and never again.
pub fn platform(store: Arc<SqliteStore>) -> Result<Report, StoreError> {
    static STARTED: AtomicBool = AtomicBool::new(false);
    if STARTED.swap(true, Ordering::SeqCst) {
        return Ok(Report::default());
    }
    mail_runtime::block_on(mail_runtime::adopt::run(&store, chrono::Utc::now()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::WatchNotify;
    use std::sync::atomic::AtomicUsize;

    fn store() -> (Arc<SqliteStore>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        (Arc::new(SqliteStore::in_memory(dir.path()).unwrap()), dir)
    }

    fn counting(calls: &Arc<AtomicUsize>) -> impl Runner {
        let calls = calls.clone();
        move |_| {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(Report::default())
        }
    }

    #[test]
    fn the_window_and_watch_start_it_and_a_short_command_does_not() {
        let (store, _dir) = store();
        let calls = Arc::new(AtomicUsize::new(0));
        for command in [
            None,
            Some(Command::Watch {
                notify: WatchNotify::Never,
            }),
        ] {
            let before = calls.load(Ordering::SeqCst);
            for_command(command.as_ref(), &store, counting(&calls))
                .expect("a long-lived process starts it")
                .join()
                .unwrap();
            assert_eq!(calls.load(Ordering::SeqCst), before + 1);
        }
        let before = calls.load(Ordering::SeqCst);
        assert!(for_command(Some(&Command::Intents), &store, counting(&calls)).is_none());
        assert_eq!(calls.load(Ordering::SeqCst), before);
    }

    #[test]
    fn it_runs_off_the_calling_thread_and_a_failure_or_a_panic_is_not_fatal() {
        let (store, _dir) = store();
        let caller = std::thread::current().id();
        let (tx, rx) = std::sync::mpsc::channel();
        for_command(None, &store, move |_| {
            tx.send(std::thread::current().id()).unwrap();
            Err(StoreError::Db("locked".to_owned()))
        })
        .unwrap()
        .join()
        .unwrap();
        assert_ne!(rx.recv().unwrap(), caller);

        // A runner that panics ends its own thread and nothing else.
        let handle = for_command(None, &store, |_| panic!("keyring exploded")).unwrap();
        assert!(handle.join().is_err());
    }

    #[test]
    fn the_platform_runner_runs_once_per_process() {
        // Over an empty store (no account to adopt), so the platform's stores are never asked.
        let (store, _dir) = store();
        let first = platform(store.clone()).unwrap();
        assert!(first.nothing_to_do());
        assert!(platform(store).unwrap().nothing_to_do());
    }
}
