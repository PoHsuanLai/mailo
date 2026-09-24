//! OpenPGP keys: reading them from files and headers, making one, and saying what one is.
//!
//! [`Cert`] is a public key and [`SecretCert`] a secret one, each already checked: every
//! self-signature binding its user ids and subkeys verified when it was read, so nothing past
//! this module holds a key whose parts were stitched together by someone else.

use super::{Rng, convert_fingerprint, convert_key_id, pgp_error};
use crate::MimeError;
use chrono::{DateTime, Utc};
use mail_domain::{Fingerprint, KeyId, KeySource, KeyTrust, PgpKey, SecretHeld};
use pgp::composed::{
    ArmorOptions, EncryptionCaps, KeyType, PublicOrSecret, SecretKeyParamsBuilder, SignedPublicKey,
    SignedPublicSubKey, SignedSecretKey, SubkeyParamsBuilder,
};
use pgp::crypto::ecc_curve::ECCCurve;
use pgp::crypto::hash::HashAlgorithm;
use pgp::crypto::sym::SymmetricKeyAlgorithm;
use pgp::packet::Signature;
use pgp::ser::Serialize as _;
use pgp::types::{CompressionAlgorithm, KeyDetails, Password, Timestamp};
use std::fmt;

/// A public key — a correspondent's, or the public half of the user's own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cert {
    pub(super) inner: SignedPublicKey,
}

/// A secret key: the user's own, as the keyring holds it.
///
/// `Debug` is written by hand for the reason `Credential`'s is: a derived one would print the
/// secret key material into every log line that ever touched it.
#[derive(Clone, PartialEq, Eq)]
pub struct SecretCert {
    pub(super) inner: SignedSecretKey,
}

impl fmt::Debug for SecretCert {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SecretCert")
            .field("fingerprint", &self.fingerprint())
            .field("secret", &"<redacted>")
            .finish()
    }
}

/// Whether a secret key's material is locked by a passphrase of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protection {
    /// Usable as it is. A key generated here is this: the OS keyring is its protection.
    Open,
    /// Needs its passphrase before it can sign or decrypt.
    Passphrase,
}

/// One key read from a file: public, or secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadKey {
    Public(Box<Cert>),
    Secret(Box<SecretCert>),
}

impl Cert {
    /// The key's fingerprint.
    pub fn fingerprint(&self) -> Fingerprint {
        fingerprint_of(&self.inner.primary_key.fingerprint())
    }

    /// The primary key's id and every subkey's.
    pub fn key_ids(&self) -> Vec<KeyId> {
        std::iter::once(convert_key_id(&self.inner.primary_key.legacy_key_id()))
            .chain(
                self.inner
                    .public_subkeys
                    .iter()
                    .map(|sub| convert_key_id(&sub.key.legacy_key_id())),
            )
            .collect()
    }

    /// The user ids, as text. One that is not UTF-8 is left out rather than guessed at.
    pub fn user_ids(&self) -> Vec<String> {
        self.inner
            .details
            .users
            .iter()
            .filter_map(|user| user.id.as_str().map(str::to_owned))
            .collect()
    }

    /// The addresses in the user ids, lower-cased, in order, each once.
    pub fn emails(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for email in self.user_ids().iter().filter_map(|uid| address_in(uid)) {
            if !out.contains(&email) {
                out.push(email);
            }
        }
        out
    }

    /// The transferable public key, binary: what the store keeps and Autocrypt carries.
    pub fn to_bytes(&self) -> Vec<u8> {
        // Serializing a key that was parsed or built by the library does not fail: the only
        // error path is the writer, and a `Vec` does not refuse bytes.
        self.inner.to_bytes().unwrap_or_default()
    }

    /// The key ASCII-armored, as `mailo pgp export` writes it.
    pub fn armored(&self) -> Result<String, MimeError> {
        self.inner
            .to_armored_string(ArmorOptions::default())
            .map_err(pgp_error)
    }

