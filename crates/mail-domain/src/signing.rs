//! The user's own signing and decryption keys, and where they are kept.
//!
//! mailo's, not porter's: an account's secrets (passwords, OAuth tokens) are the accounts
//! system's business, but an OpenPGP or S/MIME key belongs to the person and outlives any one
//! account, so it stays in mailo's own store (design/31, D14).
//!
//! [`SigningKeyRef`] names one key; [`SigningSecret`] is its secret half. Neither is an account
//! secret and neither can be mistaken for one.

use crate::id::AccountId;
use crate::pgp::Fingerprint;
use crate::smime::CertFingerprint;
use std::fmt;

/// Which key, by the name its kind is known by everywhere else.
///
/// Keyed by the fingerprint, not by the account or identity: one key may serve identities on
/// several accounts, and a key replaced on an identity must still decrypt the mail that was
/// encrypted to it. The keyring entry is therefore named by the fingerprint alone, and
/// [`SigningKeyRef::account`] records only which account it was kept for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SigningKeyId {
    /// The OpenPGP key with this fingerprint.
    OpenPgp(Fingerprint),
    /// The S/MIME certificate with this fingerprint.
    Smime(CertFingerprint),
}

/// One signing key: the account it was kept for and which key it is.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SigningKeyRef {
    /// The account the key was kept for.
    pub account: AccountId,
    /// Which key.
    pub key: SigningKeyId,
}

/// The secret half of a signing key. Lives in the platform keyring and never in SQLite.
#[derive(Clone, PartialEq, Eq)]
pub enum SigningSecret {
    /// An OpenPGP transferable secret key, ASCII-armored. Protected by its own passphrase when
    /// it was imported with one, and by the keyring alone when it was generated here.
    OpenPgp(String),
    /// An S/MIME private key, PKCS#8 PEM, unencrypted: the keyring is its protection. Taken out
    /// of the PKCS#12 file it was imported from, whose password is not kept.
    SmimeKey(String),
}

// Written by hand, not derived: a derived Debug puts the key in every log line, panic message
// and error chain that ever touches this value.
impl fmt::Debug for SigningSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SigningSecret::OpenPgp(_) => f.write_str("SigningSecret::OpenPgp(<redacted>)"),
            SigningSecret::SmimeKey(_) => f.write_str("SigningSecret::SmimeKey(<redacted>)"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_never_shows_the_key() {
        // A security property, not a formatting preference: a derived `Debug` puts the key into
        // every log line, panic message and error chain that ever formats this value.
        for secret in [
            SigningSecret::OpenPgp("-----BEGIN PGP PRIVATE KEY BLOCK-----".to_owned()),
            SigningSecret::SmimeKey("PRIVATE-KEY-MATERIAL".to_owned()),
        ] {
            let alone = format!("{secret:?}");
            let nested = format!("{:?}", vec![Some(&secret)]);
            for shown in [alone, nested] {
                assert!(!shown.contains("PRIVATE"), "{shown}");
                assert!(shown.contains("redacted"), "{shown}");
            }
        }
    }
}
