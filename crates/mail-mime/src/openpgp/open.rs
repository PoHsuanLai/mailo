//! Reading: a received message decrypted and its signature checked, on demand.
//!
//! Handles PGP/MIME (RFC 3156) — `multipart/encrypted` and `multipart/signed`, at the top or,
//! for a signature, nested inside something else — and inline PGP in a plain-text message as a
//! read-only fallback. What comes back is a whole message again, ready for [`crate::parse`]:
//! the decrypted or signed content under the original header, with any protected headers from
//! inside the encrypted part in place of the outer ones.

use super::autocrypt::{AutocryptHeader, gossip_of};
use super::entity::{Entity, MAX_DEPTH, crlf, field_name, fields, is_content_field, is_named};
use super::keys::{find, trust_of};
use super::{Keys, KnownCert, Protection, Unlocking, convert_fingerprint, convert_key_id};
use mail_domain::{Coverage, Encryption, KeyId, Verification};
use pgp::composed::SignedPublicSubKey;
use pgp::composed::{CleartextSignedMessage, Deserializable, DetachedSignature, Message, TheRing};
use pgp::packet::Signature;
use pgp::types::{Password, VerifyingKey};

/// A key a signature may have been made by: a certificate's primary key or one of its subkeys.
#[derive(Clone, Copy)]
enum Candidate<'a> {
    Primary(&'a pgp::packet::PublicKey),
    Sub(&'a SignedPublicSubKey),
}

impl Candidate<'_> {
    fn key(&self) -> &dyn VerifyingKey {
        match self {
            Candidate::Primary(key) => *key,
            Candidate::Sub(key) => *key,
        }
    }

    /// Whether `signature` is this key's over exactly `data`.
    fn signed(&self, signature: &Signature, data: &[u8]) -> bool {
        match self {
            Candidate::Primary(key) => signature.verify(*key, data).is_ok(),
            Candidate::Sub(key) => signature.verify(*key, data).is_ok(),
        }
    }
}

/// A message opened for reading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Opened {
    /// The message as the reader should show it: decrypted, or with the signature part taken
    /// off, under the original header. Parse it with [`crate::parse`]. Not to be stored: this
    /// is the decrypted content, and it is kept nowhere by this crate.
    pub message: Vec<u8>,
    pub encryption: Encryption,
    pub verification: Verification,
    /// `Autocrypt-Gossip` headers found inside the encrypted part: what the sender's client
    /// says the other recipients' keys are.
    pub gossip: Vec<AutocryptHeader>,
}

const PGP_ENCRYPTED: &str = "application/pgp-encrypted";
const PGP_SIGNATURE: &str = "application/pgp-signature";
const BEGIN_MESSAGE: &[u8] = b"-----BEGIN PGP MESSAGE-----";
const BEGIN_SIGNED: &[u8] = b"-----BEGIN PGP SIGNED MESSAGE-----";

/// The key ids an encrypted message is encrypted to, so the caller can fetch only those secret
/// keys from the keyring. `None` when the message is not encrypted.
pub fn encrypted_to(raw: &[u8]) -> Option<Vec<KeyId>> {
    let payload = encrypted_payload(raw)?;
    let message = message_from(&payload).ok()?;
    match &message {
        Message::Encrypted { esk, .. } => Some(recipients(esk)),
        _ => None,
    }
}

/// `raw` opened with `keys`: decrypted if it is encrypted and one of `keys.secrets` opens it,
/// its signature checked against `keys.certs`. `None` when it carries no OpenPGP at all.
pub fn open(raw: &[u8], keys: &Keys) -> Option<Opened> {
    let top = Entity::of(raw);
    let kind = top.content_type();
    if kind.is("multipart/encrypted", PGP_ENCRYPTED) {
        let payload = encrypted_payload(raw)?;
        return Some(open_encrypted(top, &payload, keys));
    }
    if kind.is("multipart/signed", PGP_SIGNATURE) {
        let (content, verification) = verify_signed(top, &keys.certs, Coverage::Whole)?;
        return Some(Opened {
            message: shown(top.head, content),
            encryption: Encryption::NotEncrypted,
            verification,
            gossip: Vec::new(),
        });
    }
    if kind.mime.starts_with("multipart/") {
        // A signed part inside something larger: a mailing list's footer appended beside it,
        // most often. Whatever sits beside it is unsigned, so the signature covers only part.
        let (_, verification) = nested_signed(top, &keys.certs, 0)?;
        return Some(Opened {
            message: raw.to_vec(),
            encryption: Encryption::NotEncrypted,
            verification,
            gossip: Vec::new(),
        });
    }
    if kind.mime == "text/plain" {
        return inline(raw, top, keys);
    }
    None
}

