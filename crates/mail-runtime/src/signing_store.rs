//! mailo's own store for signing keys: an OpenPGP secret key or an S/MIME private key, in the
//! platform keyring, never SQLite.
//!
//! Account secrets (passwords, tokens) are not here: they are porter's, filed through
//! `porter_secrets::Secrets` (`account_secrets.rs`). A signing key is mailo's alone (an account
//! has none of them, and the accounts daemon never holds one), so it keeps the entries it has
//! always had, service `mailo`, named by the key, in the JSON of [`stored`], frozen by
//! `tests/fixtures/keyring/`. What does not fit one entry on the Windows Credential Manager (an
//! armored key routinely exceeds 2560 bytes) is cut into parts by `keyring-chunked`, under the
//! marker [`MARK`].
//!
//! This path is blocking keyring-core calls, run on a thread of its own ([`off_runtime`]). That
//! is allowed here and only here: a signing key is read when a message is signed or opened, from
//! the thread that does it, never from a sync's runtime.

pub(crate) mod stored;

use crate::RuntimeError;
use keyring_chunked::{ChunkedStore, Limit};
use keyring_core::CredentialStore;
use keyring_core::api::CredentialStoreApi;
use mail_domain::signing::{SigningKeyId, SigningKeyRef, SigningSecret};
use mail_net::Failure;
use std::sync::Arc;
#[cfg(any(feature = "keyring", test))]
use std::sync::Mutex;
use stored::Stored;

/// A store of signing keys.
///
/// A trait because the tests must not touch the user's real keyring, and because a headless
/// machine may have no keyring at all.
pub trait SigningStore: Send + Sync {
    fn get(&self, key: &SigningKeyRef) -> Result<SigningSecret, RuntimeError>;
    fn put(&self, key: &SigningKeyRef, value: &SigningSecret) -> Result<(), RuntimeError>;
    fn forget(&self, key: &SigningKeyRef) -> Result<(), RuntimeError>;
}

/// The keyring service mailo's signing keys are filed under when nothing says otherwise: the one
/// every entry of mailo's, signing keys and earlier builds' account secrets alike, always had.
/// A Profile other than the default files its keys under a service of its own
/// ([`KeyringSigningStore::for_service`]).
pub(crate) const SERVICE: &str = "mailo";

/// Named by the key alone: see [`SigningKeyId`] for why the account is not part of it. The
/// prefix cannot collide with an account's entries, which begin with a UUID.
pub(crate) fn signing_entry_name(key: &SigningKeyRef) -> String {
    match key.key {
        SigningKeyId::OpenPgp(fingerprint) => format!("openpgp:{fingerprint}"),
        SigningKeyId::Smime(fingerprint) => format!("smime:{fingerprint}"),
        SigningKeyId::PrivacyRelay => "privacy-relay".to_owned(),
    }
}

pub(crate) fn signing_secret(stored: Stored) -> Result<SigningSecret, RuntimeError> {
    match stored {
        Stored::OpenPgp(armored) => Ok(SigningSecret::OpenPgp(armored)),
        Stored::SmimeKey(pem) => Ok(SigningSecret::SmimeKey(pem)),
        Stored::RelayKey(key) => Ok(SigningSecret::Relay(key)),
        Stored::Password(_) | Stored::OAuth { .. } => Err(RuntimeError::Secrets(Failure::said(
            "the keyring entry holds an account's credential, not a signing key",
        ))),
    }
}

pub(crate) fn stored_signing(value: &SigningSecret) -> Stored {
    match value {
        SigningSecret::OpenPgp(armored) => Stored::OpenPgp(armored.clone()),
        SigningSecret::SmimeKey(pem) => Stored::SmimeKey(pem.clone()),
        SigningSecret::Relay(key) => Stored::RelayKey(key.clone()),
    }
}

/// Signing keys in a keyring: the Secret Service on Linux and the BSDs, the login keychain on
/// macOS, the Credential Manager on Windows (which store is chosen by target in `Cargo.toml` and
/// in [`open`] below, and nowhere else), or one given keyring-core store, which is how the tests
/// use keyring-core's mock with the Credential Manager's size limit on any platform.
///
/// Entries are filed under one keyring service ([`for_service`](Self::for_service); `mailo` unless
/// told otherwise), so two services keep the same key apart.
///
/// Every call runs on a thread of its own, whichever it is over: see [`off_runtime`].
pub struct KeyringSigningStore {
    slots: Box<dyn Slots + Send + Sync>,
}

