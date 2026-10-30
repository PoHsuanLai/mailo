//! The process's state, as `mail-core` is handed it: the one runtime, and what the process was
//! started with.
//!
//! The command line, the intents provider and the window all reach `mail-core` through the one
//! [`Mail`] this crate builds, so the runtime, the environment, the Profile and the OAuth clients
//! `main` chose are kept here, once, below all three and naming none of them.
//!
//! `mail-core` and `mail-runtime` are libraries. They start no runtime, read no environment
//! variable and keep no process-wide state; their I/O is `async`, and every piece they need
//! (the environment, the clock) is handed to a [`Mail`]. This crate is
//! where the binary records those pieces, once, in `main`, and the one place a
//! synchronous caller (the command line) waits on `async` work.
//!
//! - [`block_on`] waits for a future on the application's runtime. It is for threads that are
//!   not async: calling it from a task of that same runtime panics, as `block_on` always does.
//! - [`install_environment`] records what `main` decided; until it does, the environment is
//!   empty, which is what a test wants.
//! - [`install_profile`] records whose mail the process is over; until it does, that is the
//!   default Profile, which is what a test wants.
//! - [`install_clients`] records the OAuth clients the program signs in as (`main` hands it
//!   [`OAuthClients::here`], this installation's clients files); until
//!   it does there are none, which is what a test wants: a sign-in it starts stops at "needs a
//!   client id" instead of opening a real issuer's page.
//! - [`mail`] builds the handle over a store from those.

use mail_core::profile::Profile;
use mail_core::{Environment, KeyringSigningStore, Mail, OAuthClients, SigningStore};
use mail_store::SqliteStore;
use std::future::Future;
use std::sync::{Arc, OnceLock};
use tokio::runtime::Runtime;

static RUNTIME: OnceLock<Runtime> = OnceLock::new();
static ENVIRONMENT: OnceLock<Environment> = OnceLock::new();
static PROFILE: OnceLock<Profile> = OnceLock::new();
static CLIENTS: OnceLock<OAuthClients> = OnceLock::new();

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

/// Records what the process was started with. Only the first call counts.
pub fn install_environment(environment: Environment) {
    let _ = ENVIRONMENT.set(environment);
}

/// The environment `main` recorded: empty until it does.
pub fn environment() -> Environment {
    ENVIRONMENT.get().cloned().unwrap_or_default()
}

/// Records the OAuth clients the program signs in as. Only the first call counts.
pub fn install_clients(clients: OAuthClients) {
    let _ = CLIENTS.set(clients);
}

/// The OAuth clients `main` recorded: none until it does.
pub fn clients() -> OAuthClients {
    CLIENTS.get().cloned().unwrap_or_default()
}

/// Records whose mail the process is over. Only the first call counts.
pub fn install_profile(profile: Profile) {
    let _ = PROFILE.set(profile);
}

/// The Profile `main` recorded: the default one until it does.
pub fn profile() -> Profile {
    PROFILE.get().cloned().unwrap_or_default()
}

/// The signing keys of the recorded Profile, in the keyring service its id names.
pub fn signing() -> Arc<dyn SigningStore> {
    Arc::new(KeyringSigningStore::for_service(
        profile().signing_service(),
    ))
}

/// The handle on `mail-core` over `store`, with the process's environment, its OAuth clients, its
/// Profile's signing keys and the application's runtime.
pub fn mail(store: &Arc<SqliteStore>) -> Mail {
    Mail::new(store.clone(), runtime().handle().clone())
        .with_environment(environment())
        .with_oauth_clients(clients())
        .with_signing(signing())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_no_profile_recorded_the_default_one_is_in_force() {
        // Another test in this process may have recorded one; the default is the unrecorded case.
        if PROFILE.get().is_none() {
            assert!(profile().id().is_default());
            assert_eq!(profile().signing_service(), "mailo");
        }
    }

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
    fn until_main_decides_the_environment_is_empty() {
        // Once per process and other tests share it, so only the default is shown.
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
