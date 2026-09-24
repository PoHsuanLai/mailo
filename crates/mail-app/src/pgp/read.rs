//! Reading: a message's signature checked and its encryption opened, when the reader opens it.
//!
//! Never at sync time: a sync stores the message as it arrived, and the search index holds what
//! a sync can read — for an encrypted message, nothing of its content. Decrypted content lives
//! in memory only: returned to the caller, and kept in a small cache in this process so opening
//! the same message again neither decrypts again nor asks for a passphrase again. It is never
//! written to the store, the blob directory or the index.

use super::keys::unlocking;
use super::{Ask, PgpError, epoch};
use chrono::{DateTime, Utc};
use mail_domain::*;
use mail_mime::Parsed;
use mail_mime::openpgp::{self, Cert, Keys, KnownCert};
use mail_runtime::Secrets;
use mail_store::{SqliteStore, Store};
use std::sync::Mutex;

/// A message opened for reading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Protected {
    pub encryption: Encryption,
    pub verification: Verification,
    /// The key that made a good signature, as the store holds it: its addresses and trust.
    pub signer: Option<PgpKey>,
    /// The message as the reader shows it — decrypted, protected headers applied, signature
    /// part taken off — parsed the way every message is. `None` when it could not be opened
    /// ([`Encryption::CannotDecrypt`], [`Encryption::Locked`], [`Encryption::Unreadable`]):
    /// the reader shows what it already has.
    pub shown: Option<Parsed>,
    /// The bytes `shown` was parsed from. In memory only; not to be stored.
    pub raw: Option<Vec<u8>>,
}

/// How many opened messages to keep in memory.
const CACHE_SIZE: usize = 32;

/// Opened messages, newest last, with the key epoch they were opened under. Only results worth
/// keeping are here: a message that could not be opened is asked about again next time, when the
/// user may have given a passphrase or imported a key.
static CACHE: Mutex<Vec<(BlobId, u64, Protected)>> = Mutex::new(Vec::new());

fn cached(raw: BlobId) -> Option<Protected> {
    let now = epoch();
    let cache = CACHE.lock().unwrap_or_else(|held| held.into_inner());
    cache
        .iter()
        .find(|(blob, at, _)| *blob == raw && *at == now)
        .map(|(_, _, protected)| protected.clone())
}

fn keep(raw: BlobId, protected: &Protected) {
    let mut cache = CACHE.lock().unwrap_or_else(|held| held.into_inner());
    cache.retain(|(blob, _, _)| *blob != raw);
    cache.push((raw, epoch(), protected.clone()));
    if cache.len() > CACHE_SIZE {
        cache.remove(0);
    }
}

/// Open `message` for reading: `Ok(None)` when it carries no OpenPGP, or its body is not here yet.
///
/// Decrypts with the user's secret keys the message is encrypted to, asking `ask` for a
/// passphrase only if one of those is protected, and checks the signature against every key the
/// store holds for its issuer. Gossip found inside is recorded for the message's recipients.
pub fn open_message(
    store: &SqliteStore,
    secrets: &dyn Secrets,
    message: &Message,
    ask: Ask<'_>,
    now: DateTime<Utc>,
) -> Result<Option<Protected>, PgpError> {
    let Some(raw_id) = message.body.raw() else {
        return Ok(None);
    };
    if let Some(hit) = cached(raw_id) {
        return Ok(Some(hit));
    }
    let raw = store.blobs().get(&store.connection(), raw_id)?;
    let Some((protected, gossip)) = opened(store, secrets, &raw, ask)? else {
        return Ok(None);
    };
    if !gossip.is_empty() {
        let recipients: Vec<String> = message
            .to
            .iter()
            .chain(&message.cc)
            .map(|a| a.email.clone())
            .collect();
        // What gossip teaches is a convenience; failing to record it is no reason not to show
        // the message.
        let _ =
            mail_runtime::pgp::learn_gossip(store, &gossip, &recipients, Some(message.date), now);
    }
    if matches!(
        protected.encryption,
        Encryption::Decrypted | Encryption::NotEncrypted
    ) {
        keep(raw_id, &protected);
    }
    Ok(Some(protected))
}

/// The same, on bytes the caller already holds. Nothing is cached and no gossip is recorded.
pub fn open_bytes(
    store: &SqliteStore,
    secrets: &dyn Secrets,
    raw: &[u8],
    ask: Ask<'_>,
) -> Result<Option<Protected>, PgpError> {
    Ok(opened(store, secrets, raw, ask)?.map(|(protected, _)| protected))
}

