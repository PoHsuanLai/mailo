//! OpenPGP as product vocabulary: which key, whose, how far it is trusted, what a draft asks
//! for, and what reading a protected message found.
//!
//! No cryptography here. The keys themselves are parsed, made and used in `mail-mime`, where
//! the OpenPGP implementation lives; the store keeps [`PgpKey`] rows, the OS keyring keeps the
//! secret halves ([`crate::SecretPurpose::OpenPgp`]), and these types are what passes between
//! them.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// An OpenPGP key's fingerprint: what names a key everywhere in this client.
///
/// Two lengths, because the key version decides the hash — SHA-1 for version 4, SHA-256 for
/// version 6 (RFC 9580 §5.5.4) — and a fingerprint of the wrong length names no key at all.
/// Written as upper-case hex, which is how every OpenPGP tool prints one, and stored that way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum Fingerprint {
    V4([u8; 20]),
    V6([u8; 32]),
}

impl Fingerprint {
    /// The fingerprint's bytes.
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            Fingerprint::V4(bytes) => bytes,
            Fingerprint::V6(bytes) => bytes,
        }
    }

    /// From raw bytes, by their length. `None` for any other length.
    pub fn from_bytes(bytes: &[u8]) -> Option<Fingerprint> {
        match bytes.len() {
            20 => bytes.try_into().ok().map(Fingerprint::V4),
            32 => bytes.try_into().ok().map(Fingerprint::V6),
            _ => None,
        }
    }

    /// The short id a message names this key by in its encryption and signature packets.
    ///
    /// The last eight bytes of a version 4 fingerprint, and the first eight of a version 6 one
    /// (RFC 9580 §5.5.4.2, §5.5.4.3).
    pub fn key_id(&self) -> KeyId {
        let bytes = self.as_bytes();
        let window = match self {
            Fingerprint::V4(_) => &bytes[12..20],
            Fingerprint::V6(_) => &bytes[..8],
        };
        let mut id = [0u8; 8];
        id.copy_from_slice(window);
        KeyId(id)
    }
}

impl fmt::Display for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        hex(self.as_bytes(), f)
    }
}

impl FromStr for Fingerprint {
    type Err = String;

    /// Hex, in either case, with any spaces removed — GnuPG prints fingerprints in groups.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let bytes = unhex(text)?;
        Fingerprint::from_bytes(&bytes)
            .ok_or_else(|| format!("{text:?} is not a 40- or 64-digit fingerprint"))
    }
}

impl TryFrom<String> for Fingerprint {
    type Error = String;
    fn try_from(text: String) -> Result<Self, Self::Error> {
        text.parse()
    }
}

impl From<Fingerprint> for String {
    fn from(fingerprint: Fingerprint) -> String {
        fingerprint.to_string()
    }
}

/// The eight-byte id a message uses to say which key it was encrypted to or signed by.
///
/// Not an identity: two keys can share one. Used only to find candidates, which the signature
/// or the decryption then proves or rules out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct KeyId(pub [u8; 8]);

impl KeyId {
    /// The all-zero id an anonymous recipient is written with (RFC 9580 §5.1.1).
    pub fn is_wildcard(&self) -> bool {
        self.0 == [0; 8]
    }
}

impl fmt::Display for KeyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        hex(&self.0, f)
    }
}

impl FromStr for KeyId {
    type Err = String;
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let bytes = unhex(text)?;
        let id: [u8; 8] = bytes
            .try_into()
            .map_err(|_| format!("{text:?} is not a 16-digit key id"))?;
        Ok(KeyId(id))
    }
}

impl TryFrom<String> for KeyId {
    type Error = String;
    fn try_from(text: String) -> Result<Self, Self::Error> {
        text.parse()
    }
}

impl From<KeyId> for String {
    fn from(id: KeyId) -> String {
        id.to_string()
    }
}

fn hex(bytes: &[u8], f: &mut fmt::Formatter<'_>) -> fmt::Result {
    bytes.iter().try_for_each(|b| write!(f, "{b:02X}"))
}