impl std::fmt::Debug for KeyringSigningStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("KeyringSigningStore")
    }
}

impl Default for KeyringSigningStore {
    /// The platform's keyring, under the service `mailo`.
    fn default() -> Self {
        Self::for_service(SERVICE)
    }
}

impl KeyringSigningStore {
    /// The platform's keyring, with entries filed under `service`: a Profile's own
    /// (`mail_core::profile::Profile::signing_service`).
    pub fn for_service(service: impl Into<String>) -> Self {
        Self::with_slots(Box::new(Keyring {
            service: service.into(),
        }))
    }

    pub(crate) fn with_slots(slots: Box<dyn Slots + Send + Sync>) -> Self {
        Self { slots }
    }

    /// Keeps signing keys in `store`, each in one entry, under the service `mailo`.
    pub fn over(store: Arc<CredentialStore>) -> Self {
        Self::over_service(store, SERVICE)
    }

    /// Keeps signing keys in `store`, each in one entry, under `service`.
    pub fn over_service(store: Arc<CredentialStore>, service: impl Into<String>) -> Self {
        Self::with_slots(Box::new(StoreSlots::new(store, service, Limit::None)))
    }

    /// Keeps signing keys in `store`, cut into parts of at most `units` UTF-16 code units.
    pub fn chunked(store: Arc<CredentialStore>, units: usize) -> Self {
        Self::with_slots(Box::new(StoreSlots::new(
            store,
            SERVICE,
            Limit::Utf16Units(units),
        )))
    }
}

impl SigningStore for KeyringSigningStore {
    fn get(&self, key: &SigningKeyRef) -> Result<SigningSecret, RuntimeError> {
        let name = signing_entry_name(key);
        off_runtime(|| read_with(self.slots.as_ref(), &name))
    }

    fn put(&self, key: &SigningKeyRef, value: &SigningSecret) -> Result<(), RuntimeError> {
        let (name, stored) = (signing_entry_name(key), stored_signing(value));
        off_runtime(|| write_with(self.slots.as_ref(), &name, &stored))
    }

    fn forget(&self, key: &SigningKeyRef) -> Result<(), RuntimeError> {
        let name = signing_entry_name(key);
        off_runtime(|| self.slots.delete(&name))
    }
}

fn read_with(slots: &dyn Slots, name: &str) -> Result<SigningSecret, RuntimeError> {
    let stored = slots
        .read(name)?
        .ok_or_else(|| RuntimeError::Secrets(Failure::said("no such signing key")))?;
    signing_secret(decode(&stored)?)
}

fn write_with(slots: &dyn Slots, name: &str, value: &Stored) -> Result<(), RuntimeError> {
    let encoded = serde_json::to_string(value)
        .map_err(|e| RuntimeError::Secrets(Failure::new("cannot encode signing key", e)))?;
    slots.write(name, &encoded)
}

/// An entry's JSON as [`Stored`], the shape every earlier build wrote.
pub(crate) fn decode(text: &str) -> Result<Stored, RuntimeError> {
    serde_json::from_str(text)
        .map_err(|e| RuntimeError::Secrets(Failure::new("stored credential is unreadable", e)))
}

/// The platform's entries, by name: a value read, written or removed whole, whatever the
/// platform holds in one entry. The keyring in the program, a map or a directory in the tests.
pub(crate) trait Slots {
    /// The value in `name`, or `None` when there is no such entry.
    fn read(&self, name: &str) -> Result<Option<String>, RuntimeError>;
    fn write(&self, name: &str, value: &str) -> Result<(), RuntimeError>;
    /// Remove `name`. An entry that is already gone is not an error.
    fn delete(&self, name: &str) -> Result<(), RuntimeError>;
}

/// What the head entry of a value in parts holds, before the count. Frozen: a Windows keyring
/// written by an earlier build is read by it.
const MARK: &str = "mailo-parts:";

/// One keyring-core store's entries under one service, as [`Slots`]. A value over the `limit` is
/// kept in several entries by `keyring-chunked`; with no limit, every value is one entry.
#[derive(Clone)]
pub(crate) struct StoreSlots {
    store: ChunkedStore,
    service: String,
}

impl StoreSlots {
    pub(crate) fn new(
        store: Arc<CredentialStore>,
        service: impl Into<String>,
        limit: Limit,
    ) -> Self {
        let store = ChunkedStore::new(store, MARK, limit)
            .unwrap_or_else(|_| unreachable!("the marker is not empty"));
        Self {
            store,
            service: service.into(),
        }
    }
}

