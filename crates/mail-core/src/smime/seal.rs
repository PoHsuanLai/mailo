//! Sending: an outgoing message made what its draft asks — signed, encrypted, both.
//!
//! Done when the bytes are frozen, as Send is pressed, for the reasons `crate::pgp::seal` gives.
//! Encryption goes to every `To` and `Cc` recipient's certificate and to the sender's own, so the
//! copy the server files in Sent can be read. A recipient with no certificate refuses the send,
//! naming them: nothing is ever sent in the clear because a certificate was missing.
//!
//! `Bcc` with encryption is refused, for OpenPGP's reason: an enveloped message names every
//! certificate it is encrypted to by issuer and serial number, so each recipient would learn the
//! blind ones'.
//!
//! A draft asks for OpenPGP or S/MIME, not both: the two wrap the same content in two different
//! envelopes, and one inside the other opens for nobody. [`Seal::check`] refuses it before anything is
//! built.

use super::SmimeError;
use super::certs::{cert_for, identity_of, own_cert};
use crate::pgp::Ask;
use crate::protect::Seal;
use chrono::{DateTime, Utc};
use mail_domain::*;
use mail_mime::smime::{self, Cert, Identity as Signer, Sealing};
use mail_runtime::SigningStore;
use mail_store::SqliteStore;

/// S/MIME, as a way of protecting a message.
#[derive(Debug, Clone, Copy)]
pub struct Smime;

impl Seal for Smime {
    type Error = SmimeError;
    type Other = crate::pgp::OpenPgp;
    type Own = SmimeCert;
    type Cert = Cert;
    type Signer = Signer;

    fn asked(draft: &Draft) -> Option<Mode> {
        draft.smime
    }

    fn own(
        store: &SqliteStore,
        address: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<SmimeCert>, SmimeError> {
        own_cert(store, address, now)
    }

    fn recipient(
        store: &SqliteStore,
        address: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<Cert>, SmimeError> {
        Ok(cert_for(store, address, now)?.map(|(_, cert)| cert))
    }

    /// Nothing is asked: the identity's secret has no passphrase of its own here.
    fn signer(
        store: &SqliteStore,
        secrets: &dyn SigningStore,
        own: &SmimeCert,
        _ask: Ask<'_>,
    ) -> Result<Signer, SmimeError> {
        identity_of(store, secrets, own)
    }

    fn seal(
        frozen: &[u8],
        mode: Mode,
        signer: Option<&Signer>,
        to: Vec<(String, Cert)>,
        own: &SmimeCert,
        now: DateTime<Utc>,
    ) -> Result<Vec<u8>, SmimeError> {
        let mut recipients: Vec<Cert> = Vec::new();
        if mode.encrypts() {
            recipients.extend(to.into_iter().map(|(_, cert)| cert));
            recipients.push(Cert::from_der(&own.der)?);
        }
        let sealing = Sealing {
            mode,
            signer,
            recipients: &recipients,
            now,
            boundary_prefix: "mailo",
        };
        Ok(smime::seal(frozen, &sealing, &mut rand::rngs::OsRng)?)
    }

    fn both() -> SmimeError {
        SmimeError::BothProtections
    }

    fn no_own(address: String) -> SmimeError {
        SmimeError::NoOwnCert(address)
    }

    fn blind(addresses: Vec<String>) -> SmimeError {
        SmimeError::BlindRecipients(addresses)
    }

    fn missing(addresses: Vec<String>) -> SmimeError {
        SmimeError::NoCertFor(addresses)
    }

    /// Only RSA certificates for mail can be encrypted to, and the sent copy is encrypted to it.
    fn own_encrypts(own: &SmimeCert, address: &str) -> Result<(), SmimeError> {
        if Cert::from_der(&own.der).is_ok_and(|c| c.can_encrypt()) {
            Ok(())
        } else {
            Err(SmimeError::OwnCertCannotEncrypt(address.to_owned()))
        }
    }
}