    /// The key as a store row, seen at `now` by way of `source`, for `extra` addresses beside
    /// those in its user ids (the address an Autocrypt header or WKD lookup delivered it for).
    pub fn record(&self, source: KeySource, now: DateTime<Utc>, extra: &[&str]) -> PgpKey {
        let mut emails = self.emails();
        for address in extra {
            let address = address.trim().to_ascii_lowercase();
            if !address.is_empty() && !emails.contains(&address) {
                emails.push(address);
            }
        }
        PgpKey {
            fingerprint: self.fingerprint(),
            key_ids: self.key_ids(),
            user_ids: self.user_ids(),
            emails,
            key: self.to_bytes(),
            source,
            first_seen: now,
            last_seen: now,
            trust: KeyTrust::Unverified,
            secret: SecretHeld::Absent,
        }
    }

    /// Whether some part of this key can be encrypted to at `now`.
    pub fn can_encrypt(&self, now: DateTime<Utc>) -> bool {
        !super::seal::encryption_keys(self, now).is_empty()
    }

    /// Read back a key the store holds.
    pub fn from_bytes(bytes: &[u8]) -> Result<Cert, MimeError> {
        match read_keys(bytes)?.into_iter().next() {
            Some(ReadKey::Public(cert)) => Ok(*cert),
            Some(ReadKey::Secret(secret)) => Ok(secret.public()),
            None => Err(MimeError::OpenPgp("no key in the bytes".to_owned())),
        }
    }
}

impl SecretCert {
    /// The key's fingerprint.
    pub fn fingerprint(&self) -> Fingerprint {
        fingerprint_of(&self.inner.primary_key.fingerprint())
    }

    /// The public half.
    pub fn public(&self) -> Cert {
        Cert {
            inner: self.inner.to_public_key(),
        }
    }

    /// Whether signing or decrypting with it needs a passphrase.
    pub fn protection(&self) -> Protection {
        let locked = self.inner.primary_key.secret_params().is_encrypted()
            || self
                .inner
                .secret_subkeys
                .iter()
                .any(|sub| sub.key.secret_params().is_encrypted());
        if locked {
            Protection::Passphrase
        } else {
            Protection::Open
        }
    }

    /// Whether `passphrase` opens this key. Always true for an [`Protection::Open`] key.
    pub fn unlocks_with(&self, passphrase: &str) -> bool {
        let password = Password::from(passphrase);
        let primary = self
            .inner
            .primary_key
            .unlock(&password, |_, _| Ok(()))
            .is_ok_and(|inner| inner.is_ok());
        primary
            && self.inner.secret_subkeys.iter().all(|sub| {
                sub.key
                    .unlock(&password, |_, _| Ok(()))
                    .is_ok_and(|inner| inner.is_ok())
            })
    }

    /// The key ASCII-armored: what the keyring holds, and what `export --secret` prints.
    pub fn armored(&self) -> Result<String, MimeError> {
        self.inner
            .to_armored_string(ArmorOptions::default())
            .map_err(pgp_error)
    }

    /// This key with its secret material locked by `passphrase` — the primary key and every
    /// subkey — for handing to another program.
    pub fn with_passphrase(
        &self,
        passphrase: &str,
        rng: &mut impl Rng,
    ) -> Result<SecretCert, MimeError> {
        let password = Password::from(passphrase);
        let mut inner = self.inner.clone();
        inner
            .primary_key
            .set_password(&mut *rng, &password)
            .map_err(pgp_error)?;
        for sub in &mut inner.secret_subkeys {
            sub.key
                .set_password(&mut *rng, &password)
                .map_err(pgp_error)?;
        }
        Ok(SecretCert { inner })
    }

    /// Read back what [`SecretCert::armored`] wrote.
    pub fn from_armored(text: &str) -> Result<SecretCert, MimeError> {
        match read_keys(text.as_bytes())?.into_iter().next() {
            Some(ReadKey::Secret(secret)) => Ok(*secret),
            Some(ReadKey::Public(_)) | None => Err(MimeError::OpenPgp(
                "the keyring entry holds no secret key".to_owned(),
            )),
        }
    }
}

