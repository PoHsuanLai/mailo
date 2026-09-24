//! S/MIME 4.0 for mail (RFC 8551): certificates (RFC 8550), CMS (RFC 5652), PKCS#12 identities.
//!
//! Built on the RustCrypto crates — `cms`, `x509-cert`, `der`, `rsa`, `p256`/`p384`, `aes`,
//! `cbc`, `aes-gcm`, `sha2` — all MIT OR Apache-2.0 and pure. Like the rest of this crate: the
//! random number generator and every instant are arguments, keys and trust anchors arrive as
//! values, and nothing here reads a keyring, a store, the system's certificate bundle or a
//! socket. `mail-app` decides which certificates and keys to hand in; `mail-runtime` reads the
//! keyring and the system's trust anchors.
//!
//! Reading is [`open`]; sending is [`seal`]; identities come from [`read_pkcs12`]. Decrypted
//! content only ever leaves this module as the return value of [`open`] — it is the caller's to
//! show and not to keep.
//!
//! What this does not do: fetch revocation information (no OCSP, no CRL download — nothing here
//! touches the network, and a certificate's revocation is therefore never known), key agreement
//! recipients (an elliptic-curve certificate can sign and be verified, but mail cannot be
//! encrypted to it or decrypted by it here), RSASSA-PSS signatures, and header protection
//! (RFC 8551 §3.1's wrapped `message/rfc822`): the subject of an encrypted message travels in
//! the clear, as every S/MIME client has always sent it.

mod asn1;
mod ber;
mod cert;
mod chain;
mod decrypt;
mod keys;
mod open;
mod pkcs12;
mod seal;
mod sign;
mod verify;

pub use cert::{Cert, read_certs};
pub use keys::{Identity, PrivateKey};
pub use open::{Keys, Opened, RecipientId, Signer, open, recipients};
pub use pkcs12::{read_pkcs12, write_pkcs12};
pub use seal::{Sealing, seal};

use crate::MimeError;

fn smime_error(what: &str, e: impl std::fmt::Display) -> MimeError {
    MimeError::Smime(format!("{what}: {e}"))
}