fn unhex(text: &str) -> Result<Vec<u8>, String> {
    let digits: Vec<u8> = text.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    if !digits.len().is_multiple_of(2) {
        return Err(format!("{text:?} has an odd number of hex digits"));
    }
    digits
        .chunks(2)
        .map(|pair| {
            let pair = std::str::from_utf8(pair).map_err(|_| format!("{text:?} is not hex"))?;
            u8::from_str_radix(pair, 16).map_err(|_| format!("{text:?} is not hex"))
        })
        .collect()
}

/// How a public key came to be here. Kept because it is part of how much to believe it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeySource {
    /// Made on this machine for one of the user's identities.
    Generated,
    /// From a file the user imported.
    Imported,
    /// Fetched from the address's domain by Web Key Directory lookup.
    Wkd,
    /// From an `Autocrypt:` header on mail the key's owner sent.
    Autocrypt,
    /// From an `Autocrypt-Gossip:` header: a third party's copy, inside an encrypted message.
    Gossip,
}

impl KeySource {
    /// How much this source says about whose key it is, highest first: the user's own action,
    /// then the address's own domain, then the owner's own mail, then someone else's.
    ///
    /// The order a merge keeps the stronger of, and the order a recipient's keys are preferred
    /// in after the user's own verification.
    pub fn rank(self) -> u8 {
        match self {
            KeySource::Generated => 4,
            KeySource::Imported => 3,
            KeySource::Wkd => 2,
            KeySource::Autocrypt => 1,
            KeySource::Gossip => 0,
        }
    }
}

/// Whether the user has confirmed a key is its owner's — by comparing fingerprints out of band.
///
/// Nothing sets [`KeyTrust::Verified`] but the user. A key arriving by any route, however good,
/// arrives unverified.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyTrust {
    #[default]
    Unverified,
    Verified,
}

/// Whether the OS keyring holds this key's secret half.
///
/// Recorded beside the public key so "which of my identities can sign" is a store query rather
/// than a keyring round trip per identity. The secret itself is never in the store.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretHeld {
    #[default]
    Absent,
    Held,
}

/// One OpenPGP public key the client holds: the user's own, or a correspondent's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PgpKey {
    pub fingerprint: Fingerprint,
    /// The primary key's id and every subkey's, which is what a message names a key by.
    pub key_ids: Vec<KeyId>,
    /// The user ids as written in the key: `Name <address>`, usually.
    pub user_ids: Vec<String>,
    /// The addresses the key is for, lower-cased: those in its user ids, and the address an
    /// Autocrypt header or a WKD lookup delivered it for.
    pub emails: Vec<String>,
    /// The transferable public key, binary. Never a secret key.
    pub key: Vec<u8>,
    pub source: KeySource,
    pub first_seen: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
    pub trust: KeyTrust,
    pub secret: SecretHeld,
    /// When the primary key was made, from the key itself. `None` only for a key kept before
    /// this was recorded, until the store is next opened and it is read from the key's bytes.
    #[serde(default)]
    pub created: Option<DateTime<Utc>>,
    /// When the key stops being valid, from its newest self-signature. `None` when it does not
    /// expire — or, with `created` also `None`, when that is not recorded yet.
    #[serde(default)]
    pub expires: Option<DateTime<Utc>>,
}

impl PgpKey {
    /// This record with `newer` — the same key seen again — folded in.
    ///
    /// The seen window widens; the addresses and ids are unioned; a user's verification, and a
    /// secret half once held, are kept — neither is something a later sighting can take back.
    /// The key bytes and source come from whichever sighting has the stronger source
    /// ([`KeySource::rank`]), the newer on a tie, so a copy gossiped by a third party never
    /// replaces the one the owner published.
    ///
    /// # Panics
    /// When the fingerprints differ: that is two keys, and merging them is a caller's bug.
    pub fn merged(self, newer: PgpKey) -> PgpKey {
        assert_eq!(
            self.fingerprint, newer.fingerprint,
            "merging two different keys"
        );
        let newer_wins = newer.source.rank() > self.source.rank()
            || (newer.source.rank() == self.source.rank() && newer.last_seen >= self.last_seen);
        let union = |mut a: Vec<String>, b: Vec<String>| {
            for item in b {
                if !a.contains(&item) {
                    a.push(item);
                }
            }
            a
        };
        let mut key_ids = self.key_ids.clone();
        for id in &newer.key_ids {
            if !key_ids.contains(id) {
                key_ids.push(*id);
            }
        }
        // The dates go with the bytes they were read from, unless that copy never had them.
        let (dated, other) = if newer_wins {
            (&newer, &self)
        } else {
            (&self, &newer)
        };
        let (created, expires) = if dated.created.is_some() {
            (dated.created, dated.expires)
        } else {
            (other.created, other.expires)
        };
        let (key, source) = if newer_wins {
            (newer.key, newer.source)
        } else {
            (self.key, self.source)
        };
        PgpKey {
            fingerprint: self.fingerprint,
            key_ids,
            user_ids: union(self.user_ids, newer.user_ids),
            emails: union(self.emails, newer.emails),
            key,
            source,
            first_seen: self.first_seen.min(newer.first_seen),
            last_seen: self.last_seen.max(newer.last_seen),
            trust: self.trust.max_of(newer.trust),
            secret: self.secret.max_of(newer.secret),
            created,
            expires,
        }
    }

