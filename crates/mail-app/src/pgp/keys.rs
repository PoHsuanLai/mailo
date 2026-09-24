//! Key management: make a key for an identity, import and export keys, forget one, mark one
//! verified — and find the key to use for an address.
//!
//! Public keys go to the store, secret halves to the OS keyring through `mail_runtime::pgp`.
//! Nothing here writes a secret key anywhere else.

use super::{PgpError, epoch_changed};
use chrono::{DateTime, Utc};
use mail_domain::*;
use mail_mime::openpgp::{self, Cert, Protection, ReadKey, SecretCert};
use mail_runtime::Secrets;
use mail_store::{SqliteStore, Store};

/// An identity of the user's, found by its address on any account.
pub(crate) fn identity_for(store: &SqliteStore, address: &str) -> Option<Identity> {
    let db = store.connection();
    let (id, account): (String, String) = db
        .query_row(
            "SELECT id, account FROM identities WHERE lower(from_email) = lower(?1)
             ORDER BY is_default DESC, id LIMIT 1",
            [address.trim()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .ok()?;
    let account = AccountId::from_uuid(account.parse().ok()?);
    let id = IdentityId::from_uuid(id.parse().ok()?);
    drop(db);
    crate::compose::identity_of(store, account, Some(id)).ok()
}

/// The user's own key for `address`: one whose secret half the keyring holds, best first.
pub fn own_key(store: &SqliteStore, address: &str) -> Result<Option<PgpKey>, PgpError> {
    Ok(store
        .pgp_keys_for(address)?
        .into_iter()
        .find(|k| k.secret == SecretHeld::Held))
}

/// The key to encrypt to for `address` at `now`: the best of its keys that can be encrypted to.
pub fn key_for(
    store: &SqliteStore,
    address: &str,
    now: DateTime<Utc>,
) -> Result<Option<(PgpKey, Cert)>, PgpError> {
    for key in store.pgp_keys_for(address)? {
        let Ok(cert) = Cert::from_bytes(&key.key) else {
            continue;
        };
        if cert.can_encrypt(now) {
            return Ok(Some((key, cert)));
        }
    }
    Ok(None)
}

/// Make a key for the identity with address `address`, keep its secret half in the keyring and
/// its public half in the store.
///
/// Refused when the identity already has a key of its own: a second one would silently become
/// the one used, and mail encrypted to the first would still need the first.
pub fn generate(
    store: &SqliteStore,
    secrets: &dyn Secrets,
    address: &str,
    now: DateTime<Utc>,
) -> Result<PgpKey, PgpError> {
    let identity =
        identity_for(store, address).ok_or_else(|| PgpError::NoIdentity(address.to_owned()))?;
    if let Some(held) = own_key(store, &identity.from.email)? {
        return Err(PgpError::AlreadyHasKey {
            address: identity.from.email.clone(),
            fingerprint: held.fingerprint,
        });
    }
    let user_id = match identity.from.name.as_deref().map(str::trim) {
        Some(name) if !name.is_empty() => format!("{name} <{}>", identity.from.email),
        _ => format!("<{}>", identity.from.email),
    };
    let secret = openpgp::generate(&user_id, now, &mut rand::rngs::OsRng)?;
    mail_runtime::pgp::keep_secret_key(secrets, identity.account, &secret)?;
    let mut record = secret
        .public()
        .record(KeySource::Generated, now, &[&identity.from.email]);
    record.secret = SecretHeld::Held;
    let kept = store.put_pgp_key(record)?;
    epoch_changed();
    Ok(kept)
}

/// What importing a file did, key by key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Imported {
    pub key: PgpKey,
    /// Whether its secret half was imported too, and how it is protected.
    pub secret: Option<Protection>,
}

/// Import every key in `bytes` — an armored block, several, or a binary keyring.
///
/// Public keys go to the store as [`KeySource::Imported`]. A secret key goes to the keyring as it
/// is, passphrase and all, when one of the user's identities has an address on it; one that is
/// nobody's here is refused rather than kept, since there is no identity it could sign for.
pub fn import(
    store: &SqliteStore,
    secrets: &dyn Secrets,
    bytes: &[u8],
    now: DateTime<Utc>,
) -> Result<Vec<Imported>, PgpError> {
    let mut out = Vec::new();
    for key in openpgp::read_keys(bytes)? {
        match key {
            ReadKey::Public(cert) => {
                let kept = store.put_pgp_key(cert.record(KeySource::Imported, now, &[]))?;
                out.push(Imported {
                    key: kept,
                    secret: None,
                });
            }
            ReadKey::Secret(secret) => out.push(import_secret(store, secrets, &secret, now)?),
        }
    }
    epoch_changed();
    Ok(out)
}

fn import_secret(
    store: &SqliteStore,
    secrets: &dyn Secrets,
    secret: &SecretCert,
    now: DateTime<Utc>,
) -> Result<Imported, PgpError> {
    let public = secret.public();
    let identity = public
        .emails()
        .iter()
        .find_map(|email| identity_for(store, email))
        .ok_or_else(|| PgpError::NotYours {
            fingerprint: secret.fingerprint(),
            addresses: public.emails(),
        })?;
    mail_runtime::pgp::keep_secret_key(secrets, identity.account, secret)?;
    let mut record = public.record(KeySource::Imported, now, &[]);
    record.secret = SecretHeld::Held;
    Ok(Imported {
        key: store.put_pgp_key(record)?,
        secret: Some(secret.protection()),
    })
}

/// A key named by its fingerprint, or by an address it is for.
pub fn find(store: &SqliteStore, named: &str) -> Result<PgpKey, PgpError> {
    if let Ok(fingerprint) = named.parse::<Fingerprint>() {
        return store
            .pgp_key(fingerprint)?
            .ok_or_else(|| PgpError::NoKey(named.to_owned()));
    }
    store
        .pgp_keys_for(named)?
        .into_iter()
        .next()
        .ok_or_else(|| PgpError::NoKey(named.to_owned()))
}

/// The public key, armored.
pub fn export_public(key: &PgpKey) -> Result<String, PgpError> {
    Ok(Cert::from_bytes(&key.key)?.armored()?)
}

/// The secret key, armored, from the keyring. Only for a key whose secret half is held.
pub fn export_secret(
    store: &SqliteStore,
    secrets: &dyn Secrets,
    key: &PgpKey,
) -> Result<String, PgpError> {
    if key.secret != SecretHeld::Held {
        return Err(PgpError::NoSecret(key.fingerprint));
    }
    let account = account_of(store, key);
    let secret = mail_runtime::pgp::secret_key(secrets, account, key.fingerprint)?;
    Ok(secret.armored()?)
}

/// Whether deleting a key with a secret half was confirmed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WithSecret {
    /// Delete only a key the keyring holds no secret for.
    Refuse,
    /// `--with-secret`: the secret half goes too.
    Confirmed,
}