/// Every key in `bytes`: an armored block, several concatenated, or a binary keyring.
///
/// Each key's self-signatures are checked; a key whose user ids or subkeys are not bound by its
/// own signature is refused, named by its fingerprint, rather than imported half-trusted.
pub fn read_keys(bytes: &[u8]) -> Result<Vec<ReadKey>, MimeError> {
    let blocks = armor_blocks(bytes);
    let mut out = Vec::new();
    if blocks.is_empty() {
        read_block(bytes, &mut out)?;
    } else {
        for block in blocks {
            read_block(block, &mut out)?;
        }
    }
    if out.is_empty() {
        return Err(MimeError::OpenPgp("no OpenPGP key was found".to_owned()));
    }
    Ok(out)
}

fn read_block(bytes: &[u8], out: &mut Vec<ReadKey>) -> Result<(), MimeError> {
    let (keys, _) = PublicOrSecret::from_reader_many(bytes).map_err(pgp_error)?;
    for key in keys {
        let key = key.map_err(pgp_error)?;
        if let Err(e) = key.verify_bindings() {
            let fingerprint = match &key {
                PublicOrSecret::Public(k) => k.primary_key.fingerprint(),
                PublicOrSecret::Secret(k) => k.primary_key.fingerprint(),
            };
            return Err(MimeError::OpenPgp(format!(
                "key {} is not bound by its own signatures: {e}",
                fingerprint_of(&fingerprint)
            )));
        }
        out.push(match key {
            PublicOrSecret::Public(inner) => ReadKey::Public(Box::new(Cert { inner })),
            PublicOrSecret::Secret(inner) => ReadKey::Secret(Box::new(SecretCert { inner })),
        });
    }
    Ok(())
}

/// The armored blocks in `bytes`, each from its `-----BEGIN PGP` line to its `-----END PGP`
/// line. Empty when there are none, which is what a binary keyring looks like.
fn armor_blocks(bytes: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut rest = bytes;
    let mut offset = 0;
    while let Some(start) = find(rest, b"-----BEGIN PGP ") {
        let from = offset + start;
        let after = &bytes[from..];
        let Some(end_marker) = find(after, b"-----END PGP ") else {
            break;
        };
        // To the end of the END line.
        let line_end = after[end_marker..]
            .iter()
            .position(|&b| b == b'\n')
            .map_or(after.len(), |at| end_marker + at + 1);
        out.push(&bytes[from..from + line_end]);
        offset = from + line_end;
        rest = &bytes[offset..];
    }
    out
}

pub(super) fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Make a key for `user_id` — `Name <address>` — created at `now`.
///
/// Version 4, with an Ed25519 primary key that certifies and signs and a Curve25519 subkey that
/// encrypts: the pair every current OpenPGP implementation reads, GnuPG 2.2 included. Version 6
/// keys and the RFC 9580 Ed25519/X25519 algorithm ids are newer than much of what correspondents
/// run, and a key they cannot use is a key that does not work. No passphrase: the OS keyring
/// protects it, as it protects the account passwords.
pub fn generate(
    user_id: &str,
    now: DateTime<Utc>,
    rng: &mut impl Rng,
) -> Result<SecretCert, MimeError> {
    let created = timestamp(now);
    let encrypting = SubkeyParamsBuilder::default()
        .key_type(KeyType::ECDH(ECCCurve::Curve25519Legacy))
        .can_encrypt(EncryptionCaps::All)
        .created_at(created)
        .build()
        .map_err(|e| MimeError::OpenPgp(e.to_string()))?;
    let params = SecretKeyParamsBuilder::default()
        .key_type(KeyType::Ed25519Legacy)
        .can_certify(true)
        .can_sign(true)
        .primary_user_id(user_id.to_owned())
        .created_at(created)
        .preferred_symmetric_algorithms(smallvec::smallvec![
            SymmetricKeyAlgorithm::AES256,
            SymmetricKeyAlgorithm::AES128,
        ])
        .preferred_hash_algorithms(smallvec::smallvec![
            HashAlgorithm::Sha256,
            HashAlgorithm::Sha512,
        ])
        .preferred_compression_algorithms(smallvec::smallvec![
            CompressionAlgorithm::ZLIB,
            CompressionAlgorithm::ZIP,
        ])
        .subkeys(vec![encrypting])
        .build()
        .map_err(|e| MimeError::OpenPgp(e.to_string()))?;
    let inner = params.generate(rng).map_err(pgp_error)?;
    Ok(SecretCert { inner })
}

