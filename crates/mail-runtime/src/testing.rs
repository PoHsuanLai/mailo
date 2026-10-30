//! What a test swaps in at the runtime's seams: account secrets that live
//! in a map, a signing store that does too, and OAuth clients that sign in nowhere.
//!
//! Behind the `testing` feature, which a crate enables for its tests and the program never does.
//! None of it reaches a socket or the platform's keyring, so a test of the logic above (or of a
//! library built on this crate) needs no more than these to run in a sandbox.

use crate::RuntimeError;
use crate::account_secrets::AccountSecrets;
use crate::signing_store::stored::Stored;
use crate::signing_store::{SigningStore, signing_entry_name, signing_secret, stored_signing};
use mail_domain::signing::{SigningKeyRef, SigningSecret};
use mail_net::Failure;
use porter_core::EndpointUrl;
use porter_oauth::ClientRegistry;
use porter_provider::{ClientEntry, Issuer, IssuerEndpoints};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub use porter_secrets::MemorySecrets;

/// Account secrets that live in a map, empty to begin with.
pub fn memory_secrets() -> Arc<dyn AccountSecrets> {
    Arc::new(MemorySecrets::default())
}

/// A store of signing keys that lives in a map. It never reaches the user's keyring and never
/// persists.
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

/// Stand-in Google and Microsoft clients, for a
/// test that wants a client at hand: ids that name no registration (`test-google-client`,
/// `test-microsoft-client`), whose sign-in and token endpoints are at `example.test`, so a test
/// that goes as far as a sign-in reaches neither Google nor Microsoft.
///
/// The program's registry is [`crate::clients::load_default`], which no test reads. A test about
/// a client that is missing hands an empty registry (`ClientRegistry::default()`) and says so.
pub fn fake_clients() -> ClientRegistry {
    fake_clients_for(&[Issuer::Google, Issuer::Microsoft])
}

/// [`fake_clients`] for `issuers` alone: a test about one provider at hand and another not.
pub fn fake_clients_for(issuers: &[Issuer]) -> ClientRegistry {
    let at = |issuer: Issuer, name: &str| {
        let url = |path: &str| {
            EndpointUrl::parse(&format!("https://{name}.example.test/{path}"))
                .unwrap_or_else(|_| unreachable!("an example.test URL is a URL"))
        };
        ClientEntry {
            endpoints: Some(IssuerEndpoints {
                authorize: url("authorize"),
                token: url("token"),
                revoke: None,
            }),
            ..crate::clients::entry(issuer, &format!("test-{name}-client"), None)
        }
    };
    crate::clients::registry_of(
        issuers
            .iter()
            .filter_map(|issuer| match issuer {
                Issuer::Google => Some(at(Issuer::Google, "google")),
                Issuer::Microsoft => Some(at(Issuer::Microsoft, "microsoft")),
                _ => None,
            })
            .collect(),
    )
}