/// Forget a key: its row, and — when confirmed — its secret half in the keyring.
pub fn delete(
    store: &SqliteStore,
    secrets: &dyn Secrets,
    key: &PgpKey,
    with_secret: WithSecret,
) -> Result<(), PgpError> {
    if key.secret == SecretHeld::Held {
        if with_secret == WithSecret::Refuse {
            return Err(PgpError::SecretWouldBeLost(key.fingerprint));
        }
        mail_runtime::pgp::forget_secret_key(secrets, account_of(store, key), key.fingerprint)?;
    }
    store.delete_pgp_key(key.fingerprint)?;
    epoch_changed();
    Ok(())
}

/// Mark a key as verified by the user.
pub fn verify(store: &SqliteStore, fingerprint: Fingerprint) -> Result<(), PgpError> {
    store.set_pgp_trust(fingerprint, KeyTrust::Verified)?;
    epoch_changed();
    Ok(())
}

/// The account a key's secret was kept for: that of an identity with an address on it.
///
/// Only a label — the keyring entry is named by the fingerprint alone — so a key that is no
/// identity's any more still finds its secret under the nil id.
fn account_of(store: &SqliteStore, key: &PgpKey) -> AccountId {
    key.emails
        .iter()
        .find_map(|email| identity_for(store, email))
        .map_or(AccountId::from_uuid(uuid::Uuid::nil()), |i| i.account)
}

/// The secret key to sign with or decrypt by, with its passphrase when it has one.
///
/// `ask` is asked only for a protected key, and only once.
pub(crate) fn unlocking(
    store: &SqliteStore,
    secrets: &dyn Secrets,
    key: &PgpKey,
    ask: super::Ask<'_>,
) -> Result<openpgp::Unlocking, PgpError> {
    let secret = mail_runtime::pgp::secret_key(secrets, account_of(store, key), key.fingerprint)?;
    let passphrase = match secret.protection() {
        Protection::Open => String::new(),
        Protection::Passphrase => ask(key.fingerprint).unwrap_or_default(),
    };
    Ok(openpgp::Unlocking {
        key: secret,
        passphrase,
    })
}

/// `fingerprint` in groups of four, as people compare them aloud.
pub fn grouped(fingerprint: Fingerprint) -> String {
    let hex = fingerprint.to_string();
    hex.as_bytes()
        .chunks(4)
        .map(|c| String::from_utf8_lossy(c).into_owned())
        .collect::<Vec<_>>()
        .join(" ")
}