    /// Whether the key is for `address`, compared as a whole address without regard to case.
    pub fn is_for(&self, address: &str) -> bool {
        let address = address.trim();
        self.emails.iter().any(|e| e.eq_ignore_ascii_case(address))
    }
}

impl KeyTrust {
    fn max_of(self, other: KeyTrust) -> KeyTrust {
        if self == KeyTrust::Verified || other == KeyTrust::Verified {
            KeyTrust::Verified
        } else {
            KeyTrust::Unverified
        }
    }
}

impl SecretHeld {
    fn max_of(self, other: SecretHeld) -> SecretHeld {
        if self == SecretHeld::Held || other == SecretHeld::Held {
            SecretHeld::Held
        } else {
            SecretHeld::Absent
        }
    }
}

/// What a draft asks OpenPGP to do to it when it is sent.
///
/// Defaulted so drafts saved before OpenPGP existed load as [`OpenPgp::None`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenPgp {
    #[default]
    None,
    /// PGP/MIME signed (RFC 3156 §5): readable by anyone, provably from the sender.
    Sign,
    /// PGP/MIME encrypted (RFC 3156 §4) to every recipient and the sender.
    Encrypt,
    /// Both: signed, then encrypted, in one OpenPGP message (RFC 3156 §6.2).
    SignAndEncrypt,
}

impl OpenPgp {
    /// Whether the message is signed.
    pub fn signs(self) -> bool {
        matches!(self, OpenPgp::Sign | OpenPgp::SignAndEncrypt)
    }

    /// Whether the message is encrypted.
    pub fn encrypts(self) -> bool {
        matches!(self, OpenPgp::Encrypt | OpenPgp::SignAndEncrypt)
    }
}

/// How much of a message a signature covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Coverage {
    /// The whole body the reader shows.
    Whole,
    /// Only part of it: a signed part beside unsigned ones — a mailing list's footer, text
    /// around an inline signed block. The unsigned rest could say anything.
    Part,
}

/// What checking a message's signature found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "v", rename_all = "snake_case")]
pub enum Verification {
    /// The message carries no OpenPGP signature.
    NoSignature,
    /// Signed by the key named, and the signed bytes are the bytes here.
    Good {
        signer: Fingerprint,
        /// Whether the user has verified that key.
        trust: KeyTrust,
        coverage: Coverage,
    },
    /// A signature is there, and it does not match the bytes — or the key it claims.
    Bad { coverage: Coverage },
    /// Signed by a key this client does not hold, so nothing can be said either way.
    UnknownKey { issuer: KeyId, coverage: Coverage },
}

