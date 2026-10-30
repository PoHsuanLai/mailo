//! OpenPGP and S/MIME for mail, and the check of a Verified Mark Certificate. Pure: bytes in,
//! values out.
//!
//! Its own crate because signing and sealing are neither parsing nor building: a message is made
//! with `mail_mime::build`, and this takes the bytes it made and wraps them, or takes a received
//! message and unwraps it. Callers compose the two. The crypto stack is about twenty crates, so
//! each half is behind a feature that a dependent turns on for itself.

mod error;
#[cfg(feature = "openpgp")]
pub mod openpgp;
#[cfg(feature = "smime")]
pub mod smime;
#[cfg(feature = "testing")]
pub mod testing;
#[cfg(feature = "smime")]
pub mod vmc;

pub use error::CryptoError;

#[doc = include_str!("../README.md")]
#[cfg(doctest)]
pub struct ReadmeDoctests;
