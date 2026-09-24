//! A throwaway public key infrastructure for S/MIME tests: a root, an intermediate, and people
//! with certificates issued under them, all made here from seeded generators. No real
//! certificate or key is involved, and nothing is read from disk.
//!
//! Shared by `mail-mime`'s tests and `mail-app`'s (which include this file by path), so both
//! build their fixtures the same way.

#![allow(dead_code)]

use chrono::{DateTime, TimeZone, Utc};
use der::asn1::{Ia5String, OctetString, SetOfVec};
use der::{Any, Decode, Encode};
use mail_mime::smime::{Cert, Identity, PrivateKey};
use rand::SeedableRng;
use rand::rngs::StdRng;
use spki::{EncodePublicKey, SubjectPublicKeyInfoOwned};
use std::str::FromStr;
use std::sync::OnceLock;
use std::time::Duration;
use x509_cert::builder::{Builder, CertificateBuilder, Profile};
use x509_cert::ext::pkix::name::GeneralName;
use x509_cert::ext::pkix::{ExtendedKeyUsage, SubjectAltName};
use x509_cert::name::Name;
use x509_cert::serial_number::SerialNumber;
use x509_cert::time::{Time, Validity};

pub const EMAIL_PROTECTION: const_oid::ObjectIdentifier =
    const_oid::ObjectIdentifier::new_unwrap("1.3.6.1.5.5.7.3.4");
pub const SERVER_AUTH: const_oid::ObjectIdentifier =
    const_oid::ObjectIdentifier::new_unwrap("1.3.6.1.5.5.7.3.1");

pub fn at(y: i32, m: u32, d: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(y, m, d, 12, 0, 0).unwrap()
}

/// The instant tests read and send mail at.
pub fn now() -> DateTime<Utc> {
    at(2026, 9, 24)
}

fn time(when: DateTime<Utc>) -> Time {
    Time::UtcTime(
        der::asn1::UtcTime::from_unix_duration(Duration::from_secs(when.timestamp() as u64))
            .unwrap(),
    )
}

fn validity(from: DateTime<Utc>, to: DateTime<Utc>) -> Validity {
    Validity {
        not_before: time(from),
        not_after: time(to),
    }
}

fn cert_of(built: x509_cert::Certificate) -> Cert {
    Cert::from_der(&built.to_der().unwrap()).unwrap()
}

/// A certification authority: its certificate and its signing key.
pub struct Authority {
    pub cert: Cert,
    pub key: p256::ecdsa::SigningKey,
    pub name: Name,
}

/// The root and the intermediate under it that issues people's certificates.
pub struct Pki {
    pub root: Authority,
    pub intermediate: Authority,
}

fn ec_key(seed: u64) -> p256::ecdsa::SigningKey {
    p256::ecdsa::SigningKey::random(&mut StdRng::seed_from_u64(seed))
}

/// A root and intermediate, made once per test binary.
pub fn pki() -> &'static Pki {
    static PKI: OnceLock<Pki> = OnceLock::new();
    PKI.get_or_init(|| authorities("Example Test", 1))
}

/// Another root and intermediate, which nothing trusts.
pub fn stranger_pki() -> &'static Pki {
    static PKI: OnceLock<Pki> = OnceLock::new();
    PKI.get_or_init(|| authorities("Unknown Test", 50))
}

fn authorities(label: &str, seed: u64) -> Pki {
    let root_key = ec_key(seed);
    let root_name = Name::from_str(&format!("CN={label} Root,O=Example")).unwrap();
    let root_spki = SubjectPublicKeyInfoOwned::from_key(*root_key.verifying_key()).unwrap();
    let root = CertificateBuilder::new(
        Profile::Root,
        SerialNumber::from(1u32),
        validity(at(2020, 1, 1), at(2040, 1, 1)),
        root_name.clone(),
        root_spki,
        &root_key,
    )
    .unwrap()
    .build::<p256::ecdsa::DerSignature>()
    .unwrap();

    let inter_key = ec_key(seed + 1);
    let inter_name = Name::from_str(&format!("CN={label} Mail CA,O=Example")).unwrap();
    let inter_spki = SubjectPublicKeyInfoOwned::from_key(*inter_key.verifying_key()).unwrap();
    let intermediate = CertificateBuilder::new(
        Profile::SubCA {
            issuer: root_name.clone(),
            path_len_constraint: Some(0),
        },
        SerialNumber::from(2u32),
        validity(at(2020, 1, 1), at(2040, 1, 1)),
        inter_name.clone(),
        inter_spki,
        &root_key,
    )
    .unwrap()
    .build::<p256::ecdsa::DerSignature>()
    .unwrap();

    Pki {
        root: Authority {
            cert: cert_of(root),
            key: root_key,
            name: root_name,
        },
        intermediate: Authority {
            cert: cert_of(intermediate),
            key: inter_key,
            name: inter_name,
        },
    }
}

