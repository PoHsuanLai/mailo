//! Where passwords and tokens live: the platform keyring, never SQLite.

mod chunks;

use crate::RuntimeError;
use chunks::{Limit, Slots};
use keyring_core::CredentialStore;
use mail_domain::{Credential, SecretKey, SecretPurpose};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// A store of credentials.
///
/// A trait because the tests must not touch the user's real keyring, and because a headless
/// machine may have no keyring at all.
pub trait Secrets: Send + Sync {
    fn get(&self, key: &SecretKey) -> Result<Credential, RuntimeError>;
    fn put(&self, key: &SecretKey, value: &Credential) -> Result<(), RuntimeError>;
    fn forget(&self, key: &SecretKey) -> Result<(), RuntimeError>;
}

const SERVICE: &str = "mailo";

fn entry_name(key: &SecretKey) -> String {
    let purpose = match key.purpose {
        SecretPurpose::IncomingPassword => "incoming",
        SecretPurpose::OutgoingPassword => "outgoing",
        SecretPurpose::OAuthRefresh => "oauth",
        SecretPurpose::AddressBook => "carddav",
        // Named by the key alone: see `SecretPurpose::OpenPgp` for why the account is not part
        // of it. The prefix cannot collide with an account's entries, which begin with a UUID.
        SecretPurpose::OpenPgp(fingerprint) => return format!("openpgp:{fingerprint}"),
        SecretPurpose::Smime(fingerprint) => return format!("smime:{fingerprint}"),
    };
    format!("{}:{}", key.account, purpose)
}

/// The platform keyring: the Secret Service on Linux and the BSDs, the login keychain on macOS,
/// the Credential Manager on Windows. Which one is chosen by target in `Cargo.toml` and in
/// [`open`] below, and nowhere else.
#[derive(Debug, Default)]
pub struct KeyringSecrets;

impl Secrets for KeyringSecrets {
    fn get(&self, key: &SecretKey) -> Result<Credential, RuntimeError> {
        let name = entry_name(key);
        let stored = off_runtime(move || chunks::get(&Keyring, &name))?
            .ok_or_else(|| RuntimeError::Secrets("no such credential".to_owned()))?;
        // JSON rather than a bare string so an OAuth credential keeps its expiry and refresh
        // token. The keyring holds one opaque value per entry either way.
        serde_json::from_str(&stored)
            .map_err(|e| RuntimeError::Secrets(format!("stored credential is unreadable: {e}")))
    }

    fn put(&self, key: &SecretKey, value: &Credential) -> Result<(), RuntimeError> {
        let name = entry_name(key);
        let encoded = serde_json::to_string(value)
            .map_err(|e| RuntimeError::Secrets(format!("cannot encode credential: {e}")))?;
        off_runtime(move || chunks::put(&Keyring, &name, &encoded, LIMIT))
    }

    fn forget(&self, key: &SecretKey) -> Result<(), RuntimeError> {
        let name = entry_name(key);
        off_runtime(move || chunks::forget(&Keyring, &name))
    }
}

/// How much one entry holds: 2560 bytes of UTF-16 in the Credential Manager. The Secret Service
/// and the Keychain have no limit a credential meets, so there every value is one entry, as it
/// always was.
#[cfg(windows)]
const LIMIT: Limit = Limit::Utf16Units(1280);
#[cfg(not(windows))]
const LIMIT: Limit = Limit::None;

/// This platform's credential store, as [`chunks`] reads and writes it.
struct Keyring;

impl Keyring {
    fn entry(name: &str) -> Result<keyring_core::Entry, RuntimeError> {
        store()?.build(SERVICE, name, None).map_err(refused)
    }
}

impl Slots for Keyring {
    fn read(&self, name: &str) -> Result<Option<String>, RuntimeError> {
        #[cfg(debug_assertions)]
        if let Some(dir) = scenario::dir() {
            return scenario::read(&dir, name);
        }
        match Keyring::entry(name)?.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(e) => Err(refused(e)),
        }
    }

    fn write(&self, name: &str, value: &str) -> Result<(), RuntimeError> {
        #[cfg(debug_assertions)]
        if let Some(dir) = scenario::dir() {
            return scenario::write(&dir, name, value);
        }
        Keyring::entry(name)?.set_password(value).map_err(refused)
    }

    fn delete(&self, name: &str) -> Result<(), RuntimeError> {
        #[cfg(debug_assertions)]
        if let Some(dir) = scenario::dir() {
            return scenario::delete(&dir, name);
        }
        match Keyring::entry(name)?.delete_credential() {
            // Already gone is the state we wanted.
            Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
            Err(e) => Err(refused(e)),
        }
    }
}

/// A directory of plain files standing in for the keyring, in debug builds only.
///
/// `dev/scenarios` runs the real `mailo watch` on a private bus with no Secret Service on it, and
/// must never reach the person's keyring; `MAILO_TEST_SECRETS_DIR` names a scratch directory and
/// each credential is a file in it. Compiled out of a release build, like sill's `--fake-ddc`: a
/// binary a person installs has no way to keep a password in a file.
#[cfg(debug_assertions)]
mod scenario {
    use crate::RuntimeError;
    use std::path::{Path, PathBuf};

