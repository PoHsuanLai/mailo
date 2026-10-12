//! What a test swaps in at the runtime's seams: an in-memory transport, account secrets that live
//! in a map, and a signing store that does too.
//!
//! Behind the `testing` feature, which a crate enables for its tests and the program never does.
//! None of it reaches a socket or the platform's keyring, so a test of the logic above (or of a
//! library built on this crate) needs no more than these to run in a sandbox.

use crate::account_secrets::AccountSecrets;
use crate::error::Failure;
use crate::signing_store::stored::Stored;
use crate::signing_store::{SigningStore, signing_entry_name, signing_secret, stored_signing};
use crate::{RuntimeError, Transport};
use mail_domain::signing::{SigningKeyRef, SigningSecret};
use porter_client::AuthenticatedStream;
use porter_core::stream::{DuplexEnd, duplex};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub use porter_secrets::MemorySecrets;

/// A connection that goes nowhere: the [`Transport`] a machine is driven over, and the far end a
/// test plays the server on. The transport is taken as already secure and already signed in, so
/// the machine over it must not authenticate, which is what a test of the engines wants.
pub fn memory_transport() -> (Transport, DuplexEnd) {
    let (app, server) = duplex(64 * 1024);
    let transport = Transport::relayed(AuthenticatedStream::Memory(app))
        .expect("an in-memory stream needs no socket");
    (transport, server)
}

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
