//! Where the application meets `mail-core`: the one runtime, the link to accountd, and what the
//! process was started with.
//!
//! `mail-core` and `mail-runtime` are libraries. They start no runtime, read no environment
//! variable and keep no process-wide state; their I/O is `async`, and every piece they need
//! (the link to accountd, the environment, the clock) is handed to a [`Mail`]. This module is
//! the one place the binary chooses those pieces, once, in `main`, and the one place a
//! synchronous caller (the command line, a window thread) waits on `async` work.
//!
//! - [`block_on`] waits for a future on the application's runtime. It is for threads that are
//!   not async: calling it from a task of that same runtime panics, as `block_on` always does.
//! - [`install_link`] and [`install_environment`] record what `main` decided; until it does, the
//!   link is [`Link::Local`] and the environment is empty, which is what a test wants.
//! - [`mail`] builds the handle over a store from those.

use mail_core::{Environment, Mail};
use mail_runtime::{AccountSecrets, Link};
use mail_store::SqliteStore;
use std::future::Future;
use std::sync::{Arc, OnceLock};
use tokio::runtime::Runtime;

static RUNTIME: OnceLock<Runtime> = OnceLock::new();
static LINK: OnceLock<Link> = OnceLock::new();
static ENVIRONMENT: OnceLock<Environment> = OnceLock::new();

/// The application's one runtime, started the first time anything asks. It lives as long as the
/// process, which is what the secret store's connection to the keyring needs of it.
pub fn runtime() -> &'static Runtime {
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("mailo-runtime")
            .enable_all()
            .build()
            .expect("a tokio runtime for the application")
    })
}

/// Wait for `work` on the application's runtime, from a thread that is not async.
pub fn block_on<T>(work: impl Future<Output = T>) -> T {
    runtime().block_on(work)
}

/// Records the process's link to accountd. Only the first call counts, like `OnceLock::set`.
pub fn install_link(link: Link) {
    let _ = LINK.set(link);
}

/// The process's link: [`Link::Local`] until [`install_link`] says otherwise.
pub fn link() -> Link {
    LINK.get().cloned().unwrap_or(Link::Local)
}

/// Records what the process was started with. Only the first call counts.
pub fn install_environment(environment: Environment) {
    let _ = ENVIRONMENT.set(environment);
}

/// The environment `main` recorded: empty until it does.
pub fn environment() -> Environment {
    ENVIRONMENT.get().cloned().unwrap_or_default()
}

/// The handle on `mail-core` over `store`, signed in through the process's link, with the
/// process's environment and the application's runtime.
pub fn mail(store: &Arc<SqliteStore>) -> Mail {
    Mail::new(store.clone(), link(), runtime().handle().clone()).with_environment(environment())
}

/// The store of account secrets for the process's link, for the calls that take only that.
pub fn secrets() -> Arc<dyn AccountSecrets> {
    mail_runtime::platform_secrets(&link(), runtime().handle())
}

/// mailo's own store of account secrets whatever the link: where the sign-ins Mail holds itself
/// are kept.
pub fn own_secrets() -> Arc<dyn AccountSecrets> {
    mail_runtime::own_secrets(runtime().handle())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn work_is_waited_on_from_a_plain_thread_and_from_many_at_once() {
        assert_eq!(block_on(async { 1 + 1 }), 2);
        let threads: Vec<_> = (0..4)
            .map(|n| std::thread::spawn(move || block_on(async move { n * 2 })))
            .collect();
        let got: Vec<i32> = threads.into_iter().map(|t| t.join().unwrap()).collect();
        assert_eq!(got, [0, 2, 4, 6]);
    }

    #[test]
    fn until_main_decides_the_link_is_in_process_and_the_environment_is_empty() {
        // Both are once per process and other tests share them, so only the defaults are shown.
        assert!(matches!(link(), Link::Local) || link().is_linked());
        let _ = environment();
    }

    #[test]
    fn a_handle_made_from_the_edge_carries_the_edges_pieces() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(SqliteStore::in_memory(dir.path()).unwrap());
        let handle = mail(&store);
        assert!(Arc::ptr_eq(handle.store(), &store));
    }
}
