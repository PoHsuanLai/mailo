//! mailo's own store for signing keys: an OpenPGP secret key or an S/MIME private key, in the
//! platform keyring, never SQLite.
//!
//! Account secrets (passwords, tokens) are not here: they are porter's, filed through
//! `porter_secrets::Secrets` (`account_secrets.rs`). A signing key is mailo's alone (an account
//! has none of them, and the accounts daemon never holds one), so it keeps the entries it has
//! always had, service `mailo`, named by the key, in the JSON of [`stored`], frozen by
//! `tests/fixtures/keyring/`. What does not fit one entry on the Windows Credential Manager (an
//! armored key routinely exceeds 2560 bytes) is cut into parts by [`chunks`].
//!
//! This path is blocking keyring-core calls, run on a thread of its own ([`off_runtime`]). That
//! is allowed here and only here: a signing key is read when a message is signed or opened, from
//! the thread that does it, never from a sync's runtime.

pub(crate) mod chunks;
pub(crate) mod stored;

use crate::RuntimeError;
use crate::error::Failure;
use chunks::{Limit, Slots};
use keyring_core::CredentialStore;
use mail_domain::signing::{SigningKeyId, SigningKeyRef, SigningSecret};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
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

/// The keyring service every entry of mailo's, signing keys and earlier builds' account secrets
/// alike, is filed under.
pub(crate) const SERVICE: &str = "mailo";

/// Named by the key alone: see [`SigningKeyId`] for why the account is not part of it. The
/// prefix cannot collide with an account's entries, which begin with a UUID.
fn signing_entry_name(key: &SigningKeyRef) -> String {
    match key.key {
        SigningKeyId::OpenPgp(fingerprint) => format!("openpgp:{fingerprint}"),
        SigningKeyId::Smime(fingerprint) => format!("smime:{fingerprint}"),
    }
}

fn signing_secret(stored: Stored) -> Result<SigningSecret, RuntimeError> {
    match stored {
        Stored::OpenPgp(armored) => Ok(SigningSecret::OpenPgp(armored)),
        Stored::SmimeKey(pem) => Ok(SigningSecret::SmimeKey(pem)),
        Stored::Password(_) | Stored::OAuth { .. } => Err(RuntimeError::Secrets(Failure::said(
            "the keyring entry holds an account's credential, not a signing key",
        ))),
    }
}

fn stored_signing(value: &SigningSecret) -> Stored {
    match value {
        SigningSecret::OpenPgp(armored) => Stored::OpenPgp(armored.clone()),
        SigningSecret::SmimeKey(pem) => Stored::SmimeKey(pem.clone()),
    }
}

/// Signing keys in a keyring: the Secret Service on Linux and the BSDs, the login keychain on
/// macOS, the Credential Manager on Windows (which store is chosen by target in `Cargo.toml` and
/// in [`open`] below, and nowhere else), or one given keyring-core store, which is how the tests
/// use keyring-core's mock with the Credential Manager's size limit on any platform.
///
/// Every call runs on a thread of its own, whichever it is over: see [`off_runtime`].
pub struct KeyringSigningStore {
    slots: Box<dyn Slots + Send + Sync>,
    limit: Limit,
}

impl std::fmt::Debug for KeyringSigningStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("KeyringSigningStore")
    }
}

impl Default for KeyringSigningStore {
    /// The platform's keyring.
    fn default() -> Self {
        Self::with_slots(Box::new(Keyring), LIMIT)
    }
}

impl KeyringSigningStore {
    pub(crate) fn with_slots(slots: Box<dyn Slots + Send + Sync>, limit: Limit) -> Self {
        Self { slots, limit }
    }

    /// Keeps signing keys in `store`, each in one entry.
    pub fn over(store: Arc<CredentialStore>) -> Self {
        Self::with_slots(Box::new(StoreSlots(store)), Limit::None)
    }

    /// Keeps signing keys in `store`, cut into parts of at most `units` UTF-16 code units.
    pub fn chunked(store: Arc<CredentialStore>, units: usize) -> Self {
        Self::with_slots(Box::new(StoreSlots(store)), Limit::Utf16Units(units))
    }
}

impl SigningStore for KeyringSigningStore {
    fn get(&self, key: &SigningKeyRef) -> Result<SigningSecret, RuntimeError> {
        let name = signing_entry_name(key);
        off_runtime(|| read_with(self.slots.as_ref(), &name))
    }

