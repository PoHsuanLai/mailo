//! Reading: a message's S/MIME signature checked and its encryption opened, when the reader
//! opens it.
//!
//! As with OpenPGP (`crate::pgp::read`): never at sync time for encrypted mail, and decrypted
//! content lives in memory only — returned to the caller and kept in a small cache in this
//! process, never written to the store, the blob directory or the index. What a signature
//! teaches — the signer's certificate — is kept, under the rule `mail_runtime::smime` states.

use super::certs::identity_of;
use super::{SmimeError, epoch};
use chrono::{DateTime, Utc};
use mail_domain::*;
use mail_mime::Parsed;
use mail_mime::smime::{self, Cert, Identity, Keys};
use mail_runtime::Secrets;
use mail_store::{SqliteStore, Store};
use std::sync::Mutex;

/// A message opened for reading. The same shape as [`crate::pgp::Protected`], so one badge can
/// show either.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Protected {
    pub encryption: SmimeEncryption,
    pub verification: SmimeVerification,
    /// The certificate that made the signature, as the store holds it (or would, for one it has
    /// not kept): its subject, issuer, dates and addresses. `None` when there was no signature
    /// or no certificate for it.
    pub signer: Option<SmimeCert>,
    /// The message as the reader shows it — decrypted, signature part taken off — parsed the way
    /// every message is. `None` when it could not be opened.
    pub shown: Option<Parsed>,
    /// The bytes `shown` was parsed from. In memory only; not to be stored.
    pub raw: Option<Vec<u8>>,
}

impl Protected {
    /// Attachment `index` of the opened message, numbered as the reader lists its attachments
    /// ([`crate::attach::opened_attachment`]). Refused for a message that could not be opened.
    pub fn attachment(&self, index: usize) -> Result<crate::attach::OpenedAttachment, String> {
        let shown = self
            .shown
            .as_ref()
            .ok_or("that message could not be opened, so its attachments cannot be read")?;
        crate::attach::opened_attachment(shown, index)
    }
}

/// How many opened messages to keep in memory.
const CACHE_SIZE: usize = 32;

/// Opened messages, newest last, with the certificate epoch they were opened under. Only those
/// that opened: one that could not be is asked about again, when an identity may have arrived.
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

/// Open `message` for reading: `Ok(None)` when it carries no S/MIME, or its body is not here
/// yet.
///
/// Decrypts with the user's private keys for the certificates the message is encrypted to, and
/// checks the signature against the system's trust anchors and the certificates the user trusts.
/// The signer's certificate is kept for the sender's address when the signature proves it.
pub fn open_message(
    store: &SqliteStore,
    secrets: &dyn Secrets,
    message: &Message,
    now: DateTime<Utc>,
) -> Result<Option<Protected>, SmimeError> {
    let Some(raw_id) = message.body.raw() else {
        return Ok(None);
    };
    if let Some(hit) = cached(raw_id) {
        return Ok(Some(hit));
    }
    let raw = store.blobs().get(&store.connection(), raw_id)?;
    let Some((protected, opened)) = opened(store, secrets, &raw, now)? else {
        return Ok(None);
    };
    // What a signature teaches is a convenience; failing to record it is no reason not to show
    // the message.
    let _ = mail_runtime::smime::keep_signer(store, &opened, &message.from.email, now);
    if matches!(
        protected.encryption,
        SmimeEncryption::Decrypted | SmimeEncryption::NotEncrypted
    ) {
        keep(raw_id, &protected);
    }
    Ok(Some(protected))
}

/// The same, on bytes the caller already holds. Nothing is cached and nothing is learnt.
pub fn open_bytes(
    store: &SqliteStore,
    secrets: &dyn Secrets,
    raw: &[u8],
    now: DateTime<Utc>,
) -> Result<Option<Protected>, SmimeError> {
    Ok(opened(store, secrets, raw, now)?.map(|(protected, _)| protected))
}

fn opened(
    store: &SqliteStore,
    secrets: &dyn Secrets,
    raw: &[u8],
    now: DateTime<Utc>,
) -> Result<Option<(Protected, smime::Opened)>, SmimeError> {
    let identities = identities_for(store, secrets, raw)?;
    let held = store.smime_certs()?;
    let mut certs: Vec<Cert> = Vec::new();
    let mut anchors: Vec<Cert> = mail_runtime::smime::system_anchors().to_vec();
    for record in &held {
        let Ok(cert) = Cert::from_der(&record.der) else {
            continue;
        };
        if record.trust == KeyTrust::Verified {
            anchors.push(cert.clone());
        }
        certs.push(cert);
        certs.extend(
            record
                .chain
                .iter()
                .filter_map(|der| Cert::from_der(der).ok()),
        );
    }
    let keys = Keys {
        identities: &identities,
        certs: &certs,
        anchors: &anchors,
        now,
    };
    let Some(opened) = smime::open(raw, &keys) else {
        return Ok(None);
    };
    let signer = match &opened.signer {
        Some(signer) => Some(
            store
                .smime_cert(signer.cert.fingerprint())?
                .unwrap_or_else(|| signer.cert.record(&signer.chain, CertSource::Received, now)),
        ),
        None => None,
    };
    let readable = matches!(
        opened.encryption,
        SmimeEncryption::Decrypted | SmimeEncryption::NotEncrypted
    );
    let shown = if readable {
        mail_mime::parse(&opened.message).ok()
    } else {
        None
    };
    Ok(Some((
        Protected {
            encryption: opened.encryption.clone(),
            verification: opened.verification.clone(),
            signer,
            raw: shown.is_some().then(|| opened.message.clone()),
            shown,
        },
        opened,
    )))
}

