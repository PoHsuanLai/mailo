//! S/MIME as product vocabulary: which certificate, whose, whether the user trusts it, what a
//! draft asks for, and what reading a protected message found.
//!
//! No cryptography here. Certificates are parsed, checked and used in `mail-mime`, where the CMS
//! implementation lives; the store keeps [`SmimeCert`] rows, the OS keyring keeps the private
//! keys ([`crate::SecretPurpose::Smime`]), and these types are what passes between them.
//!
//! The shapes follow OpenPGP's ([`crate::pgp`]) so one badge can show either kind of
//! protection: [`Coverage`], [`KeyTrust`] and [`SecretHeld`] are shared, and where the two
//! differ — a certificate is named by its hash, not a key id, and its signature is believed only
//! when a chain of issuers vouches for it — S/MIME has its own type.

use crate::pgp::{Coverage, KeyTrust, SecretHeld};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// A certificate's SHA-256 fingerprint: the hash of its DER encoding, which is what names a
/// certificate everywhere in this client. Written as upper-case hex, as certificate viewers
/// print one, and stored that way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct CertFingerprint(pub [u8; 32]);

impl fmt::Display for CertFingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.iter().try_for_each(|b| write!(f, "{b:02X}"))
    }
}

impl FromStr for CertFingerprint {
    type Err = String;

    /// Hex in either case, with spaces or colons between the bytes ignored — viewers print it
    /// both ways.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let digits: Vec<u8> = text
            .bytes()
            .filter(|b| !b.is_ascii_whitespace() && *b != b':')
            .collect();
        if digits.len() != 64 {
            return Err(format!(
                "{text:?} is not a 64-digit certificate fingerprint"
            ));
        }
        let mut out = [0u8; 32];
        for (slot, pair) in out.iter_mut().zip(digits.chunks(2)) {
            let pair = std::str::from_utf8(pair).map_err(|_| format!("{text:?} is not hex"))?;
            *slot = u8::from_str_radix(pair, 16).map_err(|_| format!("{text:?} is not hex"))?;
        }
        Ok(CertFingerprint(out))
    }
}

impl TryFrom<String> for CertFingerprint {
    type Error = String;
    fn try_from(text: String) -> Result<Self, Self::Error> {
        text.parse()
    }
}

impl From<CertFingerprint> for String {
    fn from(fingerprint: CertFingerprint) -> String {
        fingerprint.to_string()
    }
}

/// How a certificate came to be here. Part of how much to prefer it for an address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CertSource {
    /// One of the user's own, from a PKCS#12 identity file with its private key.
    Identity,
    /// From a certificate file the user imported.
    Imported,
    /// From a signed message its owner sent, whose signature it verified.
    Received,
}

impl CertSource {
    /// How much this source says about whose certificate it is, highest first: the user's own
    /// identity, then the user's own action, then what arrived by mail.
    pub fn rank(self) -> u8 {
        match self {
            CertSource::Identity => 2,
            CertSource::Imported => 1,
            CertSource::Received => 0,
        }
    }
}

/// One X.509 certificate the client holds: the user's own, a correspondent's, or an issuer's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmimeCert {
    pub fingerprint: CertFingerprint,
    /// The subject's distinguished name, as RFC 4514 writes it.
    pub subject: String,
    /// The issuer's distinguished name, likewise.
    pub issuer: String,
    /// The serial number, upper-case hex.
    pub serial: String,
    /// The addresses it is for, lower-cased: its subject alternative name's `rfc822Name`s, or
    /// when it has none, its subject's `emailAddress`.
    pub emails: Vec<String>,
    pub not_before: DateTime<Utc>,
    pub not_after: DateTime<Utc>,
    /// The certificate, DER.
    pub der: Vec<u8>,
    /// The issuers' certificates that came with it (DER, nearest first): from the identity file
    /// or the signed message it arrived in. What a signature made with it carries, and what its
    /// chain is built from when a message does not carry one.
    pub chain: Vec<Vec<u8>>,
    pub source: CertSource,
    pub first_seen: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
    /// Whether the user has said to trust it: a certificate marked [`KeyTrust::Verified`] is a
    /// trust anchor of its own, beside the operating system's, for itself and for everything it
    /// issued.
    pub trust: KeyTrust,
    /// Whether the keyring holds its private key: whether it is one of the user's own.
    pub secret: SecretHeld,
}