impl std::fmt::Debug for StoreSlots {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StoreSlots")
    }
}

impl Slots for StoreSlots {
    fn read(&self, name: &str) -> Result<Option<String>, RuntimeError> {
        match self
            .store
            .build(&self.service, name, None)
            .map_err(refused)?
            .get_password()
        {
            Ok(value) => Ok(Some(value)),
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(e) => Err(refused(e)),
        }
    }

    fn write(&self, name: &str, value: &str) -> Result<(), RuntimeError> {
        self.store
            .build(&self.service, name, None)
            .map_err(refused)?
            .set_password(value)
            .map_err(refused)
    }

    fn delete(&self, name: &str) -> Result<(), RuntimeError> {
        match self
            .store
            .build(&self.service, name, None)
            .map_err(refused)?
            .delete_credential()
        {
            // Already gone is the state we wanted.
            Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
            Err(e) => Err(refused(e)),
        }
    }
}

/// How much one entry holds: 2560 bytes of UTF-16 in the Credential Manager. The Secret Service
/// and the Keychain have no limit a credential meets, so there every value is one entry, as it
/// always was.
#[cfg(windows)]
pub(crate) const LIMIT: Limit = Limit::WINDOWS;
#[cfg(not(windows))]
pub(crate) const LIMIT: Limit = Limit::None;

/// This platform's credential store, as [`Slots`].
#[derive(Debug)]
pub(crate) struct Keyring {
    service: String,
}

impl Keyring {
    /// The entry's name in the scenario directory, which has no services: the default's is the
    /// name alone, as it always was, and another service's is told apart by a prefix.
    #[cfg(debug_assertions)]
    fn scenario_name(&self, name: &str) -> String {
        if self.service == SERVICE {
            name.to_owned()
        } else {
            format!("{}/{name}", self.service)
        }
    }
}

impl Slots for Keyring {
    fn read(&self, name: &str) -> Result<Option<String>, RuntimeError> {
        #[cfg(debug_assertions)]
        if let Some(dir) = scenario::dir() {
            return scenario::read(&dir, &self.scenario_name(name));
        }
        on_store(|store| StoreSlots::new(store, self.service.clone(), LIMIT).read(name))
    }

    fn write(&self, name: &str, value: &str) -> Result<(), RuntimeError> {
        #[cfg(debug_assertions)]
        if let Some(dir) = scenario::dir() {
            return scenario::write(&dir, &self.scenario_name(name), value);
        }
        on_store(|store| StoreSlots::new(store, self.service.clone(), LIMIT).write(name, value))
    }

    fn delete(&self, name: &str) -> Result<(), RuntimeError> {
        #[cfg(debug_assertions)]
        if let Some(dir) = scenario::dir() {
            return scenario::delete(&dir, &self.scenario_name(name));
        }
        on_store(|store| StoreSlots::new(store, self.service.clone(), LIMIT).delete(name))
    }
}

/// A directory of plain files standing in for the keyring, in debug builds only.
///
/// `dev/scenarios` runs the real `mailo watch` on a private bus with no Secret Service on it, and
/// must never reach the person's keyring; `MAILO_TEST_SECRETS_DIR` names a scratch directory and
/// each credential is a file in it. Compiled out of a release build, like sill's `--fake-ddc`: a
/// binary a person installs has no way to keep a password in a file.
#[cfg(debug_assertions)]
pub(crate) mod scenario {
    use crate::RuntimeError;
    use std::path::{Path, PathBuf};

    /// `MAILO_TEST_SECRETS_DIR` when it is set. Otherwise, under a test harness, a directory of
    /// this process's own: a test never reaches the person's keyring, whether or not it thought
    /// to ask for a store of its own. `MAILO_ALLOW_REAL_KEYRING` lets the live tests
    /// (`scripts/live-tests.sh`), which are run deliberately, reach it.
    pub(crate) fn dir() -> Option<PathBuf> {
        if let Some(dir) = std::env::var_os("MAILO_TEST_SECRETS_DIR") {
            return Some(PathBuf::from(dir));
        }
        if std::env::var_os("MAILO_ALLOW_REAL_KEYRING").is_some() || !under_test() {
            return None;
        }
        static SCRATCH: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
        let dir = SCRATCH.get_or_init(|| {
            std::env::temp_dir().join(format!("mailo-test-secrets-{}", std::process::id()))
        });
        std::fs::create_dir_all(dir).ok()?;
        Some(dir.clone())
    }

