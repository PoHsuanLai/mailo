//! OpenPGP's effects: secret keys in and out of the OS keyring, and keys learnt from mail.
//!
//! The cryptography is `mail_mime::openpgp`'s and is pure. What happens here is what it cannot
//! do: read and write the keyring, and write what arriving mail taught to the store. Nothing
//! here decrypts anything — mail is decrypted when the reader opens it, never as it arrives.

use crate::{RuntimeError, Secrets};
use chrono::{DateTime, Utc};
use mail_domain::autocrypt::{self, Sighting, effective_date};
use mail_domain::{AccountId, Credential, Fingerprint, KeySource, SecretKey, SecretPurpose};
use mail_mime::openpgp::{AutocryptHeader, SecretCert, autocrypt_of};
use mail_store::Store;

fn entry(account: AccountId, fingerprint: Fingerprint) -> SecretKey {
    SecretKey {
        account,
        purpose: SecretPurpose::OpenPgp(fingerprint),
    }
}

/// The secret key with `fingerprint`, from the keyring.
pub fn secret_key(
    secrets: &dyn Secrets,
    account: AccountId,
    fingerprint: Fingerprint,
) -> Result<SecretCert, RuntimeError> {
    match secrets.get(&entry(account, fingerprint))? {
        Credential::OpenPgp(armored) => Ok(SecretCert::from_armored(&armored)?),
        Credential::Password(_) | Credential::OAuth { .. } => Err(RuntimeError::Secrets(format!(
            "the keyring entry for OpenPGP key {fingerprint} holds something else"
        ))),
    }
}

/// Keep `key`'s secret half in the keyring, as it is — passphrase-protected if it was.
pub fn keep_secret_key(
    secrets: &dyn Secrets,
    account: AccountId,
    key: &SecretCert,
) -> Result<(), RuntimeError> {
    secrets.put(
        &entry(account, key.fingerprint()),
        &Credential::OpenPgp(key.armored()?),
    )
}

/// Remove the secret half of `fingerprint` from the keyring. Already gone is success.
pub fn forget_secret_key(
    secrets: &dyn Secrets,
    account: AccountId,
    fingerprint: Fingerprint,
) -> Result<(), RuntimeError> {
    secrets.forget(&entry(account, fingerprint))
}

/// Update the Autocrypt state for `from` from one arriving message, and keep the key its
/// header carried when the rules accept it.
///
/// `date` is the message's `Date`, `arrived` when it arrived; the rules use the earlier. Reads
/// the header only: a message that is not from a known peer and carries no header costs one
/// lookup and writes nothing.
pub fn learn_autocrypt(
    store: &dyn Store,
    raw: &[u8],
    from: &str,
    date: Option<DateTime<Utc>>,
    arrived: DateTime<Utc>,
) -> Result<(), RuntimeError> {
    let from = from.trim();
    if from.is_empty() {
        return Ok(());
    }
    let header = autocrypt_of(raw, from);
    let before = store.autocrypt_peer(from)?;
    if before.is_none() && header.is_none() {
        return Ok(());
    }
    let at = effective_date(date, arrived);
    let seen = Sighting {
        address: from.to_owned(),
        at,
        header: header
            .as_ref()
            .map(|h| (h.key.fingerprint(), h.prefer_encrypt)),
    };
    let after = autocrypt::update(before.clone(), &seen);
    if let (Some(header), Some(peer)) = (&header, &after)
        && peer.autocrypt_timestamp == Some(at)
        && peer.key == Some(header.key.fingerprint())
    {
        store.put_pgp_key(header.key.record(KeySource::Autocrypt, at, &[from]))?;
    }
    if let Some(peer) = after
        && Some(&peer) != before.as_ref()
    {
        store.put_autocrypt_peer(&peer)?;
    }
    Ok(())
}

/// Keep what an encrypted message's `Autocrypt-Gossip` headers said, for the addresses it was
/// sent to. Gossip about anyone else is ignored: a sender may only vouch for its own recipients.
pub fn learn_gossip(
    store: &dyn Store,
    gossip: &[AutocryptHeader],
    recipients: &[String],
    date: Option<DateTime<Utc>>,
    arrived: DateTime<Utc>,
) -> Result<(), RuntimeError> {
    let at = effective_date(date, arrived);
    for header in gossip {
        if !recipients
            .iter()
            .any(|r| r.trim().eq_ignore_ascii_case(&header.addr))
        {
            continue;
        }
        let before = store.autocrypt_peer(&header.addr)?;
        let fingerprint = header.key.fingerprint();
        let after = autocrypt::gossip(before.clone(), &header.addr, fingerprint, at);
        if Some(&after) != before.as_ref() {
            store.put_pgp_key(header.key.record(KeySource::Gossip, at, &[&header.addr]))?;
            store.put_autocrypt_peer(&after)?;
        }
    }
    Ok(())
}
