//! Where an account's passwords and tokens live: porter's store, never SQLite.
//!
//! The store is `porter_secrets`'. On Linux that is `Oo7Secrets`, the Secret Service through
//! oo7, standalone: nothing of ours has to be running. On macOS and Windows it is
//! `KeyringSecrets`, the login keychain and the Credential Manager, which cuts what the
//! Credential Manager cannot hold into parts itself. Which one is chosen by target here and in
//! `Cargo.toml`, and nowhere else.
//!
//! Every call is porter's async trait, all the way down: no account secret is read or written by
//! a blocking keyring call on any platform. [`AccountSecrets`] is that trait made object safe,
//! for the places that hold the store as `Arc<dyn AccountSecrets>`; it adds nothing to it.
//!
//! The runtime. oo7 reaches the Secret Service through zbus on tokio (that feature is on in this
//! build), and its one connection keeps its tasks on the runtime that was current when it
//! opened. So the store does not pick a runtime: it is given a [`Handle`] to the one the
//! application owns and runs every call there, however (or from whatever executor) the caller
//! awaits it. What once panicked mailo, a zbus blocking call starting a runtime inside a
//! runtime, cannot come back through this path because there is no blocking call in it; the
//! tests below run it inside one.
//!
//! Signing keys are not here: they are mailo's own (`signing_store.rs`).

use crate::RuntimeError;
use crate::error::Failure;
use porter_core::{AccountId, Credential, SecretKey};
use porter_secrets::{Secrets, SecretsError};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::runtime::Handle;

type Answer<'a, T> = Pin<Box<dyn Future<Output = Result<T, RuntimeError>> + Send + 'a>>;

/// A store of account secrets, held as a trait object.
///
/// porter's [`Secrets`] answers with `impl Future`, which cannot be a trait object, so this is the
/// same four calls with boxed futures and mailo's error. Every `Secrets` is one, and the
/// production one is [`platform_secrets`].
pub trait AccountSecrets: Send + Sync {
    /// The credential filed under `key`.
    fn get<'a>(&'a self, key: &'a SecretKey) -> Answer<'a, Credential>;
    /// Files `value` under `key`, replacing what was there.
    fn put<'a>(&'a self, key: &'a SecretKey, value: &'a Credential) -> Answer<'a, ()>;
    /// Forgets `key`; forgetting a missing one succeeds.
    fn forget<'a>(&'a self, key: &'a SecretKey) -> Answer<'a, ()>;
    /// Forgets every secret of `account`, in one step, when the account is removed.
    fn forget_account<'a>(&'a self, account: &'a AccountId) -> Answer<'a, ()>;

    /// The desktop's accountd, when this store sits beside a link to it: where the credentials of
    /// an account that is accountd's come from (`AuthPlan::Granted`), which this store holds none
    /// of. `None` for every plain store, and then such an account cannot be reached.
    fn link(&self) -> Option<Arc<dyn crate::link::Accountd>> {
        None
    }
}

/// Every secret failure is `Secrets`, which reads as `NeedsReauth`: a missing or locked store is
/// answered by asking the person to sign in, not by retrying. That is how it always was.
fn refused(why: SecretsError) -> RuntimeError {
    RuntimeError::Secrets(Failure::caused(why))
}

impl<S: Secrets> AccountSecrets for S {
    fn get<'a>(&'a self, key: &'a SecretKey) -> Answer<'a, Credential> {
        Box::pin(async move { Secrets::get(self, key).await.map_err(refused) })
    }

    fn put<'a>(&'a self, key: &'a SecretKey, value: &'a Credential) -> Answer<'a, ()> {
        Box::pin(async move { Secrets::put(self, key, value).await.map_err(refused) })
    }

    fn forget<'a>(&'a self, key: &'a SecretKey) -> Answer<'a, ()> {
        Box::pin(async move { Secrets::delete(self, key).await.map_err(refused) })
    }

    fn forget_account<'a>(&'a self, account: &'a AccountId) -> Answer<'a, ()> {
        Box::pin(async move {
            Secrets::delete_account(self, account)
                .await
                .map_err(refused)
        })
    }
}

