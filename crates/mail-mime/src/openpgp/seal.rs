//! Sending: a built message made PGP/MIME — signed (RFC 3156 §5), encrypted (§4), or both in
//! one OpenPGP message (§6.2).
//!
//! Only the content entity is signed or encrypted — its `Content-*` fields and body — never the
//! message's own header. That is RFC 3156's design, and it is what lets the outbox rewrite
//! `Date` as the message leaves (`crate::restamp`) without breaking the signature made when
//! the user pressed Send.
//!
//! An encrypted message's subject would otherwise travel in the clear, so it is carried inside
//! the encrypted part as a protected header, with `...` outside (the header protection of RFC
//! 9788, in its widely read "protected-headers=v1" form). `Date` is deliberately not among the
//! protected fields: the outer one is replaced as the message leaves, and an inner copy would
//! disagree with it.

use super::entity::{Entity, crlf, field_name, fields, is_content_field};
use super::keys::{Cert, live, newest, timestamp};
use super::{Rng, Unlocking, pgp_error};
use crate::MimeError;
use chrono::{DateTime, Utc};
use mail_domain::{OpenPgp, PreferEncrypt};
use pgp::composed::{
    ArmorOptions, DetachedSignature, MessageBuilder, SignedPublicKey, SignedPublicSubKey,
    SignedSecretKey, SubpacketConfig,
};
use pgp::crypto::hash::HashAlgorithm;
use pgp::crypto::sym::SymmetricKeyAlgorithm;
use pgp::packet::{KeyFlags, Subpacket, SubpacketData};
use pgp::types::{KeyDetails, Password, SigningKey};

/// What to do to a message as it is sent.
#[derive(Debug, Clone, Copy)]
pub struct Sealing<'a> {
    pub mode: OpenPgp,
    /// The sender's secret key and its passphrase. Required to sign.
    pub signer: Option<&'a Unlocking>,
    /// Everyone to encrypt to — every recipient *and the sender*, so the sent copy can be read.
    /// Required to encrypt.
    pub recipients: &'a [Cert],
    /// Recipients' keys to gossip inside the encrypted part (Autocrypt Level 1 key gossip), as
    /// `(address, key)`. Empty for one recipient, where there is nobody to tell.
    pub gossip: &'a [(String, Cert)],
    /// The signature's creation time.
    pub now: DateTime<Utc>,
}

/// The fields copied into the encrypted part as protected headers. `Bcc` never: those
/// addresses are exactly what the recipients must not learn.
const PROTECTED: &[&str] = &[
    "From",
    "To",
    "Cc",
    "Reply-To",
    "Subject",
    "Message-ID",
    "In-Reply-To",
    "References",
];

/// `frozen` — a whole message as `crate::build` writes it — signed, encrypted, or both, as
/// `how.mode` says. [`OpenPgp::None`] returns it as it was.
pub fn seal(frozen: &[u8], how: &Sealing<'_>, rng: &mut impl Rng) -> Result<Vec<u8>, MimeError> {
    let message = Entity::of(frozen);
    let (outer, content): (Vec<&[u8]>, Vec<&[u8]>) = fields(message.head)
        .into_iter()
        .partition(|field| !is_content_field(field));
    let content: Vec<&[u8]> = content
        .into_iter()
        .filter(|field| !field_name(field).eq_ignore_ascii_case(b"MIME-Version"))
        .collect();
    let boundary = boundary(rng);
    match how.mode {
        OpenPgp::None => Ok(frozen.to_vec()),
        OpenPgp::Sign => {
            let signer = how.signer.ok_or(MimeError::NoSigningKey)?;
            let entity =
                crlf(&[content.concat(), b"\r\n".to_vec(), message.body.to_vec()].concat());
            let signature = detached(&entity, signer, how.now, rng)?;
            Ok(signed(&outer, &entity, &signature, &boundary))
        }
        OpenPgp::Encrypt | OpenPgp::SignAndEncrypt => {
            if how.recipients.is_empty() {
                return Err(MimeError::NoRecipientKeys);
            }
            let signer = if how.mode.signs() {
                Some(how.signer.ok_or(MimeError::NoSigningKey)?)
            } else {
                None
            };
            let inner = protected(&outer, &content, message.body, how.gossip);
            let armored = encrypt(&inner, how.recipients, signer, how.now, rng)?;
            Ok(encrypted(&outer, &armored, &boundary))
        }
    }
}

/// A boundary no content will contain: 128 random bits, hex.
fn boundary(rng: &mut impl Rng) -> String {
    let mut bytes = [0u8; 16];
    rng.fill_bytes(&mut bytes);
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!("mailo-pgp-{hex}")
}

