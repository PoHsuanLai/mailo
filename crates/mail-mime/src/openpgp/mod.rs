//! OpenPGP for mail: keys, PGP/MIME (RFC 3156) and inline PGP, Autocrypt, Web Key Directory.
//!
//! Built on rPGP (the `pgp` crate, MIT OR Apache-2.0). Pure like the rest of this crate: the
//! random number generator and every instant are arguments, keys arrive as values, and nothing
//! here reads a keyring, a store or a socket. `mail-app` decides which keys to hand in;
//! `mail-runtime` fetches from Web Key Directories.
//!
//! Reading is [`open`]; sending is [`seal`]. Decrypted content only ever leaves this module as
//! the return value of [`open`] — it is the caller's to show and not to keep.

mod autocrypt;
pub(crate) mod entity;
mod keys;
mod open;
mod seal;
pub mod wkd;

pub use autocrypt::{AutocryptHeader, autocrypt_field, autocrypt_of, with_field};
pub use keys::{Cert, Protection, ReadKey, SecretCert, generate, read_keys};
pub use open::{Opened, encrypted_to, open};
pub use seal::{Sealing, seal};

use crate::MimeError;
use mail_domain::{Fingerprint, KeyId, KeyTrust};

/// What rPGP needs of a random number generator. The caller supplies one — the OS generator in
/// the application, a seeded one in a test.
pub trait Rng: rand::RngCore + rand::CryptoRng {}
impl<T: rand::RngCore + rand::CryptoRng> Rng for T {}

/// A public key and how far the user trusts it: what signatures are checked against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnownCert {
    pub cert: Cert,
    pub trust: KeyTrust,
}

/// A secret key offered for decryption, with the passphrase to try on it.
///
/// `passphrase` is empty for a key that has none, and for one whose passphrase has not been
/// asked for yet — which [`open`] reports as [`mail_domain::Encryption::Locked`].
#[derive(Clone, PartialEq, Eq)]
pub struct Unlocking {
    pub key: SecretCert,
    pub passphrase: String,
}

impl std::fmt::Debug for Unlocking {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Unlocking")
            .field("key", &self.key)
            .field("passphrase", &"<redacted>")
            .finish()
    }
}

/// The keys [`open`] may use.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Keys {
    /// Public keys to check signatures against.
    pub certs: Vec<KnownCert>,
    /// The user's secret keys that the message is encrypted to, from [`encrypted_to`].
    pub secrets: Vec<Unlocking>,
}

fn pgp_error(e: pgp::errors::Error) -> MimeError {
    MimeError::OpenPgp(e.to_string())
}

fn convert_fingerprint(fp: &pgp::types::Fingerprint) -> Option<Fingerprint> {
    Fingerprint::from_bytes(fp.as_bytes())
}

fn convert_key_id(id: &pgp::types::KeyId) -> KeyId {
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(id.as_ref());
    KeyId(bytes)
}