    fn put(&self, key: &SigningKeyRef, value: &SigningSecret) -> Result<(), RuntimeError> {
        let (name, stored) = (signing_entry_name(key), stored_signing(value));
        off_runtime(|| write_with(self.slots.as_ref(), self.limit, &name, &stored))
    }

    fn forget(&self, key: &SigningKeyRef) -> Result<(), RuntimeError> {
        let name = signing_entry_name(key);
        off_runtime(|| chunks::forget(self.slots.as_ref(), &name))
    }
}

fn read_with(slots: &dyn Slots, name: &str) -> Result<SigningSecret, RuntimeError> {
    let stored = chunks::get(slots, name)?
        .ok_or_else(|| RuntimeError::Secrets(Failure::said("no such signing key")))?;
    signing_secret(decode(&stored)?)
}

fn write_with(
    slots: &dyn Slots,
    limit: Limit,
    name: &str,
    value: &Stored,
) -> Result<(), RuntimeError> {
    let encoded = serde_json::to_string(value)
        .map_err(|e| RuntimeError::Secrets(Failure::new("cannot encode signing key", e)))?;
    chunks::put(slots, name, &encoded, limit)
}

/// An entry's JSON as [`Stored`], the shape every earlier build wrote.
pub(crate) fn decode(text: &str) -> Result<Stored, RuntimeError> {
    serde_json::from_str(text)
        .map_err(|e| RuntimeError::Secrets(Failure::new("stored credential is unreadable", e)))
}

/// One keyring-core store's entries under the service `mailo`, as [`chunks`] reads and writes them.
#[derive(Clone)]
pub(crate) struct StoreSlots(pub(crate) Arc<CredentialStore>);

impl std::fmt::Debug for StoreSlots {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StoreSlots")
    }
}

impl Slots for StoreSlots {
    fn read(&self, name: &str) -> Result<Option<String>, RuntimeError> {
        match self
            .0
            .build(SERVICE, name, None)
            .map_err(refused)?
            .get_password()
        {
            Ok(value) => Ok(Some(value)),
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(e) => Err(refused(e)),
        }
    }

    fn write(&self, name: &str, value: &str) -> Result<(), RuntimeError> {
        self.0
            .build(SERVICE, name, None)
            .map_err(refused)?
            .set_password(value)
            .map_err(refused)
    }

    fn delete(&self, name: &str) -> Result<(), RuntimeError> {
        match self
            .0
            .build(SERVICE, name, None)
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
pub(crate) const LIMIT: Limit = Limit::Utf16Units(1280);
#[cfg(not(windows))]
pub(crate) const LIMIT: Limit = Limit::None;

/// This platform's credential store, as [`chunks`] reads and writes it.
#[derive(Debug)]
pub(crate) struct Keyring;

impl Slots for Keyring {
    fn read(&self, name: &str) -> Result<Option<String>, RuntimeError> {
        #[cfg(debug_assertions)]
        if let Some(dir) = scenario::dir() {
            return scenario::read(&dir, name);
        }
        StoreSlots(store()?).read(name)
    }

    fn write(&self, name: &str, value: &str) -> Result<(), RuntimeError> {
        #[cfg(debug_assertions)]
        if let Some(dir) = scenario::dir() {
            return scenario::write(&dir, name, value);
        }
        StoreSlots(store()?).write(name, value)
    }

    fn delete(&self, name: &str) -> Result<(), RuntimeError> {
        #[cfg(debug_assertions)]
        if let Some(dir) = scenario::dir() {
            return scenario::delete(&dir, name);
        }
        StoreSlots(store()?).delete(name)
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

    pub(crate) fn dir() -> Option<PathBuf> {
        std::env::var_os("MAILO_TEST_SECRETS_DIR").map(PathBuf::from)
    }

    fn file(dir: &Path, name: &str) -> PathBuf {
        dir.join(name.replace([':', '#', '/'], "_"))
    }

    fn failed(e: std::io::Error) -> RuntimeError {
        RuntimeError::Secrets(Failure::new("the scenario secrets directory", e))
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

/// The store, opened on first use and kept.
///
/// A store that could not be opened is tried again on the next use, not remembered as missing:
/// a Secret Service that was not up when the first sync ran (a session whose keyring daemon
/// starts late) is up for the second.
fn store() -> Result<Arc<CredentialStore>, RuntimeError> {
    static OPENED: Mutex<Option<Arc<CredentialStore>>> = Mutex::new(None);
    let mut opened = OPENED
        .lock()
        .map_err(|_| RuntimeError::Secrets(Failure::said("the keyring was poisoned by a panic")))?;
    if let Some(store) = opened.as_ref() {
        return Ok(store.clone());
    }
    let store = open().map_err(|e| {
        RuntimeError::Secrets(Failure::new("no keyring to keep signing keys in", e))
    })?;
    *opened = Some(store.clone());
    Ok(store)
}

/// Linux and the BSDs: the Secret Service, which GNOME Keyring, KWallet and KeePassXC provide.
#[cfg(all(
    unix,
    not(any(target_os = "macos", target_os = "ios", target_os = "android"))
))]
fn open() -> keyring_core::Result<Arc<CredentialStore>> {
    Ok(zbus_secret_service_keyring_store::Store::new()?)
}

/// macOS: the user's login keychain.
#[cfg(target_os = "macos")]
fn open() -> keyring_core::Result<Arc<CredentialStore>> {
    Ok(apple_native_keyring_store::keychain::Store::new()?)
}

/// Windows: the Credential Manager, as generic credentials named `<entry>.mailo`.
#[cfg(windows)]
fn open() -> keyring_core::Result<Arc<CredentialStore>> {
    Ok(windows_native_keyring_store::Store::new()?)
}

#[cfg(not(any(
    windows,
    target_os = "macos",
    all(
        unix,
        not(any(target_os = "macos", target_os = "ios", target_os = "android"))
    )
)))]
compile_error!("mailo keeps signing keys in the platform keyring, and knows none for this target");

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