pub(super) fn timestamp(at: DateTime<Utc>) -> Timestamp {
    // OpenPGP times are unsigned 32-bit seconds; outside that range is not a time a message is
    // sent at, and clamping keeps a broken clock from failing a send.
    Timestamp::from_secs(u32::try_from(at.timestamp().max(0)).unwrap_or(u32::MAX))
}

fn fingerprint_of(fp: &pgp::types::Fingerprint) -> Fingerprint {
    // A parsed key's fingerprint is 20 or 32 bytes by construction; anything else is a key
    // version this library would not have parsed.
    convert_fingerprint(fp).unwrap_or(Fingerprint::V4([0; 20]))
}

/// The address in a user id: inside the last `<…>`, or the whole id when it is a bare address.
/// Lower-cased. `None` when there is no `@`.
fn address_in(user_id: &str) -> Option<String> {
    let inner = match (user_id.rfind('<'), user_id.rfind('>')) {
        (Some(open), Some(close)) if open < close => &user_id[open + 1..close],
        _ => user_id,
    };
    let inner = inner.trim();
    (inner.contains('@') && !inner.contains(char::is_whitespace))
        .then(|| inner.to_ascii_lowercase())
}

/// The newest binding signature among `signatures`: the one that says what the key is for now.
pub(super) fn newest(signatures: &[Signature]) -> Option<&Signature> {
    signatures
        .iter()
        .max_by_key(|sig| sig.created().map(Timestamp::as_secs).unwrap_or(0))
}

/// Whether a subkey's newest binding still holds at `now`: not expired.
pub(super) fn live(sub: &SignedPublicSubKey, now: DateTime<Utc>) -> bool {
    let Some(binding) = newest(&sub.signatures) else {
        return false;
    };
    match binding.key_expiration_time().map(|d| d.as_secs()) {
        None | Some(0) => true,
        Some(secs) => {
            let created = i64::from(sub.key.created_at().as_secs());
            now.timestamp() < created + i64::from(secs)
        }
    }
}

/// Whether this certificate's own trust state should be reported as the user's.
pub(super) fn trust_of(certs: &[super::KnownCert], fingerprint: Fingerprint) -> KeyTrust {
    certs
        .iter()
        .find(|known| known.cert.fingerprint() == fingerprint)
        .map_or(KeyTrust::Unverified, |known| known.trust)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_address_is_read_out_of_a_user_id() {
        const CASES: &[(&str, Option<&str>)] = &[
            ("Joe Doe <Joe.Doe@Example.ORG>", Some("joe.doe@example.org")),
            ("joe@example.org", Some("joe@example.org")),
            ("Joe Doe", None),
            ("Joe <not an address>", None),
        ];
        for (uid, expected) in CASES {
            assert_eq!(address_in(uid).as_deref(), *expected, "{uid}");
        }
    }

    #[test]
    fn armored_blocks_are_found_one_by_one() {
        let text = b"junk\n-----BEGIN PGP PUBLIC KEY BLOCK-----\nA\n-----END PGP PUBLIC KEY BLOCK-----\n\
                     between\n-----BEGIN PGP PUBLIC KEY BLOCK-----\nB\n-----END PGP PUBLIC KEY BLOCK-----\n";
        let blocks = armor_blocks(text);
        assert_eq!(blocks.len(), 2);
        assert!(blocks[1].starts_with(b"-----BEGIN") && blocks[1].ends_with(b"BLOCK-----\n"));
        assert!(armor_blocks(b"\x99\x01binary").is_empty());
    }
}
