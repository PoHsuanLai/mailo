//! S/MIME from the user's side: identities and certificates, reading protected mail, sending
//! it.
//!
//! The pieces mirror OpenPGP's (`crate::pgp`): [`certs`] imports, exports, trusts and forgets
//! certificates and identities; [`read`] opens a message when the reader shows it; [`send`]
//! seals an outgoing one as Send is pressed. The cryptography is `mail_mime::smime`'s, the
//! keyring and the system's trust anchors are `mail_runtime::smime`'s, and this module decides
//! which certificates to use.
//!
//! Revocation is never checked: nothing here fetches a CRL or asks an OCSP responder, so a
//! certificate its authority has revoked is believed as long as its chain and dates hold.

pub mod certs;
pub mod read;
pub mod send;

pub use certs::{Imported, cert_for, own_cert};
pub use read::{Protected, open_bytes, open_message};
pub use send::{check, outgoing};

use mail_domain::CertFingerprint;
use mail_mime::MimeError;
use mail_runtime::RuntimeError;
use mail_store::StoreError;

/// Why an S/MIME step could not be done, named so the caller can say what to do about it.
#[derive(Debug, thiserror::Error)]
pub enum SmimeError {
    /// Encryption was asked for and these recipients have no certificate to encrypt to.
    #[error(
        "no S/MIME certificate to encrypt to for {}; nothing was sent. A signed message from \
         them brings theirs, or import one with `mailo smime import <file>`; or send without \
         --encrypt",
        .0.join(", ")
    )]
    NoCertFor(Vec<String>),
    /// Encryption was asked for with blind recipients, whose certificates every recipient would
    /// see named.
    #[error(
        "an encrypted message cannot have Bcc recipients ({}): every recipient would see the \
         blind ones' certificates named. Send them a separate message",
        .0.join(", ")
    )]
    BlindRecipients(Vec<String>),
    /// Signing or encrypting was asked for from an identity with no current certificate.
    #[error(
        "{0} has no current S/MIME certificate of its own; import your identity with \
         `mailo smime import <file.p12>`"
    )]
    NoOwnCert(String),
    /// The user's own certificate has a key mail cannot be encrypted to here.
    #[error(
        "your S/MIME certificate for {0} cannot be encrypted to (only RSA certificates for mail \
         can), so the sent copy could not be read; send without --encrypt"
    )]
    OwnCertCannotEncrypt(String),
    /// A draft asks for OpenPGP and S/MIME at once.
    #[error(
        "a message is protected with OpenPGP or with S/MIME, not both; choose one before sending"
    )]
    BothProtections,
    #[error("{0} is not the address of any of your identities")]
    NoIdentity(String),
    #[error(
        "the certificate {fingerprint} is for {}, none of which is one of your identities",
        addresses.join(", ")
    )]
    NotYours {
        fingerprint: CertFingerprint,
        addresses: Vec<String>,
    },
    #[error("no S/MIME certificate {0}")]
    NoCert(String),
    #[error(
        "the private key of {0} is in your keyring, and deleting it means mail encrypted to it \
         can never be read again. Keep the PKCS#12 file you imported it from, then delete with \
         --with-secret"
    )]
    SecretWouldBeLost(CertFingerprint),
    /// A PKCS#12 file was given without its password.
    #[error(
        "that PKCS#12 file needs its password: set MAILO_SMIME_PASSWORD, or run from a terminal \
         to be asked"
    )]
    NoPassword,
    #[error("{0}")]
    Mime(#[from] MimeError),
    #[error("{0}")]
    Runtime(#[from] RuntimeError),
    #[error("store: {0}")]
    Store(#[from] StoreError),
}

/// A count that moves whenever the keys or certificates this process holds or trusts change —
/// imported, made, deleted, trusted, or learnt from arriving mail. Shared by OpenPGP and S/MIME
/// (`mail_runtime::epoch`): a cache of opened messages keeps the count it was made under and
/// opens them again when it has moved.
pub fn epoch() -> u64 {
    mail_runtime::epoch::keys()
}

fn epoch_changed() {
    mail_runtime::epoch::keys_changed();
}