    pub(super) fn dir() -> Option<PathBuf> {
        std::env::var_os("MAILO_TEST_SECRETS_DIR").map(PathBuf::from)
    }

    fn file(dir: &Path, name: &str) -> PathBuf {
        dir.join(name.replace([':', '#', '/'], "_"))
    }

    fn failed(e: std::io::Error) -> RuntimeError {
        RuntimeError::Secrets(format!("the scenario secrets directory: {e}"))
    }

    pub(super) fn read(dir: &Path, name: &str) -> Result<Option<String>, RuntimeError> {
        match std::fs::read_to_string(file(dir, name)) {
            Ok(value) => Ok(Some(value)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(failed(e)),
        }
    }

    pub(super) fn write(dir: &Path, name: &str, value: &str) -> Result<(), RuntimeError> {
        std::fs::create_dir_all(dir).map_err(failed)?;
        std::fs::write(file(dir, name), value).map_err(failed)
    }

    pub(super) fn delete(dir: &Path, name: &str) -> Result<(), RuntimeError> {
        match std::fs::remove_file(file(dir, name)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(failed(e)),
        }
    }
}

fn refused(e: keyring_core::Error) -> RuntimeError {
    RuntimeError::Secrets(e.to_string())
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
        .map_err(|_| RuntimeError::Secrets("the keyring was poisoned by a panic".to_owned()))?;
    if let Some(store) = opened.as_ref() {
        return Ok(store.clone());
    }
    let store = open()
        .map_err(|e| RuntimeError::Secrets(format!("no keyring to keep passwords in: {e}")))?;
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
compile_error!("mailo keeps credentials in the platform keyring, and knows none for this target");

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

/// An in-memory store. Tests only — it never reaches the user's keyring and never persists.
#[derive(Debug, Default)]
pub struct MapSecrets {
    entries: Mutex<HashMap<String, Credential>>,
}

impl Secrets for MapSecrets {
    fn get(&self, key: &SecretKey) -> Result<Credential, RuntimeError> {
        self.entries
            .lock()
            .expect("MapSecrets mutex poisoned")
            .get(&entry_name(key))
            .cloned()
            .ok_or_else(|| RuntimeError::Secrets("no such credential".to_owned()))
    }

    fn put(&self, key: &SecretKey, value: &Credential) -> Result<(), RuntimeError> {
        self.entries
            .lock()
            .expect("MapSecrets mutex poisoned")
            .insert(entry_name(key), value.clone());
        Ok(())
    }

    fn forget(&self, key: &SecretKey) -> Result<(), RuntimeError> {
        self.entries
            .lock()
            .expect("MapSecrets mutex poisoned")
            .remove(&entry_name(key));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, TimeDelta};
    use mail_domain::AccountId;

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

    fn key(purpose: SecretPurpose) -> SecretKey {
        SecretKey {
            account: AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-00000000c0de")),
            purpose,
        }
    }

    #[test]
    fn a_credential_round_trips_with_its_expiry_intact() {
        // The reason entries are JSON rather than a bare password: an OAuth credential that
        // lost its refresh token or its expiry would work for an hour and then fail forever.
        let store = MapSecrets::default();
        let at =
            DateTime::from_timestamp(1_700_000_000, 0).unwrap() + TimeDelta::try_hours(1).unwrap();
        let cred = Credential::OAuth {
            access: "ya29.access".into(),
            refresh: "1//refresh".into(),
            expires_at: at,
        };
        store.put(&key(SecretPurpose::OAuthRefresh), &cred).unwrap();
        assert_eq!(store.get(&key(SecretPurpose::OAuthRefresh)).unwrap(), cred);
    }

    #[test]
    fn purposes_do_not_collide() {
        // Incoming and outgoing may legitimately hold different credentials for one account.
        let store = MapSecrets::default();
        store
            .put(
                &key(SecretPurpose::IncomingPassword),
                &Credential::Password("in".into()),
            )
            .unwrap();
        store
            .put(
                &key(SecretPurpose::OutgoingPassword),
                &Credential::Password("out".into()),
            )
            .unwrap();
        assert_eq!(
            store.get(&key(SecretPurpose::IncomingPassword)).unwrap(),
            Credential::Password("in".into())
        );
        assert_eq!(
            store.get(&key(SecretPurpose::OutgoingPassword)).unwrap(),
            Credential::Password("out".into())
        );
    }

    #[test]
    fn forgetting_something_absent_is_not_an_error() {
        // Called on account removal and on reauthentication, where already-gone is success.
        let store = MapSecrets::default();
        store.forget(&key(SecretPurpose::OAuthRefresh)).unwrap();
    }

    #[test]
    fn a_missing_credential_reports_needs_reauth() {
        use mail_domain::{Retry, Retryable};
        let store = MapSecrets::default();
        let err = store
            .get(&key(SecretPurpose::IncomingPassword))
            .unwrap_err();
        // The outbox must prompt rather than retry: no amount of waiting creates a password.
        assert!(matches!(err.retry(), Retry::NeedsReauth), "{err:?}");
    }
}
