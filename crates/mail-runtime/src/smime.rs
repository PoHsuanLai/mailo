//! S/MIME's effects: private keys in and out of the OS keyring, the operating system's trust
//! anchors, and correspondents' certificates learnt from their signed mail.
//!
//! The cryptography is `mail_mime::smime`'s and is pure. What happens here is what it cannot do:
//! read and write the keyring, read the system's certificate bundle, and write to the store what
//! arriving mail taught. Nothing here decrypts anything — mail is decrypted when the reader opens
//! it, never as it arrives.

use crate::{RuntimeError, Secrets};
use chrono::{DateTime, Utc};
use mail_domain::{
    AccountId, CertFingerprint, CertProblem, CertSource, Credential, SecretKey, SecretPurpose,
    SmimeVerification,
};
use mail_mime::smime::{self, Cert, Keys, PrivateKey};
use mail_store::Store;
use std::sync::OnceLock;

fn entry(account: AccountId, fingerprint: CertFingerprint) -> SecretKey {
    SecretKey {
        account,
        purpose: SecretPurpose::Smime(fingerprint),
    }
}

/// The private key of the certificate with `fingerprint`, from the keyring.
pub fn private_key(
    secrets: &dyn Secrets,
    account: AccountId,
    fingerprint: CertFingerprint,
) -> Result<PrivateKey, RuntimeError> {
    match secrets.get(&entry(account, fingerprint))? {
        Credential::SmimeKey(pem) => {
            let pem = zeroize::Zeroizing::new(pem);
            Ok(PrivateKey::from_pkcs8_pem(&pem)?)
        }
        Credential::Password(_) | Credential::OAuth { .. } | Credential::OpenPgp(_) => {
            Err(RuntimeError::Secrets(format!(
                "the keyring entry for S/MIME certificate {fingerprint} holds something else"
            )))
        }
    }
}

/// Keep the private key of the certificate with `fingerprint` in the keyring.
pub fn keep_private_key(
    secrets: &dyn Secrets,
    account: AccountId,
    fingerprint: CertFingerprint,
    key: &PrivateKey,
) -> Result<(), RuntimeError> {
    let pem = key.to_pkcs8_pem()?;
    secrets.put(
        &entry(account, fingerprint),
        &Credential::SmimeKey(pem.as_str().to_owned()),
    )
}

/// Remove the private key of `fingerprint` from the keyring. Already gone is success.
pub fn forget_private_key(
    secrets: &dyn Secrets,
    account: AccountId,
    fingerprint: CertFingerprint,
) -> Result<(), RuntimeError> {
    secrets.forget(&entry(account, fingerprint))
}

/// The operating system's certificate authorities, read once per process.
///
/// What `rustls-native-certs` finds: on Linux the bundle OpenSSL would read (or the one
/// `SSL_CERT_FILE` names), the system store on macOS and Windows. The bundle does not say which
/// authorities are trusted for mail as opposed to web servers, so every one is taken as an
/// anchor for both. A certificate that cannot be parsed is skipped; a machine with no bundle has
/// no system anchors, and only certificates the user trusts are believed.
pub fn system_anchors() -> &'static [Cert] {
    static ANCHORS: OnceLock<Vec<Cert>> = OnceLock::new();
    ANCHORS.get_or_init(|| {
        rustls_native_certs::load_native_certs()
            .certs
            .iter()
            .filter_map(|der| Cert::from_der(der.as_ref()).ok())
            .collect()
    })
}

/// Keep the certificate a signed message from `from` was signed with, when its signature holds
/// and the certificate names `from`.
///
/// Whether the certificate is to be believed is not decided here — that is asked each time it
/// is used, against the trust anchors of that moment — but a certificate is kept for an address
/// only when mail from that address proved it holds the key. Reads only what arrives in the
/// clear: a signature inside an encrypted message is learnt when the reader opens it.
pub fn learn_signer(
    store: &dyn Store,
    raw: &[u8],
    from: &str,
    arrived: DateTime<Utc>,
) -> Result<(), RuntimeError> {
    let Some(opened) = smime::open(
        raw,
        &Keys {
            identities: &[],
            certs: &[],
            anchors: &[],
            now: arrived,
        },
    ) else {
        return Ok(());
    };
    keep_signer(store, &opened, from, arrived)
}

/// Keep what an opened message's signature taught, under the rule [`learn_signer`] states.
pub fn keep_signer(
    store: &dyn Store,
    opened: &smime::Opened,
    from: &str,
    seen: DateTime<Utc>,
) -> Result<(), RuntimeError> {
    let proved = match &opened.verification {
        SmimeVerification::Good { .. } => true,
        SmimeVerification::Doubtful { problems, .. } => !problems
            .iter()
            .any(|p| matches!(p, CertProblem::NotFrom { .. })),
        SmimeVerification::NoSignature
        | SmimeVerification::Bad { .. }
        | SmimeVerification::UnknownSigner { .. } => false,
    };
    let Some(signer) = &opened.signer else {
        return Ok(());
    };
    if !proved || !signer.cert.is_for(from) {
        return Ok(());
    }
    let before = store.smime_cert(signer.cert.fingerprint())?;
    let after = store.put_smime_cert(signer.cert.record(
        &signer.chain,
        CertSource::Received,
        seen,
    ))?;
    if before.is_none_or(|b| b.chain != after.chain || b.emails != after.emails) {
        crate::epoch::keys_changed();
    }
    Ok(())
}
