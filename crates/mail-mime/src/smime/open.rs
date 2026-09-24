//! Reading: a received message decrypted and its signature checked, on demand.
//!
//! Handles `multipart/signed` with a `application/pkcs7-signature` part (RFC 8551 §3.5.3) at the
//! top or nested inside something else, and `application/pkcs7-mime` (§3.3, §3.5.2) holding
//! signed-data, enveloped-data or authEnveloped-data — the `x-` spellings of both included, and
//! the CMS read whether it arrives as DER or as the BER streaming encoders write. What comes back
//! is a whole message again, ready for [`crate::parse`]: the decrypted or signed content under
//! the original header.
//!
//! Algorithms refused when reading (RFC 8551 §2.7, RFC 8550 §6): MD5 anywhere, SHA-1 in a
//! signature made after 2020, RC2 and single DES always, and triple DES in mail dated after 2023.

use super::asn1;
use super::ber::{self, Implicit};
use super::cert::Cert;
use super::decrypt::decrypt;
use super::keys::Identity;
use super::verify::signed_data;
use crate::openpgp::entity::{
    ContentType, Entity, MAX_DEPTH, crlf, field_name, fields, is_content_field,
};
use chrono::{DateTime, TimeZone, Utc};
use cms::content_info::ContentInfo;
use cms::enveloped_data::{EnvelopedData, RecipientIdentifier, RecipientInfo, RecipientInfos};
use der::{Decode, Encode};
use mail_domain::{BadSignature, Coverage, SmimeEncryption, SmimeVerification};

/// What [`open`] may use.
#[derive(Debug, Clone, Copy)]
pub struct Keys<'a> {
    /// The user's identities the message may be encrypted to — those [`recipients`] names.
    pub identities: &'a [Identity],
    /// Certificates the client holds: a signer's the message does not carry, and issuers.
    pub certs: &'a [Cert],
    /// Trust anchors: the operating system's, and certificates the user trusts.
    pub anchors: &'a [Cert],
    /// Now: when a signature states no signing time, its certificate is checked for now.
    pub now: DateTime<Utc>,
}

/// The certificate a signature was made with, and the issuers the message carried beside it:
/// what the client keeps of a correspondent's signed mail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signer {
    pub cert: Cert,
    pub chain: Vec<Cert>,
}

/// A message opened for reading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Opened {
    /// The message as the reader should show it: decrypted, or with the signature taken off,
    /// under the original header. Parse it with [`crate::parse`]. Not to be stored.
    pub message: Vec<u8>,
    pub encryption: SmimeEncryption,
    pub verification: SmimeVerification,
    /// The certificate that made the signature, whatever the verdict on it — `None` when there
    /// was no signature or no certificate for it.
    pub signer: Option<Signer>,
}

/// How an encrypted message names one of the certificates it is encrypted to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecipientId {
    IssuerSerial {
        issuer: x509_cert::name::Name,
        serial: Vec<u8>,
    },
    KeyId(Vec<u8>),
    /// A key-agreement recipient: an elliptic-curve certificate, which this client does not
    /// decrypt for.
    KeyAgreement,
}

impl RecipientId {
    /// Whether this names `cert`.
    pub fn names(&self, cert: &Cert) -> bool {
        let tbs = &cert.inner.tbs_certificate;
        match self {
            RecipientId::IssuerSerial { issuer, serial } => {
                tbs.issuer == *issuer && tbs.serial_number.as_bytes() == serial.as_slice()
            }
            RecipientId::KeyId(id) => cert.subject_key_id().as_deref() == Some(id.as_slice()),
            RecipientId::KeyAgreement => false,
        }
    }

    fn of(rid: &RecipientIdentifier) -> RecipientId {
        match rid {
            RecipientIdentifier::IssuerAndSerialNumber(ias) => RecipientId::IssuerSerial {
                issuer: ias.issuer.clone(),
                serial: ias.serial_number.as_bytes().to_vec(),
            },
            RecipientIdentifier::SubjectKeyIdentifier(ski) => {
                RecipientId::KeyId(ski.0.as_bytes().to_vec())
            }
        }
    }
}

impl std::fmt::Display for RecipientId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let hex = |bytes: &[u8]| -> String { bytes.iter().map(|b| format!("{b:02X}")).collect() };
        match self {
            RecipientId::IssuerSerial { issuer, serial } => {
                write!(f, "serial {} from {issuer}", hex(serial))
            }
            RecipientId::KeyId(id) => write!(f, "key id {}", hex(id)),
            RecipientId::KeyAgreement => f.write_str("an elliptic-curve (key agreement) recipient"),
        }
    }
}

const SIGNATURE_TYPES: [&str; 2] = [
    "application/pkcs7-signature",
    "application/x-pkcs7-signature",
];
const MIME_TYPES: [&str; 2] = ["application/pkcs7-mime", "application/x-pkcs7-mime"];

fn is_signed(kind: &ContentType) -> bool {
    kind.mime == "multipart/signed"
        && kind
            .param("protocol")
            .is_some_and(|p| SIGNATURE_TYPES.iter().any(|t| p.eq_ignore_ascii_case(t)))
}