/// What opening a message's encryption found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "v", rename_all = "snake_case")]
pub enum Encryption {
    /// Not encrypted.
    NotEncrypted,
    /// Encrypted, and opened with the user's key; what is shown is the decrypted content.
    Decrypted,
    /// Encrypted to keys none of which the user holds the secret half of.
    CannotDecrypt { to: Vec<KeyId> },
    /// Encrypted to the user's key `key`, whose secret half is passphrase-protected and no
    /// passphrase (or the wrong one) was given.
    Locked { key: Fingerprint },
    /// Encrypted, to the user, and the content could not be read: damaged, tampered with, or in
    /// a form this client does not read.
    Unreadable { why: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(source: KeySource, seen: i64, bytes: &[u8]) -> PgpKey {
        let at = DateTime::from_timestamp(1_700_000_000 + seen, 0).unwrap();
        PgpKey {
            fingerprint: Fingerprint::V4([7; 20]),
            key_ids: vec![KeyId([7; 8])],
            user_ids: vec![format!("user {seen}")],
            emails: vec!["a@example.test".to_owned()],
            key: bytes.to_vec(),
            source,
            first_seen: at,
            last_seen: at,
            trust: KeyTrust::Unverified,
            secret: SecretHeld::Absent,
            created: Some(at),
            expires: None,
        }
    }

    #[test]
    fn fingerprints_read_back_from_what_they_print() {
        const CASES: &[&str] = &[
            "0123456789ABCDEF0123456789ABCDEF01234567",
            "0123 4567 89ab cdef 0123  4567 89AB CDEF 0123 4567",
        ];
        for text in CASES {
            let fp: Fingerprint = text.parse().unwrap();
            assert_eq!(
                fp.to_string(),
                "0123456789ABCDEF0123456789ABCDEF01234567",
                "{text}"
            );
        }
        assert!("0123".parse::<Fingerprint>().is_err());
        assert!(
            "zz23456789ABCDEF0123456789ABCDEF01234567"
                .parse::<Fingerprint>()
                .is_err()
        );
    }

    #[test]
    fn a_v4_key_id_is_the_fingerprints_tail() {
        let fp: Fingerprint = "0123456789ABCDEF0123456789ABCDEF01234567".parse().unwrap();
        assert_eq!(fp.key_id().to_string(), "89ABCDEF01234567");
    }

    #[test]
    fn a_later_gossiped_copy_does_not_replace_the_owners_key() {
        let owners = key(KeySource::Autocrypt, 0, b"owner");
        let merged = owners.clone().merged(key(KeySource::Gossip, 10, b"gossip"));
        assert_eq!(merged.key, b"owner");
        assert_eq!(merged.source, KeySource::Autocrypt);
        // The window still widens: it was seen again.
        assert_eq!(merged.last_seen, key(KeySource::Gossip, 10, b"").last_seen);
        assert_eq!(merged.user_ids, vec!["user 0", "user 10"]);
    }

    #[test]
    fn a_copy_with_no_dates_takes_them_from_the_other_and_the_bytes_decide_otherwise() {
        let mut undated = key(KeySource::Imported, 0, b"old");
        undated.created = None;
        let mut dated = key(KeySource::Gossip, 5, b"gossip");
        dated.expires = Some(dated.last_seen);
        let merged = undated.clone().merged(dated.clone());
        assert_eq!(merged.key, b"old");
        assert_eq!(
            (merged.created, merged.expires),
            (dated.created, dated.expires)
        );
        // A dated copy whose bytes win keeps its own dates, a `None` expiry included.
        let newer = key(KeySource::Imported, 9, b"new");
        let merged = dated.merged(newer.clone());
        assert_eq!((merged.created, merged.expires), (newer.created, None));
    }

    #[test]
    fn a_newer_copy_from_the_same_source_replaces_the_bytes() {
        let merged =
            key(KeySource::Autocrypt, 0, b"old").merged(key(KeySource::Autocrypt, 5, b"new"));
        assert_eq!(merged.key, b"new");
    }

    #[test]
    fn verification_and_a_held_secret_survive_any_merge() {
        let mut mine = key(KeySource::Generated, 0, b"mine");
        mine.trust = KeyTrust::Verified;
        mine.secret = SecretHeld::Held;
        let merged = mine.merged(key(KeySource::Wkd, 99, b"wkd"));
        assert_eq!(merged.trust, KeyTrust::Verified);
        assert_eq!(merged.secret, SecretHeld::Held);
        assert_eq!(merged.source, KeySource::Generated);
    }

    #[test]
    fn a_key_is_for_an_address_compared_whole() {
        let k = key(KeySource::Imported, 0, b"");
        assert!(k.is_for("A@Example.TEST"));
        assert!(!k.is_for("a@example.test.evil"));
        assert!(!k.is_for("xa@example.test"));
    }
}
