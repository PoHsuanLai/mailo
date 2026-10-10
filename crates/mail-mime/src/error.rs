//! Why bytes could not be read, or a message could not be built from what was given.
//!
//! Every type here is a fact about the bytes or the keys, never the network, so each answers
//! [`Retryable`] with a fatal verdict: asking again re-runs the same failure on the same input.

use mail_domain::{Retry, Retryable};
use std::io;

/// Something in the message, or in what we were asked to build, did not hold together.
///
/// Never a panic: every byte reaching this crate came off a network a stranger controls.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MimeError {
    #[error("message could not be parsed: {0}")]
    Unparseable(String),
    #[error("required header missing: {0}")]
    MissingHeader(&'static str),
    #[error("attachment {0} was referenced but not supplied")]
    MissingPart(String),
    #[error("cannot build a message with no recipients")]
    NoRecipients,
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

impl Retryable for MimeError {
    fn retry(&self) -> Retry {
        // All of these are facts about the bytes or the keys, not about the network. Retrying re-runs the
        // same failure on the same input forever.
        Retry::Fatal(self.to_string())
    }
}

/// Why a TXT string is not a BIMI record this client will use.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RecordError {
    /// It does not open with `v=BIMI1`.
    #[error("not a BIMI record")]
    NotBimi,
    /// A tag appears twice.
    #[error("the tag {0} appears twice")]
    Repeated(String),
    /// `l=` or `a=` is not a single `https:` URL.
    #[error("{0:?} is not a single https URL")]
    NotHttps(String),
}

impl Retryable for RecordError {
    fn retry(&self) -> Retry {
        Retry::Fatal(self.to_string())
    }
}

/// Why an mbox could not be read.
#[derive(Debug, thiserror::Error)]
pub enum MboxError {
    #[error("not an mbox: the first line is not a `From ` envelope line")]
    NotMbox,
    #[error("reading the mbox: {0}")]
    Io(#[from] io::Error),
}

impl Retryable for MboxError {
    fn retry(&self) -> Retry {
        // A file already on this computer: the same read gives the same answer.
        Retry::Fatal(self.to_string())
    }
}
