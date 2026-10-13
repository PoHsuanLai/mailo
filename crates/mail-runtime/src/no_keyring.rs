//! A keyring that holds nothing: what a window or handle gets when its root handed it no real
//! one, as a test's does. It answers every read and write with "no keyring", and forgetting from
//! it succeeds, as forgetting a missing secret does, so a removal touches no keyring of the
//! person's.

use crate::RuntimeError;
use crate::account_secrets::AccountSecrets;
use crate::error::Failure;
use crate::signing_store::SigningStore;
use mail_domain::signing::{SigningKeyRef, SigningSecret};
use porter_core::{AccountId, Credential, SecretKey};
use std::future::Future;
use std::pin::Pin;

/// The keyring of a window or handle that was given none: account secrets and signing keys alike.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoKeyring;

type Answer<'a, T> = Pin<Box<dyn Future<Output = Result<T, RuntimeError>> + Send + 'a>>;

fn none() -> RuntimeError {
    RuntimeError::Secrets(Failure::said("this window has no keyring"))
}

impl AccountSecrets for NoKeyring {
    fn get<'a>(&'a self, _: &'a SecretKey) -> Answer<'a, Credential> {
        Box::pin(async { Err(none()) })
    }

    fn put<'a>(&'a self, _: &'a SecretKey, _: &'a Credential) -> Answer<'a, ()> {
        Box::pin(async { Err(none()) })
    }

    fn forget<'a>(&'a self, _: &'a SecretKey) -> Answer<'a, ()> {
        Box::pin(async { Ok(()) })
    }

    fn forget_account<'a>(&'a self, _: &'a AccountId) -> Answer<'a, ()> {
        Box::pin(async { Ok(()) })
    }
}

impl SigningStore for NoKeyring {
    fn get(&self, _: &SigningKeyRef) -> Result<SigningSecret, RuntimeError> {
        Err(none())
    }

    fn put(&self, _: &SigningKeyRef, _: &SigningSecret) -> Result<(), RuntimeError> {
        Err(none())
    }

    fn forget(&self, _: &SigningKeyRef) -> Result<(), RuntimeError> {
        Ok(())
    }
}