/// The message opened, and the gossip found inside it.
fn opened(
    store: &SqliteStore,
    secrets: &dyn Secrets,
    raw: &[u8],
    ask: Ask<'_>,
) -> Result<Option<(Protected, Vec<openpgp::AutocryptHeader>)>, PgpError> {
    let secrets_for = secrets_for(store, secrets, raw, ask)?;
    let mut keys = Keys {
        certs: Vec::new(),
        secrets: secrets_for,
    };
    let Some(mut opened) = openpgp::open(raw, &keys) else {
        return Ok(None);
    };
    // The signer is only known once the message is open, so the certificates are fetched then:
    // by the issuer's key id, from the store, and the message checked again against them.
    if let Verification::UnknownKey { issuer, .. } = &opened.verification {
        let certs = certs_for(store, *issuer)?;
        if !certs.is_empty() {
            keys.certs = certs;
            if let Some(again) = openpgp::open(raw, &keys) {
                opened = again;
            }
        }
    }
    let signer = match &opened.verification {
        Verification::Good { signer, .. } => store.pgp_key(*signer)?,
        _ => None,
    };
    let readable = matches!(
        opened.encryption,
        Encryption::Decrypted | Encryption::NotEncrypted
    );
    let shown = if readable {
        mail_mime::parse(&opened.message).ok()
    } else {
        None
    };
    Ok(Some((
        Protected {
            encryption: opened.encryption,
            verification: opened.verification,
            signer,
            raw: shown.is_some().then_some(opened.message),
            shown,
        },
        opened.gossip,
    )))
}

/// The user's secret keys that `raw` is encrypted to, unlocked as far as `ask` allows.
fn secrets_for(
    store: &SqliteStore,
    secrets: &dyn Secrets,
    raw: &[u8],
    ask: Ask<'_>,
) -> Result<Vec<openpgp::Unlocking>, PgpError> {
    let Some(to) = openpgp::encrypted_to(raw) else {
        return Ok(Vec::new());
    };
    let mut held: Vec<PgpKey> = Vec::new();
    for id in &to {
        for key in store.pgp_keys_by_id(*id)? {
            if key.secret == SecretHeld::Held
                && !held.iter().any(|k| k.fingerprint == key.fingerprint)
            {
                held.push(key);
            }
        }
    }
    // A message to an anonymous recipient names no key; every key of the user's is a candidate.
    if to.iter().any(KeyId::is_wildcard) {
        for key in store.pgp_keys()? {
            if key.secret == SecretHeld::Held
                && !held.iter().any(|k| k.fingerprint == key.fingerprint)
            {
                held.push(key);
            }
        }
    }
    held.iter()
        .map(|key| unlocking(store, secrets, key, ask))
        .collect()
}

fn certs_for(store: &SqliteStore, issuer: KeyId) -> Result<Vec<KnownCert>, PgpError> {
    Ok(store
        .pgp_keys_by_id(issuer)?
        .into_iter()
        .filter_map(|key| {
            Cert::from_bytes(&key.key).ok().map(|cert| KnownCert {
                cert,
                trust: key.trust,
            })
        })
        .collect())
}

/// What `mailo show` says about a protected message, one line each, before its text.
pub fn describe(protected: &Protected) -> String {
    let mut out = String::new();
    match &protected.encryption {
        Encryption::NotEncrypted => {}
        Encryption::Decrypted => out.push_str("    encrypted; decrypted for reading\n"),
        Encryption::CannotDecrypt { to } => {
            let ids: Vec<String> = to.iter().map(ToString::to_string).collect();
            out.push_str(&format!(
                "    encrypted to keys you do not hold ({}); it cannot be read here\n",
                ids.join(", ")
            ));
        }
        Encryption::Locked { key } => out.push_str(&format!(
            "    encrypted to your key {key}, which needs its passphrase \
             (set MAILO_PGP_PASSPHRASE, or run from a terminal to be asked)\n"
        )),
        Encryption::Unreadable { why } => {
            out.push_str(&format!("    encrypted, and could not be read: {why}\n"));
        }
    }
    let part = |coverage: &Coverage| match coverage {
        Coverage::Whole => "",
        Coverage::Part => " — only part of the message is signed; the rest could say anything",
    };
    match &protected.verification {
        Verification::NoSignature => {}
        Verification::Good {
            signer,
            trust,
            coverage,
        } => {
            let who = protected
                .signer
                .as_ref()
                .and_then(|k| k.emails.first().cloned())
                .unwrap_or_default();
            let trust = match trust {
                KeyTrust::Verified => "verified",
                KeyTrust::Unverified => "not verified by you",
            };
            out.push_str(&format!(
                "    good signature by {who} ({signer}, {trust}){}\n",
                part(coverage)
            ));
        }
        Verification::Bad { coverage } => out.push_str(&format!(
            "    BAD SIGNATURE: the message was changed after it was signed, or the signature is \
             forged{}\n",
            part(coverage)
        )),
        Verification::UnknownKey { issuer, coverage } => out.push_str(&format!(
            "    signed by key {issuer}, which you do not have; `mailo pgp lookup <address>` or \
             `mailo pgp import <file>` to check it{}\n",
            part(coverage)
        )),
    }
    out
}
