//! What goes wrong keeping or minting a credential.

use crate::SecretTrouble;
use mail_domain::{Retry, Retryable};
use mail_net::{Failure, NetError};
use std::time::Duration;

/// A credential could not be read, kept, renewed or minted.
#[derive(Debug, thiserror::Error)]
pub enum CredentialError {
    /// The store, the issuer or the sign-in said no. What it asks of the person is
    /// [`SecretTrouble::of`]'s to say.
    #[error("secret store: {0}")]
    Secrets(#[source] Failure),
    /// A file the credentials' configuration lives in could not be read or written.
    #[error("io: {0}")]
    Io(#[source] Failure),
    /// The issuer could not be reached, or the connection to it failed.
    #[error(transparent)]
    Net(#[from] NetError),
}

impl CredentialError {
    /// The issuer, or the way to it, did not answer.
    pub(crate) fn connect(why: Failure) -> Self {
        CredentialError::Net(NetError::Connect(why))
    }
}

impl Retryable for CredentialError {
    fn retry(&self) -> Retry {
        match self {
            // Missing is a sign-in to ask for; a locked keyring is a wait (`SecretTrouble`).
            CredentialError::Secrets(_) => SecretTrouble::of(self).retry(),
            CredentialError::Io(_) => Retry::After(Duration::from_secs(5)),
            CredentialError::Net(error) => error.retry(),
        }
    }
}
