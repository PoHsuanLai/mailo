//! Certificate management: import an identity or a certificate, export one, trust one, forget
//! one — and find the certificate to use for an address.
//!
//! Certificates go to the store, private keys to the OS keyring through `mail_runtime::smime`.
//! Nothing here writes a private key anywhere else; the PKCS#12 file's password is used to open
//! it and kept nowhere.

use super::{SmimeError, epoch_changed};
use crate::pgp::WithSecret;
use crate::pgp::keys::identity_for;
use chrono::{DateTime, Utc};
use mail_domain::*;
use mail_mime::smime::{self, Cert, Identity};
use mail_runtime::Secrets;
use mail_store::{SqliteStore, Store};

/// The user's own current certificate for `address`: one whose private key the keyring holds and
/// that is valid at `now`, best first.
pub fn own_cert(
    store: &SqliteStore,
    address: &str,
    now: DateTime<Utc>,
) -> Result<Option<SmimeCert>, SmimeError> {
    Ok(store
        .smime_certs_for(address)?
        .into_iter()
        .find(|c| c.secret == SecretHeld::Held && c.current_at(now)))
}

/// The certificate to encrypt to for `address` at `now`: the best of its certificates that is
/// current and that mail can be encrypted to.
pub fn cert_for(
    store: &SqliteStore,
    address: &str,
    now: DateTime<Utc>,
) -> Result<Option<(SmimeCert, Cert)>, SmimeError> {
    for record in store.smime_certs_for(address)? {
        let Ok(cert) = Cert::from_der(&record.der) else {
            continue;
        };
        if record.current_at(now) && cert.can_encrypt() {
            return Ok(Some((record, cert)));
        }
    }
    Ok(None)
}

/// What importing a file kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Imported {
    pub cert: SmimeCert,
}

/// Import `bytes`: certificates (PEM or DER), kept as [`CertSource::Imported`]; or else a
/// PKCS#12 identity, opened with the password `password` gives, its certificate kept as the
/// user's own and its private key put in the keyring.
///
/// An identity whose certificate names none of the user's addresses is refused rather than
/// kept: there is no identity it could sign for.
pub fn import(
    store: &SqliteStore,
    secrets: &dyn Secrets,
    bytes: &[u8],
    password: &dyn Fn() -> Option<String>,
    now: DateTime<Utc>,
) -> Result<Vec<Imported>, SmimeError> {
    if let Ok(certs) = smime::read_certs(bytes)
        && !certs.is_empty()
    {
        let mut out = Vec::new();
        for cert in &certs {
            let kept = store.put_smime_cert(cert.record(&[], CertSource::Imported, now))?;
            out.push(Imported { cert: kept });
        }
        epoch_changed();
        return Ok(out);
    }
    let password = zeroize::Zeroizing::new(password().ok_or(SmimeError::NoPassword)?);
    let identity = smime::read_pkcs12(bytes, &password)?;
    Ok(vec![import_identity(store, secrets, &identity, now)?])
}

/// Keep `identity`: its private key in the keyring, its certificate and chain in the store.
pub fn import_identity(
    store: &SqliteStore,
    secrets: &dyn Secrets,
    identity: &Identity,
    now: DateTime<Utc>,
) -> Result<Imported, SmimeError> {
    let emails = identity.cert.emails();
    let owner = emails
        .iter()
        .find_map(|email| identity_for(store, email))
        .ok_or_else(|| SmimeError::NotYours {
            fingerprint: identity.cert.fingerprint(),
            addresses: emails.clone(),
        })?;
    let fingerprint = identity.cert.fingerprint();
    mail_runtime::smime::keep_private_key(secrets, owner.account, fingerprint, &identity.key)?;
    let mut record = identity
        .cert
        .record(&identity.chain, CertSource::Identity, now);
    record.secret = SecretHeld::Held;
    let kept = store.put_smime_cert(record)?;
    // The issuers are certificates too, and a later message signed under them needs them to
    // build its chain even when it does not carry them.
    for issuer in &identity.chain {
        store.put_smime_cert(issuer.record(&[], CertSource::Imported, now))?;
    }
    epoch_changed();
    Ok(Imported { cert: kept })
}

/// A certificate named by its fingerprint, or by an address it is for.
pub fn find(store: &SqliteStore, named: &str) -> Result<SmimeCert, SmimeError> {
    if let Ok(fingerprint) = named.parse::<CertFingerprint>() {
        return store
            .smime_cert(fingerprint)?
            .ok_or_else(|| SmimeError::NoCert(named.to_owned()));
    }
    store
        .smime_certs_for(named)?
        .into_iter()
        .next()
        .ok_or_else(|| SmimeError::NoCert(named.to_owned()))
}

/// The certificate, PEM.
pub fn export(cert: &SmimeCert) -> Result<String, SmimeError> {
    Ok(Cert::from_der(&cert.der)?.pem())
}

/// Trust a certificate as an anchor of the user's own, or take that back.
pub fn trust(
    store: &SqliteStore,
    fingerprint: CertFingerprint,
    trust: KeyTrust,
) -> Result<(), SmimeError> {
    store.set_smime_trust(fingerprint, trust)?;
    epoch_changed();
    Ok(())
}

/// Forget a certificate: its row, and — when confirmed — its private key in the keyring.
pub fn delete(
    store: &SqliteStore,
    secrets: &dyn Secrets,
    cert: &SmimeCert,
    with_secret: WithSecret,
) -> Result<(), SmimeError> {
    if cert.secret == SecretHeld::Held {
        if with_secret == WithSecret::Refuse {
            return Err(SmimeError::SecretWouldBeLost(cert.fingerprint));
        }
        mail_runtime::smime::forget_private_key(
            secrets,
            account_of(store, cert),
            cert.fingerprint,
        )?;
    }
    store.delete_smime_cert(cert.fingerprint)?;
    epoch_changed();
    Ok(())
}

/// The account a private key was kept for: that of an identity with an address on the
/// certificate. Only a label — the keyring entry is named by the fingerprint alone.
fn account_of(store: &SqliteStore, cert: &SmimeCert) -> AccountId {
    cert.emails
        .iter()
        .find_map(|email| identity_for(store, email))
        .map_or(AccountId::from_uuid(uuid::Uuid::nil()), |i| i.account)
}

/// The user's identity for `record`: its private key from the keyring, its certificate and its
/// chain.
pub(crate) fn identity_of(
    store: &SqliteStore,
    secrets: &dyn Secrets,
    record: &SmimeCert,
) -> Result<Identity, SmimeError> {
    let key =
        mail_runtime::smime::private_key(secrets, account_of(store, record), record.fingerprint)?;
    let cert = Cert::from_der(&record.der)?;
    let chain = record
        .chain
        .iter()
        .filter_map(|der| Cert::from_der(der).ok())
        .collect();
    Ok(Identity::new(key, cert, chain)?)
}