fn is_pkcs7(kind: &ContentType) -> bool {
    MIME_TYPES.contains(&kind.mime.as_str())
}

/// The certificates an encrypted message is encrypted to, so the caller can fetch only those
/// private keys from the keyring. `None` when the message is not S/MIME encrypted.
pub fn recipients(raw: &[u8]) -> Option<Vec<RecipientId>> {
    let top = Entity::of(raw);
    if !is_pkcs7(&top.content_type()) {
        return None;
    }
    let info = content_info(top)?;
    let infos = match info.content_type {
        asn1::ENVELOPED_DATA => enveloped(&info)?.recip_infos,
        asn1::AUTH_ENVELOPED_DATA => auth_enveloped(&info)?.recip_infos,
        _ => return None,
    };
    Some(ids(&infos))
}

pub(super) fn ids(infos: &RecipientInfos) -> Vec<RecipientId> {
    infos
        .0
        .iter()
        .map(|info| match info {
            RecipientInfo::Ktri(ktri) => RecipientId::of(&ktri.rid),
            _ => RecipientId::KeyAgreement,
        })
        .collect()
}

/// `raw` opened with `keys`: decrypted if it is encrypted to one of `keys.identities`, its
/// signature checked. `None` when it carries no S/MIME at all.
pub fn open(raw: &[u8], keys: &Keys<'_>) -> Option<Opened> {
    let top = Entity::of(raw);
    let kind = top.content_type();
    if !is_signed(&kind) && !is_pkcs7(&kind) && !kind.mime.starts_with("multipart/") {
        return None;
    }
    let (from, date) = sender(top.head);
    let from = from.as_deref();
    if is_signed(&kind) {
        let checked = detached(top, keys, Coverage::Whole, from)?;
        return Some(Opened {
            message: shown(top.head, &checked.content),
            encryption: SmimeEncryption::NotEncrypted,
            verification: checked.verification,
            signer: checked.signer,
        });
    }
    if is_pkcs7(&kind) {
        return Some(opaque(top, keys, from, date));
    }
    if kind.mime.starts_with("multipart/") {
        // A signed part inside something larger — a mailing list's footer beside it — covers
        // only part of what the reader shows.
        let checked = nested(top, keys, from, 0)?;
        return Some(Opened {
            message: raw.to_vec(),
            encryption: SmimeEncryption::NotEncrypted,
            verification: checked.verification,
            signer: checked.signer,
        });
    }
    None
}

/// The sender's address and the message's date, from its header.
fn sender(head: &[u8]) -> (Option<String>, Option<DateTime<Utc>>) {
    let mut block = head.to_vec();
    block.extend_from_slice(b"\r\n");
    let Some(parsed) = mail_parser::MessageParser::default().parse_headers(&block) else {
        return (None, None);
    };
    let from = parsed
        .from()
        .and_then(|a| a.first())
        .and_then(|a| a.address())
        .map(|a| a.trim().to_ascii_lowercase());
    let date = parsed
        .date()
        .and_then(|d| Utc.timestamp_opt(d.to_timestamp(), 0).single());
    (from, date)
}

/// A signature checked: the content it covers, as the reader should show it, and the verdict.
pub(super) struct Checked {
    pub content: Vec<u8>,
    pub verification: SmimeVerification,
    pub signer: Option<Signer>,
}

/// The CMS object in an `application/pkcs7-mime` or `-signature` part.
fn content_info(entity: Entity<'_>) -> Option<ContentInfo> {
    let bytes = crate::reconstruct::decode_part(entity.raw, b"");
    ber::decode::<ContentInfo>(&bytes, Implicit::Keep)
}

pub(super) fn inner<T: for<'a> Decode<'a>>(info: &ContentInfo, implicit: Implicit) -> Option<T> {
    ber::decode::<T>(&info.content.to_der().ok()?, implicit)
}

pub(super) fn enveloped(info: &ContentInfo) -> Option<EnvelopedData> {
    inner(info, Implicit::Keep).or_else(|| inner(info, Implicit::Join))
}

pub(super) fn auth_enveloped(info: &ContentInfo) -> Option<asn1::AuthEnvelopedData> {
    inner(info, Implicit::Keep).or_else(|| inner(info, Implicit::Join))
}