    /// This crate's own unit tests, or an executable in a cargo `deps` directory: where cargo
    /// puts every test and bench binary, and never the `mailo` that is run or installed.
    fn under_test() -> bool {
        cfg!(test)
            || std::env::current_exe().ok().is_some_and(|exe| {
                exe.parent()
                    .and_then(Path::file_name)
                    .is_some_and(|dir| dir == "deps")
            })
    }

    fn file(dir: &Path, name: &str) -> PathBuf {
        dir.join(name.replace([':', '#', '/'], "_"))
    }

    fn failed(e: std::io::Error) -> RuntimeError {
        RuntimeError::Secrets(mail_net::Failure::new("the scenario secrets directory", e))
    }

    pub(crate) fn read(dir: &Path, name: &str) -> Result<Option<String>, RuntimeError> {
        match std::fs::read_to_string(file(dir, name)) {
            Ok(value) => Ok(Some(value)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(failed(e)),
        }
    }

    pub(crate) fn write(dir: &Path, name: &str, value: &str) -> Result<(), RuntimeError> {
        std::fs::create_dir_all(dir).map_err(failed)?;
        std::fs::write(file(dir, name), value).map_err(failed)
    }

    pub(crate) fn delete(dir: &Path, name: &str) -> Result<(), RuntimeError> {
        match std::fs::remove_file(file(dir, name)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(failed(e)),
        }
    }
}

fn refused(e: keyring_core::Error) -> RuntimeError {
    RuntimeError::Secrets(Failure::caused(e))
}

/// `call` on the platform's store, opened on first use and kept.
///
/// A store that could not be opened is tried again on the next use, not remembered as missing:
/// a Secret Service that was not up when the first sync ran (a session whose keyring daemon
/// starts late) is up for the second. A kept store that stops answering is opened again, as
/// [`Kept`] says.
#[cfg(feature = "keyring")]
fn on_store<T>(
    call: impl Fn(Arc<CredentialStore>) -> Result<T, RuntimeError>,
) -> Result<T, RuntimeError> {
    static OPENED: Kept<CredentialStore> = Kept::new();
    OPENED.run(
        || {
            open().map_err(|e| {
                RuntimeError::Secrets(Failure::new("no keyring to keep signing keys in", e))
            })
        },
        call,
    )
}

/// A store opened on first use and kept until it stops answering.
///
/// The Secret Service store holds a connection, and a session, with the daemon that was running
/// when it opened. When that daemon goes (the desktop session ends, and the next one starts a
/// daemon of its own), every call on the kept store fails, and a process that outlives the
/// session, `mailo watch`, would never reach the new daemon. So a call that finds the store
/// unreachable forgets it, and a call that used a kept store is tried once more on a fresh one.
#[cfg(any(feature = "keyring", test))]
struct Kept<S: ?Sized> {
    slot: Mutex<Option<Arc<S>>>,
}

#[cfg(any(feature = "keyring", test))]
impl<S: ?Sized> Kept<S> {
    const fn new() -> Self {
        Self {
            slot: Mutex::new(None),
        }
    }

    fn slot(&self) -> std::sync::MutexGuard<'_, Option<Arc<S>>> {
        self.slot
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn open(
        &self,
        open: &impl Fn() -> Result<Arc<S>, RuntimeError>,
    ) -> Result<Arc<S>, RuntimeError> {
        let store = open()?;
        *self.slot() = Some(store.clone());
        Ok(store)
    }

    /// Forgets `store`, unless another call has already put a newer one in its place.
    fn forget(&self, store: &Arc<S>) {
        let mut slot = self.slot();
        if slot.as_ref().is_some_and(|held| Arc::ptr_eq(held, store)) {
            *slot = None;
        }
    }