/// The armored OpenPGP message in the second part of a `multipart/encrypted`, or in a plain
/// text body.
fn encrypted_payload(raw: &[u8]) -> Option<Vec<u8>> {
    let top = Entity::of(raw);
    let kind = top.content_type();
    if kind.is("multipart/encrypted", PGP_ENCRYPTED) {
        let children = top.children();
        let payload = children.get(1)?;
        return Some(crate::reconstruct::decode_part(payload.raw, b""));
    }
    if kind.mime == "text/plain" {
        let text = crate::reconstruct::decode_part(raw, b"");
        return armored_block(&text, BEGIN_MESSAGE).map(|(_, block, _)| block.to_vec());
    }
    None
}

fn message_from(bytes: &[u8]) -> pgp::errors::Result<Message<'_>> {
    if find(bytes, b"-----BEGIN PGP").is_some() {
        Message::from_armor(bytes).map(|(message, _)| message)
    } else {
        Message::from_bytes(bytes)
    }
}

fn recipients(esk: &[pgp::composed::Esk]) -> Vec<KeyId> {
    esk.iter()
        .filter_map(|esk| match esk {
            pgp::composed::Esk::PublicKeyEncryptedSessionKey(pkesk) => {
                pkesk.id().ok().map(convert_key_id)
            }
            pgp::composed::Esk::SymKeyEncryptedSessionKey(_) => None,
        })
        .collect()
}

fn open_encrypted(top: Entity<'_>, payload: &[u8], keys: &Keys) -> Opened {
    match decrypt(payload, keys) {
        Err(encryption) => Opened {
            message: top.raw.to_vec(),
            encryption,
            verification: Verification::NoSignature,
            gossip: Vec::new(),
        },
        Ok(plain) => {
            let inner = Entity::of(&plain.data);
            let gossip = gossip_of(inner.head);
            // Signed, then encrypted, as two MIME layers (RFC 3156 §6.1) rather than one
            // OpenPGP message: the signature is on the inner multipart/signed.
            let nested = inner
                .content_type()
                .is("multipart/signed", PGP_SIGNATURE)
                .then(|| verify_signed(inner, &keys.certs, Coverage::Whole))
                .flatten();
            let (message, verification) = match nested {
                Some((content, verification)) => (shown(top.head, content), verification),
                None => (
                    shown(top.head, plain.data.clone()),
                    plain.verification.unwrap_or(Verification::NoSignature),
                ),
            };
            Opened {
                message,
                encryption: plain.encryption,
                verification,
                gossip,
            }
        }
    }
}

/// What decrypting (or just reading) an OpenPGP message produced.
struct Plain {
    data: Vec<u8>,
    encryption: Encryption,
    /// The OpenPGP message's own signature, when it was signed inside; `None` when it was not.
    verification: Option<Verification>,
}