/// The multipart/signed message: the outer fields, the entity as it was signed, the signature.
fn signed(outer: &[&[u8]], entity: &[u8], signature: &str, boundary: &str) -> Vec<u8> {
    let mut out = outer.concat();
    out.extend_from_slice(
        format!(
            "MIME-Version: 1.0\r\nContent-Type: multipart/signed; micalg=pgp-sha256;\r\n \
             protocol=\"application/pgp-signature\"; boundary=\"{boundary}\"\r\n\r\n\
             This is an OpenPGP/MIME signed message (RFC 4880 and 3156)\r\n--{boundary}\r\n"
        )
        .as_bytes(),
    );
    out.extend_from_slice(entity);
    out.extend_from_slice(
        format!(
            "\r\n--{boundary}\r\nContent-Type: application/pgp-signature; name=\"signature.asc\"\r\n\
             Content-Description: OpenPGP digital signature\r\n\
             Content-Disposition: attachment; filename=\"signature.asc\"\r\n\r\n"
        )
        .as_bytes(),
    );
    out.extend_from_slice(&crlf(signature.as_bytes()));
    out.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    out
}

/// The multipart/encrypted message: the outer fields with the subject hidden, the version
/// part, the OpenPGP message.
fn encrypted(outer: &[&[u8]], armored: &str, boundary: &str) -> Vec<u8> {
    let mut out = Vec::new();
    for field in outer {
        if field_name(field).eq_ignore_ascii_case(b"Subject") {
            out.extend_from_slice(b"Subject: ...\r\n");
        } else {
            out.extend_from_slice(field);
        }
    }
    out.extend_from_slice(
        format!(
            "MIME-Version: 1.0\r\nContent-Type: multipart/encrypted;\r\n \
             protocol=\"application/pgp-encrypted\"; boundary=\"{boundary}\"\r\n\r\n\
             This is an OpenPGP/MIME encrypted message (RFC 4880 and 3156)\r\n\
             --{boundary}\r\nContent-Type: application/pgp-encrypted\r\n\
             Content-Description: PGP/MIME version identification\r\n\r\nVersion: 1\r\n\r\n\
             --{boundary}\r\nContent-Type: application/octet-stream; name=\"encrypted.asc\"\r\n\
             Content-Description: OpenPGP encrypted message\r\n\
             Content-Disposition: inline; filename=\"encrypted.asc\"\r\n\r\n"
        )
        .as_bytes(),
    );
    out.extend_from_slice(&crlf(armored.as_bytes()));
    out.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    out
}

/// The entity that is encrypted: protected copies of the message's own fields, gossip, then
/// the content with `protected-headers="v1"` on its type.
fn protected(
    outer: &[&[u8]],
    content: &[&[u8]],
    body: &[u8],
    gossip: &[(String, Cert)],
) -> Vec<u8> {
    let mut inner = Vec::new();
    for field in outer {
        if PROTECTED
            .iter()
            .any(|name| field_name(field).eq_ignore_ascii_case(name.as_bytes()))
        {
            inner.extend_from_slice(field);
        }
    }
    for (address, key) in gossip {
        inner.extend_from_slice(&super::autocrypt_field(
            "Autocrypt-Gossip",
            address,
            PreferEncrypt::NoPreference,
            key,
        ));
    }
    let mut typed = false;
    for field in content {
        if field_name(field).eq_ignore_ascii_case(b"Content-Type") {
            let bare = field
                .strip_suffix(b"\r\n")
                .or_else(|| field.strip_suffix(b"\n"))
                .unwrap_or(field);
            inner.extend_from_slice(bare);
            inner.extend_from_slice(b"; protected-headers=\"v1\"\r\n");
            typed = true;
        } else {
            inner.extend_from_slice(field);
        }
    }
    if !typed {
        inner.extend_from_slice(
            b"Content-Type: text/plain; charset=utf-8; protected-headers=\"v1\"\r\n",
        );
    }
    inner.extend_from_slice(b"\r\n");
    inner.extend_from_slice(body);
    crlf(&inner)
}

/// The signature subpackets, with the creation time we were given rather than the clock's.
fn subpackets(key: &dyn SigningKey, now: DateTime<Utc>) -> Result<SubpacketConfig, MimeError> {
    let hashed = vec![
        Subpacket::regular(SubpacketData::IssuerFingerprint(key.fingerprint()))
            .map_err(pgp_error)?,
        Subpacket::regular(SubpacketData::SignatureCreationTime(timestamp(now)))
            .map_err(pgp_error)?,
    ];
    let unhashed = vec![
        Subpacket::regular(SubpacketData::IssuerKeyId(key.legacy_key_id())).map_err(pgp_error)?,
    ];
    Ok(SubpacketConfig::UserDefined { hashed, unhashed })
}

/// A detached signature over `data`, armored.
fn detached(
    data: &[u8],
    signer: &Unlocking,
    now: DateTime<Utc>,
    rng: &mut impl Rng,
) -> Result<String, MimeError> {
    let secret = unlocked(signer)?;
    let key = signing_key(secret)?;
    let password = Password::from(signer.passphrase.as_str());
    let signature = DetachedSignature::sign_binary_data_with_subpackets(
        &mut *rng,
        &Box::new(key),
        &password,
        HashAlgorithm::Sha256,
        data,
        subpackets(key, now)?,
    )
    .map_err(pgp_error)?;
    signature
        .to_armored_string(ArmorOptions::default())
        .map_err(pgp_error)
}