    fn run<T>(
        &self,
        open: impl Fn() -> Result<Arc<S>, RuntimeError>,
        call: impl Fn(Arc<S>) -> Result<T, RuntimeError>,
    ) -> Result<T, RuntimeError> {
        let held = self.slot().clone();
        let kept = held.is_some();
        let store = match held {
            Some(store) => store,
            None => self.open(&open)?,
        };
        let answer = call(store.clone());
        if !answer.as_ref().is_err_and(unreachable) {
            return answer;
        }
        self.forget(&store);
        if !kept {
            return answer;
        }
        let fresh = self.open(&open)?;
        let answer = call(fresh.clone());
        if answer.as_ref().is_err_and(unreachable) {
            self.forget(&fresh);
        }
        answer
    }
}

/// Whether `error` is the store not answering, rather than an answer about an entry.
#[cfg(any(feature = "keyring", test))]
fn unreachable(error: &RuntimeError) -> bool {
    let RuntimeError::Secrets(failure) = error else {
        return false;
    };
    matches!(
        std::error::Error::source(failure).and_then(|source| source.downcast_ref()),
        Some(keyring_core::Error::PlatformFailure(_) | keyring_core::Error::NoStorageAccess(_))
    )
}

/// Linux and the BSDs: the Secret Service, which GNOME Keyring, KWallet and KeePassXC provide.
#[cfg(all(
    feature = "keyring",
    unix,
    not(any(target_os = "macos", target_os = "ios", target_os = "android"))
))]
fn open() -> keyring_core::Result<Arc<CredentialStore>> {
    Ok(zbus_secret_service_keyring_store::Store::new()?)
}

/// macOS: the user's login keychain.
#[cfg(all(feature = "keyring", target_os = "macos"))]
fn open() -> keyring_core::Result<Arc<CredentialStore>> {
    Ok(apple_native_keyring_store::keychain::Store::new()?)
}

/// Windows: the Credential Manager, as generic credentials named `<entry>.mailo`.
#[cfg(all(feature = "keyring", windows))]
fn open() -> keyring_core::Result<Arc<CredentialStore>> {
    Ok(windows_native_keyring_store::Store::new()?)
}

#[cfg(all(
    feature = "keyring",
    not(any(
        windows,
        target_os = "macos",
        all(
            unix,
            not(any(target_os = "macos", target_os = "ios", target_os = "android"))
        )
    ))
))]
compile_error!("mailo keeps signing keys in the platform keyring, and knows none for this target");

/// A build without the `keyring` feature has no platform keyring to open: a signing key is kept
/// in whatever [`SigningStore`] the caller hands in, or not at all.
#[cfg(not(feature = "keyring"))]
fn on_store<T>(
    _: impl Fn(Arc<CredentialStore>) -> Result<T, RuntimeError>,
) -> Result<T, RuntimeError> {
    Err(RuntimeError::Secrets(Failure::said(
        "this build has no platform keyring to keep signing keys in",
    )))
}