/// Decrypt `payload` with whichever of `keys.secrets` it is encrypted to. A message that turns
/// out not to be encrypted — an inline signed-only `PGP MESSAGE` — is read as it is.
fn decrypt(payload: &[u8], keys: &Keys) -> Result<Plain, Encryption> {
    let unreadable = |e: &dyn std::fmt::Display| Encryption::Unreadable { why: e.to_string() };
    let message = message_from(payload).map_err(|e| unreadable(&e))?;
    let (message, encryption) = match message {
        Message::Encrypted { ref esk, .. } => {
            let to = recipients(esk);
            let anonymous = to.iter().any(KeyId::is_wildcard);
            let candidates: Vec<&Unlocking> = keys
                .secrets
                .iter()
                .filter(|s| anonymous || s.key.public().key_ids().iter().any(|id| to.contains(id)))
                .collect();
            if candidates.is_empty() {
                return Err(Encryption::CannotDecrypt { to });
            }
            let usable: Vec<&Unlocking> = candidates
                .iter()
                .copied()
                .filter(|s| {
                    s.key.protection() == Protection::Open || s.key.unlocks_with(&s.passphrase)
                })
                .collect();
            if usable.is_empty() {
                return Err(Encryption::Locked {
                    key: candidates[0].key.fingerprint(),
                });
            }
            let passwords: Vec<Password> = usable
                .iter()
                .map(|s| Password::from(s.passphrase.as_str()))
                .collect();
            let ring = TheRing {
                secret_keys: usable.iter().map(|s| &s.key.inner).collect(),
                key_passwords: passwords.iter().collect(),
                ..TheRing::default()
            };
            let (message, _) = message
                .decrypt_the_ring(ring, true)
                .map_err(|e| unreadable(&e))?;
            (message, Encryption::Decrypted)
        }
        other => (other, Encryption::NotEncrypted),
    };
    let mut message = if message.is_compressed() {
        message.decompress().map_err(|e| unreadable(&e))?
    } else {
        message
    };
    // Reading to the end is what checks the integrity code: a tampered ciphertext fails here.
    let data = message.as_data_vec().map_err(|e| unreadable(&e))?;
    let verification = match &message {
        Message::Signed { reader, .. } => {
            let signatures: Vec<(usize, Signature)> = (0..reader.num_signatures())
                .filter_map(|i| reader.signature(i).cloned().map(|sig| (i, sig)))
                .collect();
            Some(judge(
                &signatures,
                &keys.certs,
                Coverage::Whole,
                |index, key| message.verify_nested_explicit(index, key.key()).is_ok(),
            ))
        }
        _ => None,
    };
    Ok(Plain {
        data,
        encryption,
        verification,
    })
}

/// A `multipart/signed` entity checked: its signed content, and what the check found.
fn verify_signed(
    entity: Entity<'_>,
    certs: &[KnownCert],
    coverage: Coverage,
) -> Option<(Vec<u8>, Verification)> {
    let children = entity.children();
    let (signed, signature) = (children.first()?, children.get(1)?);
    let data = crlf(signed.raw);
    let armored = crate::reconstruct::decode_part(signature.raw, b"");
    let signatures: Vec<(usize, Signature)> =
        match DetachedSignature::from_reader_many(&armored[..]) {
            Ok((found, _)) => found
                .filter_map(Result::ok)
                .map(|detached| detached.signature)
                .enumerate()
                .collect(),
            Err(_) => Vec::new(),
        };
    let verification = if signatures.is_empty() {
        // A signature part that holds no signature is a signature that does not verify.
        Verification::Bad { coverage }
    } else {
        judge(&signatures, certs, coverage, |index, key| {
            signatures
                .iter()
                .find(|(i, _)| *i == index)
                .is_some_and(|(_, sig)| key.signed(sig, &data))
        })
    };
    Some((signed.raw.to_vec(), verification))
}

/// The first `multipart/signed` anywhere below `entity`, checked, with [`Coverage::Part`].
fn nested_signed(
    entity: Entity<'_>,
    certs: &[KnownCert],
    depth: usize,
) -> Option<(Vec<u8>, Verification)> {
    if depth > MAX_DEPTH {
        return None;
    }
    entity.children().into_iter().find_map(|child| {
        if child.content_type().is("multipart/signed", PGP_SIGNATURE) {
            verify_signed(child, certs, Coverage::Part)
        } else {
            nested_signed(child, certs, depth + 1)
        }
    })
}