/// An RSA key, made once per `seed` per test binary.
pub fn rsa_key(seed: u64) -> rsa::RsaPrivateKey {
    static KEYS: OnceLock<std::sync::Mutex<Vec<(u64, rsa::RsaPrivateKey)>>> = OnceLock::new();
    let keys = KEYS.get_or_init(Default::default);
    let mut keys = keys.lock().unwrap();
    if let Some((_, key)) = keys.iter().find(|(s, _)| *s == seed) {
        return key.clone();
    }
    let key = rsa::RsaPrivateKey::new(&mut StdRng::seed_from_u64(seed), 2048).unwrap();
    keys.push((seed, key.clone()));
    key
}

/// What a person's certificate says.
pub struct Person<'a> {
    pub name: &'a str,
    /// The addresses in its subject alternative name.
    pub emails: &'a [&'a str],
    pub valid: (DateTime<Utc>, DateTime<Utc>),
    /// Its extended key usage; `None` for none stated.
    pub purposes: Option<Vec<const_oid::ObjectIdentifier>>,
    pub serial: u32,
    pub key_seed: u64,
}

impl<'a> Person<'a> {
    /// A person with a certificate for `email` valid 2026–2028, for mail.
    pub fn new(name: &'a str, emails: &'a [&'a str], serial: u32, key_seed: u64) -> Person<'a> {
        Person {
            name,
            emails,
            valid: (at(2026, 1, 1), at(2028, 1, 1)),
            purposes: Some(vec![EMAIL_PROTECTION]),
            serial,
            key_seed,
        }
    }
}

/// `person`'s identity: an RSA key and a certificate issued by `by`, with the intermediate and
/// root as its chain.
pub fn identity(pki: &Pki, person: &Person<'_>) -> Identity {
    let key = rsa_key(person.key_seed);
    let spki = SubjectPublicKeyInfoOwned::from_key(key.to_public_key()).unwrap();
    let issuer = &pki.intermediate;
    let mut builder = CertificateBuilder::new(
        Profile::Leaf {
            issuer: issuer.name.clone(),
            enable_key_agreement: false,
            enable_key_encipherment: true,
        },
        SerialNumber::from(person.serial),
        validity(person.valid.0, person.valid.1),
        Name::from_str(&format!("CN={}", person.name)).unwrap(),
        spki,
        &issuer.key,
    )
    .unwrap();
    let names: Vec<GeneralName> = person
        .emails
        .iter()
        .map(|e| GeneralName::Rfc822Name(Ia5String::new(e).unwrap()))
        .collect();
    if !names.is_empty() {
        builder.add_extension(&SubjectAltName(names)).unwrap();
    }
    if let Some(purposes) = &person.purposes {
        builder
            .add_extension(&ExtendedKeyUsage(purposes.clone()))
            .unwrap();
    }
    let cert = cert_of(builder.build::<p256::ecdsa::DerSignature>().unwrap());
    Identity::new(
        PrivateKey::Rsa(Box::new(key)),
        cert,
        vec![pki.intermediate.cert.clone(), pki.root.cert.clone()],
    )
    .unwrap()
}

/// A plain message from `from` to `to`, dated `date`, whose body ends its lines with trailing
/// spaces — which a signature must survive unchanged.
pub fn message(from: &str, to: &str, date: &str) -> Vec<u8> {
    format!(
        "From: Sender <{from}>\r\nTo: {to}\r\nSubject: The plan\r\nDate: {date}\r\n\
         Message-ID: <plan@example.test>\r\nMIME-Version: 1.0\r\n\
         Content-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: 7bit\r\n\r\n\
         Meet at noon.  \r\nBring the map. \r\n"
    )
    .into_bytes()
}

pub const DATE: &str = "Thu, 24 Sep 2026 10:00:00 +0000";

/// A seeded generator, so a failure reproduces.
pub fn rng(seed: u64) -> StdRng {
    StdRng::seed_from_u64(seed)
}

/// `der` re-encoded the way streaming BER encoders write it: every constructed value with an
/// indefinite length, every OCTET STRING longer than 16 bytes cut into a constructed string of
/// 16-byte pieces, and `[0] IMPLICIT` encrypted content as constructed pieces too.
pub fn as_streaming_ber(der: &[u8]) -> Vec<u8> {
    fn walk(bytes: &[u8], out: &mut Vec<u8>) -> usize {
        let tag = bytes[0];
        let (len, header) = if bytes[1] < 0x80 {
            (bytes[1] as usize, 2)
        } else {
            let n = (bytes[1] & 0x7F) as usize;
            let mut len = 0usize;
            for b in &bytes[2..2 + n] {
                len = (len << 8) | *b as usize;
            }
            (len, 2 + n)
        };
        let content = &bytes[header..header + len];
        let piece = |out: &mut Vec<u8>, tag: u8, content: &[u8]| {
            out.push(tag | 0x20);
            out.push(0x80);
            for chunk in content.chunks(16) {
                out.push(0x04);
                out.push(chunk.len() as u8);
                out.extend_from_slice(chunk);
            }
            out.extend_from_slice(&[0, 0]);
        };
        if tag == 0x04 && len > 16 {
            piece(out, 0x04, content);
        } else if tag == 0x80 && len > 16 {
            // [0] IMPLICIT OCTET STRING: EncryptedContentInfo's encryptedContent.
            piece(out, 0x80, content);
        } else if tag & 0x20 != 0 {
            out.push(tag);
            out.push(0x80);
            let mut at = 0;
            while at < content.len() {
                at += walk(&content[at..], out);
            }
            out.extend_from_slice(&[0, 0]);
        } else {
            out.extend_from_slice(&bytes[..header + len]);
        }
        header + len
    }
    let mut out = Vec::new();
    walk(der, &mut out);
    out
}

/// The base64 body of a message's single S/MIME part, decoded: its CMS object.
pub fn cms_of(sealed: &[u8]) -> Vec<u8> {
    use base64::Engine;
    let text = String::from_utf8_lossy(sealed);
    let body = text.split("\r\n\r\n").nth(1).unwrap();
    let joined: String = body.split_whitespace().collect();
    base64::engine::general_purpose::STANDARD
        .decode(joined)
        .unwrap()
}

/// `head` (the outer header, ending in the blank line's first CRLF) with an
/// `application/pkcs7-mime` part holding `der`.
pub fn pkcs7_message(head: &str, smime_type: &str, der: &[u8]) -> Vec<u8> {
    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(der);
    let mut out = format!(
        "{head}MIME-Version: 1.0\r\nContent-Type: application/pkcs7-mime; \
         smime-type={smime_type}; name=smime.p7m\r\nContent-Transfer-Encoding: base64\r\n\r\n"
    )
    .into_bytes();
    for line in encoded.as_bytes().chunks(76) {
        out.extend_from_slice(line);
        out.extend_from_slice(b"\r\n");
    }
    out
}

/// A CMS attribute with one value.
pub fn attribute(oid: &str, value: &impl Encode) -> x509_cert::attr::Attribute {
    let mut values = SetOfVec::new();
    values
        .insert(Any::from_der(&value.to_der().unwrap()).unwrap())
        .unwrap();
    x509_cert::attr::Attribute {
        oid: const_oid::ObjectIdentifier::new_unwrap(oid),
        values,
    }
}

/// An OCTET STRING.
pub fn octets(bytes: &[u8]) -> OctetString {
    OctetString::new(bytes.to_vec()).unwrap()
}

/// The issuer and serial number of `cert`, as CMS names a certificate.
pub fn issuer_and_serial(cert: &Cert) -> cms::cert::IssuerAndSerialNumber {
    let parsed = x509_cert::Certificate::from_der(cert.der()).unwrap();
    cms::cert::IssuerAndSerialNumber {
        issuer: parsed.tbs_certificate.issuer,
        serial_number: parsed.tbs_certificate.serial_number,
    }
}

/// `public`'s SPKI, DER.
pub fn spki_der(public: &rsa::RsaPublicKey) -> Vec<u8> {
    public.to_public_key_der().unwrap().as_bytes().to_vec()
}