/// Wait for `work` on this thread with nothing but a parked thread, for a caller that has no
/// runtime to hand (a test, a one-off tool). The application's own edges use their runtime.
///
/// It needs no runtime of its own and cannot start one inside another, which is the panic this
/// path once had. The account store does its work on the [`Handle`] it was given and this only
/// waits for the answer, so it is safe on any thread, including one a tokio runtime is driving
/// (which it then blocks, as the keyring calls it replaces did).
pub fn block_on<T>(work: impl Future<Output = T>) -> T {
    use std::task::{Context, Poll, Wake, Waker};
    struct Unpark(std::thread::Thread);
    impl Wake for Unpark {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let mut work = std::pin::pin!(work);
    let waker = Waker::from(Arc::new(Unpark(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    loop {
        match work.as_mut().poll(&mut cx) {
            Poll::Ready(done) => return done,
            Poll::Pending => std::thread::park(),
        }
    }
}

/// This platform's store of account secrets.
#[cfg(target_os = "linux")]
type Native = porter_secrets::Oo7Secrets;
/// This platform's store of account secrets.
#[cfg(any(target_os = "macos", windows))]
type Native = porter_secrets::KeyringSecrets;
#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
compile_error!(
    "mailo keeps account secrets in porter-secrets, which has a store only on Linux, macOS and Windows"
);

/// The platform's store, every call run on the [`Handle`] it was given.
///
/// oo7 opens one Secret Service connection and keeps it for the process (porter-secrets caches
/// it), and under zbus's `tokio` feature that connection's tasks live on the runtime that was
/// current when it opened. A caller that waits on a short-lived runtime would let the connection
/// die with it, so the store is handed the long-lived one, the application's, and callers await
/// the answer from wherever they are.
#[derive(Clone)]
pub struct PlatformSecrets<S = Native> {
    native: Arc<S>,
    runtime: Handle,
}

// By hand: the store has no `Debug` to rely on, and nothing in here is printable.
impl<S> std::fmt::Debug for PlatformSecrets<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PlatformSecrets")
    }
}

impl PlatformSecrets {
    /// This platform's store, run on `runtime`.
    pub fn new(runtime: Handle) -> Self {
        Self::over(Native::default(), runtime)
    }
}

impl<S> PlatformSecrets<S> {
    /// `native` as the store, run on `runtime`.
    pub fn over(native: S, runtime: Handle) -> Self {
        Self {
            native: Arc::new(native),
            runtime,
        }
    }

    /// Run `work` on the store's runtime and await it from here.
    async fn on_runtime<T: Send + 'static>(
        &self,
        work: impl Future<Output = Result<T, SecretsError>> + Send + 'static,
    ) -> Result<T, SecretsError> {
        self.runtime
            .spawn(work)
            .await
            .unwrap_or(Err(SecretsError::Unavailable))
    }
}

impl<S: Secrets + 'static> Secrets for PlatformSecrets<S> {
    async fn put(&self, key: &SecretKey, value: &Credential) -> Result<(), SecretsError> {
        let (native, key, value) = (self.native.clone(), key.clone(), value.clone());
        self.on_runtime(async move { native.put(&key, &value).await })
            .await
    }

    async fn get(&self, key: &SecretKey) -> Result<Credential, SecretsError> {
        let (native, k) = (self.native.clone(), key.clone());
        self.on_runtime(async move { native.get(&k).await }).await
    }

    async fn delete(&self, key: &SecretKey) -> Result<(), SecretsError> {
        let (native, k) = (self.native.clone(), key.clone());
        self.on_runtime(async move { native.delete(&k).await })
            .await
    }

    async fn delete_account(&self, account: &AccountId) -> Result<(), SecretsError> {
        let (native, a) = (self.native.clone(), account.clone());
        self.on_runtime(async move { native.delete_account(&a).await })
            .await
    }
}

