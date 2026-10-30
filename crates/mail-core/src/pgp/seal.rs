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
use super::{Ask, PgpError, Protected, open_message};
use crate::message::{Protection, Scheme};
use crate::protect::Seal;
use chrono::{DateTime, Utc};
use mail_crypto::openpgp::{self, Cert, Sealing, Unlocking};
use mail_domain::*;
use mail_mime::Parsed;
use mail_secrets::SigningStore;
use mail_store::SqliteStore;

/// OpenPGP, as a way of protecting a message.
#[derive(Debug, Clone, Copy)]
pub struct OpenPgp;

impl Seal for OpenPgp {
    type Error = PgpError;
    type Other = crate::smime::Smime;
    type Own = PgpKey;
    type Cert = Cert;
    type Signer = Unlocking;
    type Protected = Protected;

    const SCHEME: Scheme = Scheme::OpenPgp;

    fn asked(draft: &Draft) -> Option<Mode> {
        draft.openpgp
    }

    fn own(
        store: &SqliteStore,
        address: &str,
        _now: DateTime<Utc>,
    ) -> Result<Option<PgpKey>, PgpError> {
        own_key(store, address)
    }

    fn recipient(
        store: &SqliteStore,
        address: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<Cert>, PgpError> {
        Ok(key_for(store, address, now)?.map(|(_, cert)| cert))
    }

    fn signer(
        store: &SqliteStore,
        secrets: &dyn SigningStore,
        own: &PgpKey,
        ask: Ask<'_>,
    ) -> Result<Unlocking, PgpError> {
        unlocking(store, secrets, own, ask)
    }

    /// With the recipients' keys gossiped inside the encrypted part, so a reply-all can be
    /// encrypted too. With one recipient there is nobody to tell.
    fn seal(
        frozen: &[u8],
        mode: Mode,
        signer: Option<&Unlocking>,
        to: Vec<(String, Cert)>,
        own: &PgpKey,
        now: DateTime<Utc>,
    ) -> Result<Vec<u8>, PgpError> {
        let mut recipients: Vec<Cert> = Vec::new();
        let mut gossip: Vec<(String, Cert)> = Vec::new();
        if mode.encrypts() {
            for (address, cert) in to {
                recipients.push(cert.clone());
                gossip.push((address, cert));
            }
            recipients.push(Cert::from_bytes(&own.key)?);
        }
        if gossip.len() < 2 {
            gossip.clear();
        }
        let sealing = Sealing {
            mode,
            signer,
            recipients: &recipients,
            gossip: &gossip,
            now,
            boundary_prefix: "mailo",
        };
        openpgp::seal(frozen, &sealing, &mut rand::rngs::OsRng).map_err(PgpError::from)
    }

    fn open(
        store: &SqliteStore,
        secrets: &dyn SigningStore,
        message: &Message,
        ask: Ask<'_>,
        now: DateTime<Utc>,
    ) -> Result<Option<Protected>, PgpError> {
        open_message(store, secrets, message, ask, now)
    }

    fn wrap(protected: Protected) -> Protection {
        Protection::OpenPgp(Box::new(protected))
    }

    fn parts(protected: &mut Protected) -> (&mut Option<Parsed>, &mut Option<Vec<u8>>) {
        (&mut protected.shown, &mut protected.raw)
    }

    fn locked(protected: &Protected) -> Option<Fingerprint> {
        match protected.encryption {
            Encryption::Locked { key } => Some(key),
            _ => None,
        }
    }

    fn both() -> PgpError {
        PgpError::BothProtections
    }

    fn no_own(address: String) -> PgpError {
        PgpError::NoOwnKey(address)
    }

    fn blind(addresses: Vec<String>) -> PgpError {
        PgpError::BlindRecipients(addresses)
    }

    fn missing(addresses: Vec<String>) -> PgpError {
        PgpError::NoKeyFor(addresses)
    }

    /// The sender's Autocrypt header added when the identity has a key, whatever the draft asks.
    fn prepare(
        store: &SqliteStore,
        identity: &Identity,
        frozen: Vec<u8>,
    ) -> Result<Vec<u8>, PgpError> {
        let Some(own) = own_key(store, &identity.from.email)? else {
            return Ok(frozen);
        };
        let cert = Cert::from_bytes(&own.key)?;
        // `prefer-encrypt` is left unsaid: it is the user's preference to state, and nothing
        // here has asked them.
        Ok(openpgp::with_field(
            &frozen,
            &openpgp::autocrypt_field(
                "Autocrypt",
                &identity.from.email,
                PreferEncrypt::NoPreference,
                &cert,
            ),
        ))
    }
}