/// `plaintext` as an armored OpenPGP message encrypted to every recipient, signed first when
/// `signer` is given.
///
/// SEIPD version 1 with AES-256: what every version 4 key's owner can read. The newer AEAD
/// packet needs the recipient to have said it reads one, and version 4 keys mostly have not.
fn encrypt(
    plaintext: &[u8],
    recipients: &[Cert],
    signer: Option<&Unlocking>,
    now: DateTime<Utc>,
    rng: &mut impl Rng,
) -> Result<String, MimeError> {
    let mut builder = MessageBuilder::from_bytes("", plaintext.to_vec())
        .seipd_v1(&mut *rng, SymmetricKeyAlgorithm::AES256);
    for cert in recipients {
        let targets = encryption_keys(cert, now);
        if targets.is_empty() {
            return Err(MimeError::CannotEncryptTo(cert.fingerprint()));
        }
        for target in targets {
            match target {
                Target::Sub(sub) => builder.encrypt_to_key(&mut *rng, sub),
                Target::Primary(primary) => builder.encrypt_to_key(&mut *rng, primary),
            }
            .map_err(pgp_error)?;
        }
    }
    if let Some(signer) = signer {
        let secret = unlocked(signer)?;
        let key = signing_key(secret)?;
        builder.sign_with_subpackets(
            key,
            Password::from(signer.passphrase.as_str()),
            HashAlgorithm::Sha256,
            subpackets(key, now)?,
        );
        return builder
            .to_armored_string(&mut *rng, ArmorOptions::default())
            .map_err(pgp_error);
    }
    builder
        .to_armored_string(&mut *rng, ArmorOptions::default())
        .map_err(pgp_error)
}

/// The signer's key, refused up front when its passphrase does not open it: rPGP's own error
/// for that says only that something failed.
fn unlocked(signer: &Unlocking) -> Result<&SignedSecretKey, MimeError> {
    if signer.key.protection() == super::Protection::Passphrase
        && !signer.key.unlocks_with(&signer.passphrase)
    {
        return Err(MimeError::KeyLocked(signer.key.fingerprint()));
    }
    Ok(&signer.key.inner)
}

/// The part of a secret key that signs: the primary key when its flags let it (or it carries
/// none and its algorithm can), otherwise the newest subkey flagged for signing.
fn signing_key(secret: &SignedSecretKey) -> Result<&dyn SigningKey, MimeError> {
    let primary_flags = secret
        .details
        .users
        .iter()
        .filter_map(|user| newest(&user.signatures))
        .map(|sig| sig.key_flags())
        .next()
        .unwrap_or_default();
    let primary_signs = if primary_flags == KeyFlags::default() {
        secret.primary_key.algorithm().can_sign()
    } else {
        primary_flags.sign()
    };
    if primary_signs {
        return Ok(&secret.primary_key);
    }
    secret
        .secret_subkeys
        .iter()
        .rev()
        .find(|sub| newest(&sub.signatures).is_some_and(|sig| sig.key_flags().sign()))
        .map(|sub| &sub.key as &dyn SigningKey)
        .ok_or_else(|| MimeError::OpenPgp("that key has no part that can sign".to_owned()))
}

/// Where to encrypt to in one certificate.
pub(super) enum Target<'a> {
    Sub(&'a SignedPublicSubKey),
    Primary(&'a SignedPublicKey),
}

/// The part of `cert` to encrypt to at `now`: its newest live subkey flagged for encryption,
/// or the primary key when it is flagged for encryption itself and no subkey is. Empty when
/// nothing in it can be encrypted to.
pub(super) fn encryption_keys(cert: &Cert, now: DateTime<Utc>) -> Vec<Target<'_>> {
    let key = &cert.inner;
    let newest_sub = key
        .public_subkeys
        .iter()
        .filter(|sub| live(sub, now))
        .filter(|sub| {
            newest(&sub.signatures).is_some_and(|sig| {
                let flags = sig.key_flags();
                if flags == KeyFlags::default() {
                    sub.key.algorithm().can_encrypt()
                } else {
                    flags.encrypt_comms() || flags.encrypt_storage()
                }
            })
        })
        .max_by_key(|sub| sub.key.created_at().as_secs());
    if let Some(sub) = newest_sub {
        return vec![Target::Sub(sub)];
    }
    let primary_encrypts = key
        .details
        .users
        .iter()
        .filter_map(|user| newest(&user.signatures))
        .any(|sig| sig.key_flags().encrypt_comms() || sig.key_flags().encrypt_storage());
    if primary_encrypts && key.primary_key.algorithm().can_encrypt() {
        vec![Target::Primary(key)]
    } else {
        Vec::new()
    }
}