/// The store an entry point opens for a process linked as `link`: accountd's, and then Mail
/// keeps no secret of its own and opens no keyring, or none, and mailo's own store, whose calls
/// run on `runtime`.
pub fn platform_secrets(link: &crate::link::Link, runtime: &Handle) -> Arc<dyn AccountSecrets> {
    match link {
        crate::link::Link::Local => own_store(runtime),
        crate::link::Link::Accountd(link) => {
            Arc::new(crate::link::LinkedSecrets::new(link.clone()))
        }
    }
}

/// mailo's own store, whatever the link: where the sign-ins Mail holds itself are kept. Linked to
/// accountd [`platform_secrets`] holds none of them; removing an account Mail signed in itself is
/// what asks for this, to forget its items with it.
pub fn own_secrets(runtime: &Handle) -> Arc<dyn AccountSecrets> {
    own_store(runtime)
}

/// mailo's own store: the platform's, or (in a debug build run by `dev/scenarios`,
/// `MAILO_TEST_SECRETS_DIR`) a directory of files that never reaches the person's keyring.
fn own_store(runtime: &Handle) -> Arc<dyn AccountSecrets> {
    #[cfg(debug_assertions)]
    if let Some(files) = scenario_files() {
        return Arc::new(files);
    }
    Arc::new(PlatformSecrets::new(runtime.clone()))
}

#[cfg(debug_assertions)]
pub(crate) fn scenario_files() -> Option<scenario::Files> {
    crate::signing_store::scenario::dir().map(scenario::Files)
}

/// A directory of files standing in for the store, in debug builds only: see
/// [`crate::signing_store::scenario`], which is why the directory is the same one.
#[cfg(debug_assertions)]
mod scenario {
    use super::*;
    use porter_secrets::attributes;
    use std::path::PathBuf;

    #[derive(Debug)]
    pub(crate) struct Files(pub(super) PathBuf);

    fn stem(account: &AccountId) -> String {
        format!("porter.{account}.")
    }

    fn file(dir: &std::path::Path, key: &SecretKey) -> PathBuf {
        let purpose = attributes(key).purpose;
        let word: String = purpose
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        dir.join(format!("{}{word}", stem(&key.account)))
    }

    impl Secrets for Files {
        async fn put(&self, key: &SecretKey, value: &Credential) -> Result<(), SecretsError> {
            let text = serde_json::to_string(value).map_err(|_| SecretsError::Unreadable)?;
            std::fs::create_dir_all(&self.0).map_err(|_| SecretsError::Unavailable)?;
            std::fs::write(file(&self.0, key), text).map_err(|_| SecretsError::Unavailable)
        }

        async fn get(&self, key: &SecretKey) -> Result<Credential, SecretsError> {
            match std::fs::read_to_string(file(&self.0, key)) {
                Ok(text) => serde_json::from_str(&text).map_err(|_| SecretsError::Unreadable),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(SecretsError::Missing),
                Err(_) => Err(SecretsError::Unavailable),
            }
        }

        async fn delete(&self, key: &SecretKey) -> Result<(), SecretsError> {
            match std::fs::remove_file(file(&self.0, key)) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(_) => Err(SecretsError::Unavailable),
            }
        }