/// The user's identities `raw` is encrypted to, their private keys read from the keyring. Empty
/// for a message not encrypted, or not to the user; the keyring is not touched then.
fn identities_for(
    store: &SqliteStore,
    secrets: &dyn Secrets,
    raw: &[u8],
) -> Result<Vec<Identity>, SmimeError> {
    let Some(to) = smime::recipients(raw) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for record in store.smime_certs()? {
        if record.secret != SecretHeld::Held {
            continue;
        }
        let Ok(cert) = Cert::from_der(&record.der) else {
            continue;
        };
        if to.iter().any(|id| id.names(&cert)) {
            out.push(identity_of(store, secrets, &record)?);
        }
    }
    Ok(out)
}

/// What `mailo show` says about an S/MIME message, one line each, before its text.
pub fn describe(protected: &Protected) -> String {
    let mut out = String::new();
    match &protected.encryption {
        SmimeEncryption::NotEncrypted => {}
        SmimeEncryption::Decrypted => out.push_str("    S/MIME encrypted; decrypted for reading\n"),
        SmimeEncryption::CannotDecrypt { to } => out.push_str(&format!(
            "    S/MIME encrypted to certificates you hold no key for ({}); it cannot be read \
             here\n",
            to.join("; ")
        )),
        SmimeEncryption::Unreadable { why } => {
            out.push_str(&format!(
                "    S/MIME encrypted, and could not be read: {why}\n"
            ));
        }
    }
    let part = |coverage: &Coverage| match coverage {
        Coverage::Whole => "",
        Coverage::Part => " — only part of the message is signed; the rest could say anything",
    };
    let who = protected
        .signer
        .as_ref()
        .map(|c| {
            c.emails
                .first()
                .cloned()
                .unwrap_or_else(|| c.subject.clone())
        })
        .unwrap_or_default();
    match &protected.verification {
        SmimeVerification::NoSignature => {}
        SmimeVerification::Good { signer, coverage } => out.push_str(&format!(
            "    good S/MIME signature by {who} (certificate {signer}){}\n",
            part(coverage)
        )),
        SmimeVerification::Doubtful {
            signer,
            problems,
            coverage,
        } => {
            let said: Vec<String> = problems.iter().map(problem).collect();
            out.push_str(&format!(
                "    S/MIME signature by {who} (certificate {signer}) matches the message, but \
                 {}{}\n",
                said.join("; "),
                part(coverage)
            ));
        }
        SmimeVerification::Bad { why, coverage } => out.push_str(&format!(
            "    BAD S/MIME SIGNATURE: {}{}\n",
            bad(why),
            part(coverage)
        )),
        SmimeVerification::UnknownSigner { coverage } => out.push_str(&format!(
            "    S/MIME signed by a certificate neither the message nor you hold; it cannot be \
             checked{}\n",
            part(coverage)
        )),
    }
    out
}

fn problem(problem: &CertProblem) -> String {
    match problem {
        CertProblem::Untrusted => "its certificate is not issued by an authority you trust \
                                   (trust it with `mailo smime trust <fingerprint>`)"
            .to_owned(),
        CertProblem::Expired { not_after } => {
            format!(
                "a certificate in it expired on {}",
                not_after.format("%Y-%m-%d")
            )
        }
        CertProblem::NotYetValid { not_before } => format!(
            "a certificate in it was not valid until {}",
            not_before.format("%Y-%m-%d")
        ),
        CertProblem::NotForEmail => "its certificate is not for signing mail".to_owned(),
        CertProblem::NotFrom { from } if from.is_empty() => {
            "the message names no sender to check the certificate against".to_owned()
        }
        CertProblem::NotFrom { from } => {
            format!("its certificate is not for {from}, who the message says it is from")
        }
    }
}

fn bad(why: &BadSignature) -> String {
    match why {
        BadSignature::Altered => "the message was changed after it was signed".to_owned(),
        BadSignature::Forged => {
            "the signature does not match its certificate: forged, or damaged".to_owned()
        }
        BadSignature::Weak { algorithm } => {
            format!("made with {algorithm}, which is too weak to mean anything now")
        }
        BadSignature::Unsupported { algorithm } => {
            format!("made with {algorithm}, which this client does not check")
        }
        BadSignature::Malformed { why } => format!("unreadable: {why}"),
    }
}
