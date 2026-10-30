//! Why a key, a certificate or a sealed message could not be read or made.
//!
//! Every variant is a fact about the bytes or the keys, never the network, so it answers
//! [`Retryable`] with a fatal verdict: asking again re-runs the same failure on the same input.

use mail_domain::{Retry, Retryable};

/// A key, a certificate or a sealed message did not hold together, or could not be made.
///
/// Never a panic: every byte reaching this crate came off a network a stranger controls.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum CryptoError {
    /// OpenPGP data that could not be read or made: a malformed key, a damaged message, a
    /// cryptographic failure. The text is the OpenPGP library's.
    #[error("OpenPGP: {0}")]
    OpenPgp(String),
    /// The secret key needs its passphrase, and none was given or the one given is wrong.
    #[error("the OpenPGP key {0} needs its passphrase")]
    KeyLocked(mail_domain::Fingerprint),
    /// Signing was asked for with no secret key to sign with.
    #[error("there is no OpenPGP key to sign with")]
    NoSigningKey,
    /// Encryption was asked for with no key to encrypt to.
    #[error("there is no OpenPGP key to encrypt to")]
    NoRecipientKeys,
    /// A recipient's key has no part that can be encrypted to (revoked, expired, sign-only).
    #[error("the OpenPGP key {0} cannot be encrypted to")]
    CannotEncryptTo(mail_domain::Fingerprint),
    /// S/MIME data that could not be read or made: a malformed certificate, message or PKCS#12
    /// file, an algorithm refused or not supported. The text says which.
    #[error("S/MIME: {0}")]
    Smime(String),
    /// A PKCS#12 file's password is wrong.
    #[error("that password does not open the PKCS#12 file")]
    WrongPassword,
}

impl Retryable for CryptoError {
    fn retry(&self) -> Retry {
        Retry::Fatal(self.to_string())
    }
}