        async fn delete_account(&self, account: &AccountId) -> Result<(), SecretsError> {
            let Ok(entries) = std::fs::read_dir(&self.0) else {
                return Ok(());
            };
            let stem = stem(account);
            for entry in entries.flatten() {
                if entry.file_name().to_string_lossy().starts_with(&stem) {
                    std::fs::remove_file(entry.path()).map_err(|_| SecretsError::Unavailable)?;
                }
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mail_domain::id::account_id_from_uuid;
    use porter_core::{CapabilityKind, SecretPurpose, SecretText};
    use porter_secrets::MemorySecrets;

    fn key(purpose: SecretPurpose) -> SecretKey {
        SecretKey {
            account: account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-00000000c0de")),
            purpose,
        }
    }

    fn password(text: &str) -> Credential {
        Credential::Password(SecretText::new(text))
    }

    #[tokio::test]
    async fn forgetting_an_account_forgets_all_of_it_and_nothing_of_another() {
        let store: Arc<dyn AccountSecrets> = Arc::new(MemorySecrets::default());
        let mine = key(SecretPurpose::IncomingPassword);
        let contacts = key(SecretPurpose::ServicePassword(CapabilityKind::Contacts));
        let other = SecretKey {
            account: account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-00000000beef")),
            purpose: SecretPurpose::IncomingPassword,
        };
        store.put(&mine, &password("a")).await.unwrap();
        store.put(&contacts, &password("b")).await.unwrap();
        store.put(&other, &password("c")).await.unwrap();
        // `forget` takes one key and leaves the account's others.
        store.forget(&contacts).await.unwrap();
        assert!(store.get(&contacts).await.is_err(), "forgotten");
        assert_eq!(store.get(&mine).await.unwrap(), password("a"), "kept");
        store.put(&contacts, &password("b")).await.unwrap();
        store.forget_account(&mine.account).await.unwrap();
        assert!(store.get(&mine).await.is_err());
        assert!(store.get(&contacts).await.is_err());
        assert_eq!(store.get(&other).await.unwrap(), password("c"));
        // Called on reauthentication, where already-gone is success.
        store.forget(&mine).await.unwrap();
    }

    #[tokio::test]
    async fn a_missing_credential_reports_needs_reauth() {
        use mail_domain::{Retry, Retryable};
        let store: Arc<dyn AccountSecrets> = Arc::new(MemorySecrets::default());
        let err = store
            .get(&key(SecretPurpose::IncomingPassword))
            .await
            .unwrap_err();
        // The outbox must prompt rather than retry: no amount of waiting creates a password.
        assert!(matches!(err.retry(), Retry::NeedsReauth), "{err:?}");
    }

    /// The panic this path once had: zbus's blocking API, with its `tokio` feature on (it is, in
    /// this build, through oo7), starts a runtime of its own for each call, and starting one on a
    /// thread already driving a runtime panics. Account secrets are read from inside every
    /// sync's runtime. The path here is porter's async trait through [`PlatformSecrets`], and
    /// nothing in it blocks: it is awaited from a multi-thread runtime's task, from a
    /// current-thread runtime (a front-end's own), and from a
    /// thread with no runtime at all through [`block_on`], over a store that chunks as the
    /// Credential Manager's does.
    #[test]
    fn the_account_path_runs_on_any_executor_without_a_blocking_call() {
        use porter_secrets::StoreSecrets;
        let store = keyring_core::mock::Store::new().unwrap();
        let multi = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        let secrets: Arc<dyn AccountSecrets> = Arc::new(PlatformSecrets::over(
            StoreSecrets::chunked(store, 16),
            multi.handle().clone(),
        ));
        let k = key(SecretPurpose::IncomingPassword);

        multi.block_on(async {
            secrets.put(&k, &password("hunter2")).await.unwrap();
            let (again, k2) = (secrets.clone(), k.clone());
            let got = tokio::spawn(async move { again.get(&k2).await })
                .await
                .unwrap();
            assert_eq!(got.unwrap(), password("hunter2"));
        });

        let current = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        assert_eq!(
            current.block_on(secrets.get(&k)).unwrap(),
            password("hunter2")
        );

        // No runtime on this thread, and none started by it; and from a thread a runtime is
        // driving, which `block_on` blocks as the keyring calls it replaces did.
        assert_eq!(block_on(secrets.get(&k)).unwrap(), password("hunter2"));
        multi.block_on(async {
            assert_eq!(block_on(secrets.get(&k)).unwrap(), password("hunter2"));
        });
        block_on(secrets.forget_account(&k.account)).unwrap();
        assert!(block_on(secrets.get(&k)).is_err());
    }
}
