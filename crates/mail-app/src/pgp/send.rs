//! Sending: an outgoing message made what its draft asks — signed, encrypted, both — and
//! carrying the sender's Autocrypt header.
//!
//! Done when the bytes are frozen, as Send is pressed (`compose::queue`), not when the outbox
//! later hands them to the server: that is when the user is there to be asked for a passphrase
//! or told that a recipient has no key, and the signature covers only the content, so the
//! `Date` the outbox rewrites as the message leaves does not break it.
//!
//! Encryption goes to every `To` and `Cc` recipient's key and to the sender's own, so the copy
//! the server files in Sent can be read. A recipient with no key refuses the send, naming them:
//! nothing is ever sent in the clear because a key was missing.
//!
//! `Bcc` with encryption is refused. An encrypted message names the key of every recipient it is
//! encrypted to, so a blind recipient's key id would be in the copy every other recipient gets —
//! the one thing `Bcc` promises they will not learn. Encrypting a separate copy per blind
//! recipient would keep the promise, but the outbox holds one submission per draft and a partial
//! failure there would deliver to some and not others with nothing to show it; anonymous
//! recipient ids would hide who but still show how many. Refusing is the only choice that
//! cannot leak; the user can send the blind copy as a message of its own.

use super::keys::{key_for, own_key, unlocking};
use super::{Ask, PgpError};
use chrono::{DateTime, Utc};
use mail_domain::*;
use mail_mime::openpgp::{self, Cert, Sealing};
use mail_runtime::Secrets;
use mail_store::SqliteStore;

/// What stands between `draft` and sending it as it asks, without touching the keyring: the
/// composer asks this to warn before Send, and [`outgoing`] asks it again as the send happens.
pub fn check(
    store: &SqliteStore,
    draft: &Draft,
    identity: &Identity,
    now: DateTime<Utc>,
) -> Result<(), PgpError> {
    if draft.openpgp == OpenPgp::None {
        return Ok(());
    }
    if own_key(store, &identity.from.email)?.is_none() {
        return Err(PgpError::NoOwnKey(identity.from.email.clone()));
    }
    if !draft.openpgp.encrypts() {
        return Ok(());
    }
    if !draft.bcc.is_empty() {
        return Err(PgpError::BlindRecipients(
            draft.bcc.iter().map(|a| a.email.clone()).collect(),
        ));
    }
    let missing: Vec<String> = recipients(draft)
        .into_iter()
        .filter(|address| !matches!(key_for(store, address, now), Ok(Some(_))))
        .collect();
    if !missing.is_empty() {
        return Err(PgpError::NoKeyFor(missing));
    }
    Ok(())
}

/// `frozen` — the bytes built for the wire — with the sender's Autocrypt header added when the
/// identity has a key, then signed and encrypted as the draft asks.
pub fn outgoing(
    store: &SqliteStore,
    secrets: &dyn Secrets,
    ask: Ask<'_>,
    draft: &Draft,
    identity: &Identity,
    frozen: Vec<u8>,
    now: DateTime<Utc>,
) -> Result<Vec<u8>, PgpError> {
    let own = own_key(store, &identity.from.email)?;
    let own_cert = own.as_ref().map(|k| Cert::from_bytes(&k.key)).transpose()?;
    let mut bytes = frozen;
    if let Some(cert) = &own_cert {
        // `prefer-encrypt` is left unsaid: it is the user's preference to state, and nothing
        // here has asked them.
        bytes = openpgp::with_field(
            &bytes,
            &openpgp::autocrypt_field(
                "Autocrypt",
                &identity.from.email,
                PreferEncrypt::NoPreference,
                cert,
            ),
        );
    }
    if draft.openpgp == OpenPgp::None {
        return Ok(bytes);
    }
    check(store, draft, identity, now)?;
    let (Some(own), Some(own_cert)) = (own, own_cert) else {
        return Err(PgpError::NoOwnKey(identity.from.email.clone()));
    };
    let signer = if draft.openpgp.signs() {
        Some(unlocking(store, secrets, &own, ask)?)
    } else {
        None
    };
    let mut to: Vec<Cert> = Vec::new();
    let mut gossip: Vec<(String, Cert)> = Vec::new();
    if draft.openpgp.encrypts() {
        for address in recipients(draft) {
            let Some((_, cert)) = key_for(store, &address, now)? else {
                return Err(PgpError::NoKeyFor(vec![address]));
            };
            gossip.push((address, cert.clone()));
            to.push(cert);
        }
        to.push(own_cert);
    }
    // Gossip tells each recipient the others' keys, so a reply-all can be encrypted too. With one
    // recipient there is nobody to tell.
    if gossip.len() < 2 {
        gossip.clear();
    }
    let sealing = Sealing {
        mode: draft.openpgp,
        signer: signer.as_ref(),
        recipients: &to,
        gossip: &gossip,
        now,
    };
    openpgp::seal(&bytes, &sealing, &mut rand::rngs::OsRng).map_err(PgpError::from)
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