/// The verdict on a set of signatures: good if any signature by a known key verifies, bad if one
/// by a known key does not, unknown if no signer is known.
fn judge(
    signatures: &[(usize, Signature)],
    certs: &[KnownCert],
    coverage: Coverage,
    verifies: impl Fn(usize, Candidate<'_>) -> bool,
) -> Verification {
    let mut bad = false;
    for (index, signature) in signatures {
        let ids: Vec<KeyId> = signature
            .issuer_key_id()
            .into_iter()
            .map(convert_key_id)
            .chain(
                signature
                    .issuer_fingerprint()
                    .into_iter()
                    .filter_map(convert_fingerprint)
                    .map(|fp| fp.key_id()),
            )
            .collect();
        for known in certs {
            let key = &known.cert.inner;
            let primary = std::iter::once(Candidate::Primary(&key.primary_key))
                .filter(|k| ids.contains(&convert_key_id(&k.key().legacy_key_id())));
            let subkeys = key
                .public_subkeys
                .iter()
                .map(Candidate::Sub)
                .filter(|k| ids.contains(&convert_key_id(&k.key().legacy_key_id())));
            for candidate in primary.chain(subkeys) {
                if verifies(*index, candidate) {
                    let signer = known.cert.fingerprint();
                    return Verification::Good {
                        signer,
                        trust: trust_of(certs, signer),
                        coverage,
                    };
                }
                bad = true;
            }
        }
    }
    if bad {
        return Verification::Bad { coverage };
    }
    let issuer = signatures
        .iter()
        .find_map(|(_, sig)| {
            sig.issuer_fingerprint()
                .into_iter()
                .filter_map(convert_fingerprint)
                .map(|fp| fp.key_id())
                .next()
                .or_else(|| sig.issuer_key_id().into_iter().map(convert_key_id).next())
        })
        .unwrap_or(KeyId([0; 8]));
    Verification::UnknownKey { issuer, coverage }
}

/// Inline PGP in a plain-text body: an armored `PGP MESSAGE` decrypted in place, or a
/// cleartext-signed block checked. Text outside the block makes the coverage partial.
fn inline(raw: &[u8], top: Entity<'_>, keys: &Keys) -> Option<Opened> {
    let text = crate::reconstruct::decode_part(raw, b"");
    if let Some((before, block, after)) = armored_block(&text, BEGIN_MESSAGE) {
        let coverage = coverage_of(before, after);
        return Some(match decrypt(block, keys) {
            Err(encryption) => Opened {
                message: raw.to_vec(),
                encryption,
                verification: Verification::NoSignature,
                gossip: Vec::new(),
            },
            Ok(plain) => Opened {
                message: shown(top.head, plain_text(&[before, &plain.data, after].concat())),
                encryption: plain.encryption,
                verification: match plain.verification {
                    Some(Verification::Good { signer, trust, .. }) => Verification::Good {
                        signer,
                        trust,
                        coverage,
                    },
                    Some(Verification::Bad { .. }) => Verification::Bad { coverage },
                    Some(Verification::UnknownKey { issuer, .. }) => {
                        Verification::UnknownKey { issuer, coverage }
                    }
                    Some(Verification::NoSignature) | None => Verification::NoSignature,
                },
                gossip: Vec::new(),
            },
        });
    }
    let (before, block, after) = armored_block(&text, BEGIN_SIGNED)?;
    let coverage = coverage_of(before, after);
    let block = String::from_utf8_lossy(block);
    let (signed, _) = CleartextSignedMessage::from_string(&block).ok()?;
    let signatures: Vec<(usize, Signature)> =
        signed.signatures().iter().cloned().enumerate().collect();
    let verification = judge(&signatures, &keys.certs, coverage, |index, key| {
        signed
            .signatures()
            .get(index)
            .is_some_and(|sig| key.signed(sig, signed.signed_text().as_bytes()))
    });
    Some(Opened {
        message: shown(
            top.head,
            plain_text(&[before, signed.text().as_bytes(), after].concat()),
        ),
        encryption: Encryption::NotEncrypted,
        verification,
        gossip: Vec::new(),
    })
}

/// `text` split around the first armored block that starts with `begin`: the text before, the
/// block through its `-----END PGP …-----` line, and the text after.
fn armored_block<'a>(text: &'a [u8], begin: &[u8]) -> Option<(&'a [u8], &'a [u8], &'a [u8])> {
    let start = find(text, begin)?;
    let end_marker = start + find(&text[start..], b"-----END PGP ")?;
    let end = text[end_marker..]
        .iter()
        .position(|&b| b == b'\n')
        .map_or(text.len(), |at| end_marker + at + 1);
    Some((&text[..start], &text[start..end], &text[end..]))
}

fn coverage_of(before: &[u8], after: &[u8]) -> Coverage {
    if before.trim_ascii().is_empty() && after.trim_ascii().is_empty() {
        Coverage::Whole
    } else {
        Coverage::Part
    }
}

/// A plain-text entity holding `text`.
fn plain_text(text: &[u8]) -> Vec<u8> {
    let mut out =
        b"Content-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: 8bit\r\n\r\n"
            .to_vec();
    out.extend_from_slice(text);
    out
}

/// The message to show: the outer header's own fields, with any the content entity carries
/// itself (protected headers) taken from the entity instead, then the entity.
fn shown(outer_head: &[u8], entity: Vec<u8>) -> Vec<u8> {
    let inner = Entity::of(&entity);
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
        // Gossip is for this client, not for the reader, and it is several kilobytes of base64.
        if !is_named(field, "Autocrypt-Gossip") {
            out.extend_from_slice(field);
        }
    }
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(inner.body);
    out
}