impl SmimeCert {
    /// This record with `newer` — the same certificate seen again — folded in.
    ///
    /// The seen window widens, the addresses and issuers are unioned, and the user's trust and a
    /// private key once held are kept. The source is the stronger of the two
    /// ([`CertSource::rank`]). The certificate itself cannot differ: it is what the fingerprint
    /// is the hash of.
    ///
    /// # Panics
    /// When the fingerprints differ: that is two certificates, and merging them is a caller's bug.
    pub fn merged(self, newer: SmimeCert) -> SmimeCert {
        assert_eq!(
            self.fingerprint, newer.fingerprint,
            "merging two different certificates"
        );
        let mut emails = self.emails;
        for email in newer.emails {
            if !emails.contains(&email) {
                emails.push(email);
            }
        }
        let mut chain = self.chain;
        for issuer in newer.chain {
            if !chain.contains(&issuer) {
                chain.push(issuer);
            }
        }
        let source = if newer.source.rank() > self.source.rank() {
            newer.source
        } else {
            self.source
        };
        let trust = if self.trust == KeyTrust::Verified || newer.trust == KeyTrust::Verified {
            KeyTrust::Verified
        } else {
            KeyTrust::Unverified
        };
        let secret = if self.secret == SecretHeld::Held || newer.secret == SecretHeld::Held {
            SecretHeld::Held
        } else {
            SecretHeld::Absent
        };
        SmimeCert {
            emails,
            chain,
            source,
            first_seen: self.first_seen.min(newer.first_seen),
            last_seen: self.last_seen.max(newer.last_seen),
            trust,
            secret,
            ..self
        }
    }

    /// Whether the certificate is for `address`, compared as a whole address without regard to
    /// case.
    pub fn is_for(&self, address: &str) -> bool {
        let address = address.trim();
        self.emails.iter().any(|e| e.eq_ignore_ascii_case(address))
    }

    /// Whether it is valid at `at` by its own dates. Says nothing of who issued it.
    pub fn current_at(&self, at: DateTime<Utc>) -> bool {
        self.not_before <= at && at <= self.not_after
    }
}

/// What a draft asks S/MIME to do to it when it is sent.
///
/// Defaulted so drafts saved before S/MIME existed load as [`Smime::None`]. A draft asks this or
/// [`crate::OpenPgp`], never both; sending refuses a draft that asks both.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Smime {
    #[default]
    None,
    /// `multipart/signed` (RFC 8551 §3.5.3): readable by anyone, provably from the sender.
    Sign,
    /// `application/pkcs7-mime` enveloped-data (RFC 8551 §3.3) to every recipient and the sender.
    Encrypt,
    /// Signed, then the signed message encrypted (RFC 8551 §3.6).
    SignAndEncrypt,
}

impl Smime {
    /// Whether the message is signed.
    pub fn signs(self) -> bool {
        matches!(self, Smime::Sign | Smime::SignAndEncrypt)
    }

    /// Whether the message is encrypted.
    pub fn encrypts(self) -> bool {
        matches!(self, Smime::Encrypt | Smime::SignAndEncrypt)
    }
}

/// One reason a signature that matches its bytes is still not believed: something wrong with the
/// certificate that made it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "v", rename_all = "snake_case")]
pub enum CertProblem {
    /// No chain of issuers leads from it to a certificate the system or the user trusts.
    Untrusted,
    /// It, or an issuer in its chain, had expired at the moment it was checked for: the signing
    /// time the message states, or now when it states none.
    Expired { not_after: DateTime<Utc> },
    /// It, or an issuer in its chain, was not yet valid at that moment.
    NotYetValid { not_before: DateTime<Utc> },
    /// Its key usage or extended key usage does not allow signing mail (RFC 8550 §4.4).
    NotForEmail,
    /// It does not name the address the message is from.
    NotFrom { from: String },
}

/// Why a signature does not hold at all.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "v", rename_all = "snake_case")]
pub enum BadSignature {
    /// The content's digest is not the one the signature covers: the message was changed after
    /// it was signed.
    Altered,
    /// The signature does not verify with the key of the certificate it names: forged, or
    /// damaged.
    Forged,
    /// Made with an algorithm too weak to mean anything now: MD5, or SHA-1 after 2020.
    Weak { algorithm: String },
    /// Made with an algorithm this client does not check.
    Unsupported { algorithm: String },
    /// The signature could not be read.
    Malformed { why: String },
}

