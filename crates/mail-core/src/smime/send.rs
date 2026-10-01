//! Sending: an outgoing message made what its draft asks — signed, encrypted, both.
//!
//! Done when the bytes are frozen, as Send is pressed, for the reasons `crate::pgp::send` gives.
//! Encryption goes to every `To` and `Cc` recipient's certificate and to the sender's own, so the
//! copy the server files in Sent can be read. A recipient with no certificate refuses the send,
//! naming them: nothing is ever sent in the clear because a certificate was missing.
//!
//! `Bcc` with encryption is refused, for OpenPGP's reason: an enveloped message names every
//! certificate it is encrypted to by issuer and serial number, so each recipient would learn the
//! blind ones'.
//!
//! A draft asks for OpenPGP or S/MIME, not both: the two wrap the same content in two different
//! envelopes, and one inside the other opens for nobody. [`check`] refuses it before anything is
//! built.

use super::SmimeError;
use super::certs::{cert_for, identity_of, own_cert};
use chrono::{DateTime, Utc};
use mail_domain::*;
use mail_mime::smime::{self, Cert, Sealing};
use mail_runtime::Secrets;
use mail_store::SqliteStore;

/// What stands between `draft` and sending it as it asks, without touching the keyring: the
/// composer asks this to warn before Send, and [`outgoing`] asks it again as the send happens.
pub fn check(
    store: &SqliteStore,
    draft: &Draft,
    identity: &Identity,
    now: DateTime<Utc>,
) -> Result<(), SmimeError> {
    if draft.smime == Smime::None {
        return Ok(());
    }
    if draft.openpgp != OpenPgp::None {
        return Err(SmimeError::BothProtections);
    }
    let Some(own) = own_cert(store, &identity.from.email, now)? else {
        return Err(SmimeError::NoOwnCert(identity.from.email.clone()));
    };
    if !draft.smime.encrypts() {
        return Ok(());
    }
    if !draft.bcc.is_empty() {
        return Err(SmimeError::BlindRecipients(
            draft.bcc.iter().map(|a| a.email.clone()).collect(),
        ));
    }
    if !Cert::from_der(&own.der).is_ok_and(|c| c.can_encrypt()) {
        return Err(SmimeError::OwnCertCannotEncrypt(
            identity.from.email.clone(),
        ));
    }
    let missing: Vec<String> = recipients(draft)
        .into_iter()
        .filter(|address| !matches!(cert_for(store, address, now), Ok(Some(_))))
        .collect();
    if !missing.is_empty() {
        return Err(SmimeError::NoCertFor(missing));
    }
    Ok(())
}

/// `frozen` — the bytes built for the wire — signed and encrypted as the draft asks.
pub fn outgoing(
    store: &SqliteStore,
    secrets: &dyn Secrets,
    draft: &Draft,
    identity: &Identity,
    frozen: Vec<u8>,
    now: DateTime<Utc>,
) -> Result<Vec<u8>, SmimeError> {
    if draft.smime == Smime::None {
        return Ok(frozen);
    }
    check(store, draft, identity, now)?;
    let Some(own) = own_cert(store, &identity.from.email, now)? else {
        return Err(SmimeError::NoOwnCert(identity.from.email.clone()));
    };
    let signer = if draft.smime.signs() {
        Some(identity_of(store, secrets, &own)?)
    } else {
        None
    };
    let mut to: Vec<Cert> = Vec::new();
    if draft.smime.encrypts() {
        for address in recipients(draft) {
            let Some((_, cert)) = cert_for(store, &address, now)? else {
                return Err(SmimeError::NoCertFor(vec![address]));
            };
            to.push(cert);
        }
        to.push(Cert::from_der(&own.der)?);
    }
    let sealing = Sealing {
        mode: draft.smime,
        signer: signer.as_ref(),
        recipients: &to,
        now,
    };
    Ok(smime::seal(&frozen, &sealing, &mut rand::rngs::OsRng)?)
}

/// The addresses the message is encrypted to: `To` and `Cc`, each once, lower-cased.
fn recipients(draft: &Draft) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for address in draft.to.iter().chain(&draft.cc) {
        let email = address.email.trim().to_ascii_lowercase();
        if !email.is_empty() && !out.contains(&email) {
            out.push(email);
        }
    }
    out
}