/// Run `work` on a thread of its own and wait for it.
///
/// On Linux the keyring reaches the Secret Service through zbus's blocking API, and a build that
/// turns on zbus's `tokio` feature anywhere (quire's settings crate does) makes that API start a
/// Tokio runtime of its own for each call. Starting one on a thread already driving a runtime
/// panics, and the keyring is read from inside every sync's runtime. A plain thread drives none,
/// whichever executor zbus was built for. The Keychain and the Credential Manager are plain
/// blocking calls, which a thread of their own keeps off the runtime's workers as well.
pub fn off_runtime<T: Send>(work: impl FnOnce() -> T + Send) -> T {
    std::thread::scope(|scope| match scope.spawn(work).join() {
        Ok(done) => done,
        Err(panic) => std::panic::resume_unwind(panic),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mail_domain::id::account_id_from_uuid;
    use mail_domain::signing::SigningKeyId;

    /// keyring-core's `error` as a store call hands it on.
    fn said(error: keyring_core::Error) -> RuntimeError {
        refused(error)
    }

    fn gone() -> RuntimeError {
        said(keyring_core::Error::PlatformFailure(Box::new(
            std::io::Error::other("the daemon left the bus"),
        )))
    }

    #[test]
    fn a_kept_store_that_stops_answering_is_opened_again_within_the_same_call() {
        use std::sync::atomic::{AtomicU32, Ordering};
        let kept: Kept<u32> = Kept::new();
        let opens = AtomicU32::new(0);
        let open = || Ok(Arc::new(opens.fetch_add(1, Ordering::SeqCst) + 1));
        // The daemon restarts after the first store is opened: only stores from 2 on answer.
        let read = |store: Arc<u32>| match *store >= 2 {
            true => Ok(*store),
            false => Err(gone()),
        };
        let first = kept.run(open, |store| Ok(*store));
        assert_eq!(first.ok(), Some(1));
        assert_eq!(
            kept.run(open, read).ok(),
            Some(2),
            "the call reached the new daemon"
        );
        assert_eq!(
            kept.run(open, read).ok(),
            Some(2),
            "and the new store is kept"
        );
        assert_eq!(opens.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_store_that_answers_about_an_entry_is_kept() {
        use std::sync::atomic::{AtomicU32, Ordering};
        let kept: Kept<()> = Kept::new();
        let opens = AtomicU32::new(0);
        let open = || {
            opens.fetch_add(1, Ordering::SeqCst);
            Ok(Arc::new(()))
        };
        let missing = kept.run(open, |_| Err::<(), _>(said(keyring_core::Error::NoEntry)));
        assert!(missing.is_err());
        let refused = kept.run(open, |_| {
            Err::<(), _>(RuntimeError::Secrets(Failure::said("no such signing key")))
        });
        assert!(refused.is_err());
        assert_eq!(opens.load(Ordering::SeqCst), 1, "opened once, never again");
        let down = kept.run(open, |_| Err::<(), _>(gone()));
        assert!(down.is_err());
        assert_eq!(
            opens.load(Ordering::SeqCst),
            2,
            "an unreachable store is opened again"
        );
    }

    const OPENPGP: &str = include_str!("../tests/fixtures/keyring/openpgp.json");
    const SMIME: &str = include_str!("../tests/fixtures/keyring/smime.json");

    fn pgp_ref() -> SigningKeyRef {
        SigningKeyRef {
            account: account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-00000000c0de")),
            key: SigningKeyId::OpenPgp(mail_domain::Fingerprint::V4([7; 20])),
        }
    }

    /// What zbus's blocking API does under its `tokio` feature, done from inside a runtime as a
    /// sync does: without `off_runtime` this panics "Cannot start a runtime from within a runtime".
    #[test]
    fn work_that_starts_a_runtime_can_be_run_from_inside_one() {
        let outer = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let answer = outer.block_on(async {
            off_runtime(|| {
                tokio::runtime::Runtime::new()
                    .unwrap()
                    .block_on(async { 42 })
            })
        });
        assert_eq!(answer, 42);
    }

    /// Entries that start a runtime of their own, as the Secret Service store does under zbus's
    /// `tokio` feature, which this build has on (oo7 turns it on for the account path).
    struct StartsRuntime(Mem);

    type Mem = std::sync::Mutex<std::collections::BTreeMap<String, String>>;

    impl Slots for StartsRuntime {
        fn read(&self, name: &str) -> Result<Option<String>, RuntimeError> {
            tokio::runtime::Runtime::new().unwrap().block_on(async {});
            Ok(self.0.lock().unwrap().get(name).cloned())
        }
        fn write(&self, name: &str, value: &str) -> Result<(), RuntimeError> {
            tokio::runtime::Runtime::new().unwrap().block_on(async {});
            self.0
                .lock()
                .unwrap()
                .insert(name.to_owned(), value.to_owned());
            Ok(())
        }
        fn delete(&self, name: &str) -> Result<(), RuntimeError> {
            tokio::runtime::Runtime::new().unwrap().block_on(async {});
            self.0.lock().unwrap().remove(name);
            Ok(())
        }
    }

    /// The feature-unification regression, on the signing path: a keyring whose every call
    /// starts a runtime, used from inside a runtime (a window's thread, a sync's task).
    #[test]
    fn the_signing_path_runs_inside_a_runtime_whose_keyring_starts_one() {
        let store = KeyringSigningStore::with_slots(Box::new(StartsRuntime(Mem::default())));
        let outer = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .build()
            .unwrap();
        outer.block_on(async {
            let secret = SigningSecret::OpenPgp("-----BEGIN PGP PRIVATE KEY BLOCK-----".into());
            store.put(&pgp_ref(), &secret).unwrap();
            assert_eq!(store.get(&pgp_ref()).unwrap(), secret);
            store.forget(&pgp_ref()).unwrap();
            assert!(store.get(&pgp_ref()).is_err());
        });
    }

    /// The frozen fixtures are what the signing store reads and writes, byte for byte.
    #[test]
    fn the_frozen_signing_fixtures_read_and_write_back_through_the_store() {
        let mock = keyring_core::mock::Store::new().unwrap();
        let store = KeyringSigningStore::over(mock.clone());
        for (text, make) in [
            (
                OPENPGP,
                SigningKeyId::OpenPgp(mail_domain::Fingerprint::V4([1; 20])),
            ),
            (
                SMIME,
                SigningKeyId::Smime(mail_domain::smime::CertFingerprint([2; 32])),
            ),
        ] {
            let key = SigningKeyRef {
                account: pgp_ref().account,
                key: make,
            };
            let entries: Vec<Stored> = serde_json::from_str(text).unwrap();
            // Written the way every earlier build wrote it: the entry is the one JSON value.
            StoreSlots::new(mock.clone(), SERVICE, Limit::None)
                .write(
                    &signing_entry_name(&key),
                    &serde_json::to_string(&entries[0]).unwrap(),
                )
                .unwrap();
            let secret = store.get(&key).unwrap();
            store.put(&key, &secret).unwrap();
            let kept = StoreSlots::new(mock.clone(), SERVICE, Limit::None)
                .read(&signing_entry_name(&key))
                .unwrap()
                .unwrap();
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&kept).unwrap(),
                serde_json::to_value(&entries[0]).unwrap()
            );
        }
    }

    /// An armored OpenPGP key or an S/MIME PKCS#8 PEM is longer than the Credential Manager's
    /// 2560 bytes. On the limited mock keyring (which refuses an entry over the limit, as that
    /// does), it is kept in parts, read back whole, and forgotten without leaving a part.
    #[test]
    fn a_key_over_the_credential_managers_limit_is_kept_in_parts_and_forgotten_whole() {
        let mock = keyring_core::mock::Store::new().unwrap();
        // Room for the head's marker and a part, as the Windows limit is for ours.
        let store = KeyringSigningStore::chunked(mock.clone(), 1280);
        let armored = format!(
            "-----BEGIN PGP PRIVATE KEY BLOCK-----\n{}\n-----END PGP PRIVATE KEY BLOCK-----",
            "x".repeat(6000)
        );
        let pem = format!(
            "-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----",
            "y".repeat(4000)
        );
        let smime = SigningKeyRef {
            account: pgp_ref().account,
            key: SigningKeyId::Smime(mail_domain::smime::CertFingerprint([9; 32])),
        };
        // The entries as the keyring holds them, under the chunking: a head, and the parts.
        let raw = |name: &str| match mock.build(SERVICE, name, None).unwrap().get_password() {
            Ok(value) => Some(value),
            Err(keyring_core::Error::NoEntry) => None,
            Err(other) => panic!("{other}"),
        };
        for (key, secret) in [
            (pgp_ref(), SigningSecret::OpenPgp(armored)),
            (smime, SigningSecret::SmimeKey(pem)),
        ] {
            store.put(&key, &secret).unwrap();
            let name = signing_entry_name(&key);
            let head = raw(&name).unwrap();
            assert!(head.starts_with("mailo-parts:"), "{head}");
            assert!(raw(&format!("{name}#1")).is_some());
            assert_eq!(store.get(&key).unwrap(), secret);
            store.forget(&key).unwrap();
            assert!(raw(&name).is_none());
            assert!(raw(&format!("{name}#1")).is_none());
            assert!(store.get(&key).is_err());
        }
    }

    /// Two Profiles may hold the same key. Each keeps its own entry, under its own service, and
    /// forgetting one leaves the other.
    #[test]
    fn two_services_keep_the_same_key_apart() {
        let mock = keyring_core::mock::Store::new().unwrap();
        let default = KeyringSigningStore::over(mock.clone());
        let other = KeyringSigningStore::over_service(mock.clone(), "mailo/0123abcd");
        let (first, second) = (
            SigningSecret::OpenPgp("first".into()),
            SigningSecret::OpenPgp("second".into()),
        );
        default.put(&pgp_ref(), &first).unwrap();
        assert!(other.get(&pgp_ref()).is_err(), "the other service has none");
        other.put(&pgp_ref(), &second).unwrap();
        assert_eq!(default.get(&pgp_ref()).unwrap(), first);
        assert_eq!(other.get(&pgp_ref()).unwrap(), second);
        // The default's entry is where every earlier build kept it.
        let name = signing_entry_name(&pgp_ref());
        assert!(
            StoreSlots::new(mock.clone(), "mailo", Limit::None)
                .read(&name)
                .unwrap()
                .is_some()
        );
        other.forget(&pgp_ref()).unwrap();
        assert!(other.get(&pgp_ref()).is_err());
        assert_eq!(default.get(&pgp_ref()).unwrap(), first);
    }
}