/// An in-memory store. Tests only: it never reaches the user's keyring and never persists.
#[derive(Debug, Default)]
pub struct MapSigningStore {
    entries: Mutex<HashMap<String, Stored>>,
}

impl SigningStore for MapSigningStore {
    fn get(&self, key: &SigningKeyRef) -> Result<SigningSecret, RuntimeError> {
        let held = self
            .entries
            .lock()
            .expect("MapSigningStore mutex poisoned")
            .get(&signing_entry_name(key))
            .cloned()
            .ok_or_else(|| RuntimeError::Secrets(Failure::said("no such signing key")))?;
        signing_secret(held)
    }

    fn put(&self, key: &SigningKeyRef, value: &SigningSecret) -> Result<(), RuntimeError> {
        self.entries
            .lock()
            .expect("MapSigningStore mutex poisoned")
            .insert(signing_entry_name(key), stored_signing(value));
        Ok(())
    }

    fn forget(&self, key: &SigningKeyRef) -> Result<(), RuntimeError> {
        self.entries
            .lock()
            .expect("MapSigningStore mutex poisoned")
            .remove(&signing_entry_name(key));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mail_domain::id::account_id_from_uuid;
    use mail_domain::signing::SigningKeyId;

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
        let store =
            KeyringSigningStore::with_slots(Box::new(StartsRuntime(Mem::default())), Limit::None);
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
            StoreSlots(mock.clone())
                .write(
                    &signing_entry_name(&key),
                    &serde_json::to_string(&entries[0]).unwrap(),
                )
                .unwrap();
            let secret = store.get(&key).unwrap();
            store.put(&key, &secret).unwrap();
            let kept = StoreSlots(mock.clone())
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
        let slots = StoreSlots(mock);
        for (key, secret) in [
            (pgp_ref(), SigningSecret::OpenPgp(armored)),
            (smime, SigningSecret::SmimeKey(pem)),
        ] {
            store.put(&key, &secret).unwrap();
            let name = signing_entry_name(&key);
            let head = slots.read(&name).unwrap().unwrap();
            assert!(head.starts_with("mailo-parts:"), "{head}");
            assert!(slots.read(&format!("{name}#1")).unwrap().is_some());
            assert_eq!(store.get(&key).unwrap(), secret);
            store.forget(&key).unwrap();
            assert!(slots.read(&name).unwrap().is_none());
            assert!(slots.read(&format!("{name}#1")).unwrap().is_none());
            assert!(store.get(&key).is_err());
        }
    }
}