/// An `application/pkcs7-mime` part: signed-data opened and checked, or enveloped data
/// decrypted and whatever is inside it read in turn.
fn opaque(
    top: Entity<'_>,
    keys: &Keys<'_>,
    from: Option<&str>,
    date: Option<DateTime<Utc>>,
) -> Opened {
    let unreadable = |why: &str| Opened {
        message: top.raw.to_vec(),
        encryption: SmimeEncryption::Unreadable {
            why: why.to_owned(),
        },
        verification: SmimeVerification::NoSignature,
        signer: None,
    };
    let Some(info) = content_info(top) else {
        return unreadable("the S/MIME part could not be read");
    };
    match info.content_type {
        asn1::SIGNED_DATA => {
            let checked = signed_data(&info, None, keys, Coverage::Whole, from);
            Opened {
                message: shown(top.head, &checked.content),
                encryption: SmimeEncryption::NotEncrypted,
                verification: checked.verification,
                signer: checked.signer,
            }
        }
        asn1::ENVELOPED_DATA | asn1::AUTH_ENVELOPED_DATA => {
            match decrypt(&info, keys, date) {
                Err(encryption) => Opened {
                    message: top.raw.to_vec(),
                    encryption,
                    verification: SmimeVerification::NoSignature,
                    signer: None,
                },
                Ok(plain) => {
                    // Signed, then encrypted (RFC 8551 §3.6): the signature is on what is inside.
                    let checked = within(&plain, keys, from, date, 0);
                    Opened {
                        message: shown(top.head, &checked.content),
                        encryption: SmimeEncryption::Decrypted,
                        verification: checked.verification,
                        signer: checked.signer,
                    }
                }
            }
        }
        other => unreadable(&format!(
            "an S/MIME part holding {} is not read here",
            asn1::name(&other)
        )),
    }
}

/// The decrypted content of an encrypted message, read for a signature inside it.
fn within(
    plain: &[u8],
    keys: &Keys<'_>,
    from: Option<&str>,
    date: Option<DateTime<Utc>>,
    depth: usize,
) -> Checked {
    let entity = Entity::of(plain);
    let kind = entity.content_type();
    let unsigned = || Checked {
        content: plain.to_vec(),
        verification: SmimeVerification::NoSignature,
        signer: None,
    };
    if depth > MAX_DEPTH {
        return unsigned();
    }
    if is_signed(&kind) {
        return detached(entity, keys, Coverage::Whole, from).unwrap_or_else(unsigned);
    }
    if is_pkcs7(&kind) {
        let Some(info) = content_info(entity) else {
            return unsigned();
        };
        if info.content_type == asn1::SIGNED_DATA {
            return signed_data(&info, None, keys, Coverage::Whole, from);
        }
        // Encrypted twice over: opened again, for the same recipient.
        if matches!(
            info.content_type,
            asn1::ENVELOPED_DATA | asn1::AUTH_ENVELOPED_DATA
        ) && let Ok(again) = decrypt(&info, keys, date)
        {
            return within(&again, keys, from, date, depth + 1);
        }
    }
    unsigned()
}

/// A `multipart/signed` entity checked: its first part against the signature in its second.
fn detached(
    entity: Entity<'_>,
    keys: &Keys<'_>,
    coverage: Coverage,
    from: Option<&str>,
) -> Option<Checked> {
    let children = entity.children();
    let (signed, signature) = (children.first()?, children.get(1)?);
    // RFC 8551 §3.1.1: the signature is over the canonical form, CRLF line ends and nothing else
    // changed — trailing whitespace included.
    let data = crlf(signed.raw);
    let Some(info) = content_info(*signature).filter(|i| i.content_type == asn1::SIGNED_DATA)
    else {
        return Some(Checked {
            content: signed.raw.to_vec(),
            verification: SmimeVerification::Bad {
                why: BadSignature::Malformed {
                    why: "the signature part holds no signature".to_owned(),
                },
                coverage,
            },
            signer: None,
        });
    };
    let mut checked = signed_data(&info, Some(&data), keys, coverage, from);
    checked.content = signed.raw.to_vec();
    Some(checked)
}

/// The first S/MIME-signed part anywhere below `entity`, checked, as [`Coverage::Part`].
fn nested(
    entity: Entity<'_>,
    keys: &Keys<'_>,
    from: Option<&str>,
    depth: usize,
) -> Option<Checked> {
    if depth > MAX_DEPTH {
        return None;
    }
    entity.children().into_iter().find_map(|child| {
        let kind = child.content_type();
        if is_signed(&kind) {
            detached(child, keys, Coverage::Part, from)
        } else if is_pkcs7(&kind) {
            let info = content_info(child)?;
            (info.content_type == asn1::SIGNED_DATA)
                .then(|| signed_data(&info, None, keys, Coverage::Part, from))
        } else {
            nested(child, keys, from, depth + 1)
        }
    })
}

/// The message to show: the outer header's own fields, less any the content entity carries
/// itself, then the entity.
fn shown(outer_head: &[u8], entity: &[u8]) -> Vec<u8> {
    let inner = Entity::of(entity);
    let inner_fields = fields(inner.head);
    let carried: Vec<&[u8]> = inner_fields
        .iter()
        .copied()
        .filter(|field| !is_content_field(field))
        .map(field_name)
        .collect();
    let mut out = Vec::with_capacity(outer_head.len() + entity.len() + 32);
    for field in fields(outer_head) {
        let name = field_name(field);
        if is_content_field(field) || carried.iter().any(|c| c.eq_ignore_ascii_case(name)) {
            continue;
        }
        out.extend_from_slice(field);
    }
    out.extend_from_slice(b"MIME-Version: 1.0\r\n");
    for field in inner_fields {
        out.extend_from_slice(field);
    }
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(inner.body);
    out
}
