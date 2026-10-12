//! Where the application meets `mail-core`: the one runtime, and what the process was started
//! with.
//!
//! `mail-core` and `mail-runtime` are libraries. They start no runtime, read no environment
//! variable and keep no process-wide state; their I/O is `async`, and every piece they need
//! (the environment, the clock) is handed to a [`Mail`]. This module is
//! the one place the binary chooses those pieces, once, in `main`, and the one place a
//! synchronous caller (the command line, a window thread) waits on `async` work.
//!
//! - [`block_on`] waits for a future on the application's runtime. It is for threads that are
//!   not async: calling it from a task of that same runtime panics, as `block_on` always does.
//! - [`install_environment`] records what `main` decided; until it does, the environment is
//!   empty, which is what a test wants.
//! - [`install_profile`] records whose mail the process is over; until it does, that is the
//!   default Profile, which is what a test wants.
//! - [`mail`] builds the handle over a store from those.

use mail_core::profile::{Profile, Roots};
use mail_core::{Environment, KeyringSigningStore, Mail, SigningStore};
use mail_runtime::AccountSecrets;
use mail_store::SqliteStore;
use std::future::Future;
use std::sync::{Arc, OnceLock};
use tokio::runtime::Runtime;

static RUNTIME: OnceLock<Runtime> = OnceLock::new();
static ENVIRONMENT: OnceLock<Environment> = OnceLock::new();
static PROFILE: OnceLock<(Profile, Roots)> = OnceLock::new();

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

/// Records whose mail the process is over, and the directories its paths are under. Only the first
/// call counts.
pub fn install_profile(profile: Profile, roots: Roots) {
    let _ = PROFILE.set((profile, roots));
}

/// The Profile `main` recorded: the default one until it does.
pub fn profile() -> Profile {
    PROFILE
        .get()
        .map(|(profile, _)| profile.clone())
        .unwrap_or_default()
}

/// The directories the recorded Profile's paths are under: `None` until `main` records them.
pub fn roots() -> Option<Roots> {
    PROFILE.get().map(|(_, roots)| roots.clone())
}

/// The signing keys of the recorded Profile, in the keyring service its id names.
pub fn signing() -> Arc<dyn SigningStore> {
    Arc::new(KeyringSigningStore::for_service(profile().signing_service()))
}

/// The handle on `mail-core` over `store`, with the process's environment, its Profile's signing
/// keys and the application's runtime.
pub fn mail(store: &Arc<SqliteStore>) -> Mail {
    Mail::new(store.clone(), runtime().handle().clone())
        .with_environment(environment())
        .with_signing(signing())
}

/// The profile named by a leading `--profile <x>` or `--profile=<x>`, and the arguments without
/// it, so everything that reads the command line afterwards reads what it always read.
///
/// The flag is the program's own and goes ahead of the verb (`mailo --profile work open <thread>`,
/// `mailo --profile work mailto:…`, `mailo --profile work`); after the verb the words are the
/// verb's. A flag with no value is an error, and given twice the last one counts.
pub fn take_profile_flag(
    args: impl IntoIterator<Item = String>,
) -> Result<(Vec<String>, Option<String>), String> {
    let mut rest = args.into_iter().peekable();
    let mut profile = None;
    loop {
        let value = match rest.peek().map(String::as_str) {
            Some("--profile") => {
                rest.next();
                rest.next()
            }
            Some(arg) if arg.starts_with("--profile=") => rest
                .next()
                .and_then(|arg| arg.strip_prefix("--profile=").map(str::to_owned)),
            _ => break,
        };
        match value.filter(|value| !value.is_empty()) {
            Some(value) => profile = Some(value),
            None => {
                return Err(
                    "--profile needs a profile id or name: mailo --profile <id-or-name> <command>"
                        .to_owned(),
                );
            }
        }
    }
    Ok((rest.collect(), profile))
}

/// mailo's store of account secrets, for the calls that take only that.
pub fn secrets() -> Arc<dyn AccountSecrets> {
    mail_runtime::platform_secrets(runtime().handle())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn taken(words: &[&str]) -> Result<(Vec<String>, Option<String>), String> {
        take_profile_flag(words.iter().map(|word| (*word).to_owned()))
    }

    #[test]
    fn the_profile_flag_is_taken_ahead_of_the_verb_and_nothing_else_moves() {
        const CASES: &[(&[&str], &[&str], Option<&str>)] = &[
            (&[], &[], None),
            (&["list"], &["list"], None),
            (&["--profile", "work"], &[], Some("work")),
            (&["--profile=work"], &[], Some("work")),
            (
                &["--profile", "work", "list", "inbox"],
                &["list", "inbox"],
                Some("work"),
            ),
            (
                &["--profile=work", "open", "t1"],
                &["open", "t1"],
                Some("work"),
            ),
            (
                &["--profile", "work", "mailto:a@example.test"],
                &["mailto:a@example.test"],
                Some("work"),
            ),
            // Twice: the last one.
            (
                &["--profile", "a", "--profile=b", "sync"],
                &["sync"],
                Some("b"),
            ),
            // After the verb it is the verb's, which says what it makes of it.
            (
                &["list", "--profile", "work"],
                &["list", "--profile", "work"],
                None,
            ),
            (&["search", "--profile=x"], &["search", "--profile=x"], None),
            // A word that merely starts like it is not it.
            (&["--profiles", "x"], &["--profiles", "x"], None),
        ];
        for (words, rest, profile) in CASES {
            let (got_rest, got) = taken(words).unwrap();
            assert_eq!(&got_rest, rest, "{words:?}");
            assert_eq!(got.as_deref(), *profile, "{words:?}");
        }
    }

    #[test]
    fn a_profile_flag_with_no_value_is_refused() {
        for words in [
            &["--profile"][..],
            &["--profile="],
            &["--profile", "", "list"],
        ] {
            let error = taken(words).unwrap_err();
            assert!(error.contains("--profile needs"), "{words:?}: {error}");
        }
    }

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
