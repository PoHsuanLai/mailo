//! Where passwords and tokens live: the platform keyring, never SQLite.

use crate::RuntimeError;
use mail_domain::{Credential, SecretKey, SecretPurpose};
use std::collections::HashMap;
use std::sync::Mutex;

/// A store of credentials.
///
/// A trait because the tests must not touch the user's real keyring, and because a headless
/// machine may have no Secret Service at all.
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

/// The platform keyring — Secret Service on Linux.
#[derive(Debug, Default)]
pub struct KeyringSecrets;

impl Secrets for KeyringSecrets {
    fn get(&self, key: &SecretKey) -> Result<Credential, RuntimeError> {
        let name = entry_name(key);
        let stored = off_runtime(move || {
            keyring::Entry::new(SERVICE, &name)
                .and_then(|entry| entry.get_password())
                .map_err(|e| RuntimeError::Secrets(e.to_string()))
        })?;
        // JSON rather than a bare string so an OAuth credential keeps its expiry and refresh
        // token. The keyring holds one opaque value per entry either way.
        serde_json::from_str(&stored)
            .map_err(|e| RuntimeError::Secrets(format!("stored credential is unreadable: {e}")))
    }

    fn put(&self, key: &SecretKey, value: &Credential) -> Result<(), RuntimeError> {
        let name = entry_name(key);
        let encoded = serde_json::to_string(value)
            .map_err(|e| RuntimeError::Secrets(format!("cannot encode credential: {e}")))?;
        off_runtime(move || {
            keyring::Entry::new(SERVICE, &name)
                .and_then(|entry| entry.set_password(&encoded))
                .map_err(|e| RuntimeError::Secrets(e.to_string()))
        })
    }

    fn forget(&self, key: &SecretKey) -> Result<(), RuntimeError> {
        let name = entry_name(key);
        off_runtime(move || {
            let entry = keyring::Entry::new(SERVICE, &name)
                .map_err(|e| RuntimeError::Secrets(e.to_string()))?;
            match entry.delete_credential() {
                Ok(()) => Ok(()),
                // Already gone is the state we wanted.
                Err(keyring::Error::NoEntry) => Ok(()),
                Err(e) => Err(RuntimeError::Secrets(e.to_string())),
            }
        })
    }
}

/// Run `work` on a thread of its own and wait for it.
///
/// The keyring reaches the Secret Service through zbus's blocking API, and a build that turns on
/// zbus's `tokio` feature anywhere (quire's settings crate does) makes that API start a Tokio
/// runtime of its own for each call. Starting one on a thread already driving a runtime panics,
/// and the keyring is read from inside every sync's runtime. A plain thread drives none, whichever
/// executor zbus was built for.
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