/// What checking a message's S/MIME signature found. Parallel to [`crate::Verification`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "v", rename_all = "snake_case")]
pub enum SmimeVerification {
    /// The message carries no S/MIME signature.
    NoSignature,
    /// Signed by the certificate named, the signed bytes are the bytes here, and the certificate
    /// chains to a trust anchor, was valid, is for mail, and names the sender.
    Good {
        signer: CertFingerprint,
        coverage: Coverage,
    },
    /// The signed bytes are the bytes here, and the certificate falls short: each problem named.
    Doubtful {
        signer: CertFingerprint,
        problems: Vec<CertProblem>,
        coverage: Coverage,
    },
    /// A signature is there and does not hold.
    Bad {
        why: BadSignature,
        coverage: Coverage,
    },
    /// Signed by a certificate neither the message nor this client holds, so nothing can be
    /// said either way.
    UnknownSigner { coverage: Coverage },
}

/// What opening a message's S/MIME encryption found. Parallel to [`crate::Encryption`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "v", rename_all = "snake_case")]
pub enum SmimeEncryption {
    /// Not encrypted.
    NotEncrypted,
    /// Encrypted, and opened with the user's key; what is shown is the decrypted content.
    Decrypted,
    /// Encrypted to certificates none of which is the user's, each named as the message names
    /// it: its issuer and serial number, or its subject key identifier.
    CannotDecrypt { to: Vec<String> },
    /// Encrypted to the user, and the content could not be read: damaged, tampered with, or with
    /// an algorithm this client refuses or does not read.
    Unreadable { why: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cert(source: CertSource, seen: i64) -> SmimeCert {
        let at = DateTime::from_timestamp(1_700_000_000 + seen, 0).unwrap();
        SmimeCert {
            fingerprint: CertFingerprint([9; 32]),
            subject: "CN=A".to_owned(),
            issuer: "CN=CA".to_owned(),
            serial: "01".to_owned(),
            emails: vec![format!("a{seen}@example.test")],
            not_before: at,
            not_after: at,
            der: vec![0x30],
            chain: vec![vec![seen as u8]],
            source,
            first_seen: at,
            last_seen: at,
            trust: KeyTrust::Unverified,
            secret: SecretHeld::Absent,
        }
    }

    #[test]
    fn a_fingerprint_reads_back_from_what_it_prints_with_or_without_colons() {
        let hex = "00112233445566778899AABBCCDDEEFF00112233445566778899AABBCCDDEEFF";
        let fp: CertFingerprint = hex.parse().unwrap();
        assert_eq!(fp.to_string(), hex);
        let colons = hex
            .as_bytes()
            .chunks(2)
            .map(|c| std::str::from_utf8(c).unwrap().to_ascii_lowercase())
            .collect::<Vec<_>>()
            .join(":");
        assert_eq!(colons.parse::<CertFingerprint>().unwrap(), fp);
        assert!("0011".parse::<CertFingerprint>().is_err());
    }

    #[test]
    fn a_received_copy_never_downgrades_the_users_own_identity() {
        let mut own = cert(CertSource::Identity, 0);
        own.secret = SecretHeld::Held;
        own.trust = KeyTrust::Verified;
        let merged = own.merged(cert(CertSource::Received, 10));
        assert_eq!(merged.source, CertSource::Identity);
        assert_eq!(merged.secret, SecretHeld::Held);
        assert_eq!(merged.trust, KeyTrust::Verified);
        assert_eq!(merged.emails, vec!["a0@example.test", "a10@example.test"]);
        assert_eq!(merged.chain, vec![vec![0], vec![10]]);
        assert_eq!(merged.last_seen, cert(CertSource::Received, 10).last_seen);
    }

    #[test]
    fn a_certificate_is_for_an_address_compared_whole() {
        let c = cert(CertSource::Imported, 0);
        assert!(c.is_for("A0@Example.TEST"));
        assert!(!c.is_for("a0@example.test.evil"));
        assert!(!c.is_for("xa0@example.test"));
    }
}
