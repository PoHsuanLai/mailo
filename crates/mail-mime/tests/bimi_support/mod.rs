//! Throwaway Verified Mark Certificates for BIMI tests: a root, an intermediate under it, and
//! mark certificates they issue, made here from seeded generators. No real certificate or key
//! is involved, and nothing is read from disk.
//!
//! The logotype extension is written by hand from RFC 3709 §4.1's module (implicit tags), the
//! shape a mark certificate carries its logo in. Shared by `mail-mime`'s tests and
//! `mail-runtime`'s (which include this file by path).

#![allow(dead_code)]

use chrono::{DateTime, TimeZone, Utc};
use der::asn1::Ia5String;
use der::{Encode, Length, Writer};
use mail_mime::smime::Cert;
use rand::SeedableRng;
use rand::rngs::StdRng;
use sha2::Digest;
use spki::SubjectPublicKeyInfoOwned;
use std::str::FromStr;
use std::time::Duration;
use x509_cert::builder::{Builder, CertificateBuilder, Profile};
use x509_cert::ext::AsExtension;
use x509_cert::ext::pkix::name::GeneralName;
use x509_cert::ext::pkix::{ExtendedKeyUsage, SubjectAltName};
use x509_cert::name::Name;
use x509_cert::serial_number::SerialNumber;
use x509_cert::time::{Time, Validity};

pub const MARK_PURPOSE: const_oid::ObjectIdentifier =
    const_oid::ObjectIdentifier::new_unwrap("1.3.6.1.5.5.7.3.31");
pub const SERVER_AUTH: const_oid::ObjectIdentifier =
    const_oid::ObjectIdentifier::new_unwrap("1.3.6.1.5.5.7.3.1");

/// A small SVG Tiny PS logo: a filled square and a circle, no text, nothing external.
pub const LOGO: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" version="1.2" baseProfile="tiny-ps" viewBox="0 0 64 64"><title>Brand</title><rect width="64" height="64" fill="#1a73e8"/><circle cx="32" cy="32" r="16" fill="#ffffff"/></svg>"##;

pub fn at(y: i32, m: u32, d: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(y, m, d, 12, 0, 0).unwrap()
}

/// The instant the tests check certificates at.
pub fn now() -> DateTime<Utc> {
    at(2026, 9, 27)
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

fn key(seed: u64) -> p256::ecdsa::SigningKey {
    p256::ecdsa::SigningKey::random(&mut StdRng::seed_from_u64(seed))
}

fn cert_of(built: x509_cert::Certificate) -> Cert {
    Cert::from_der(&built.to_der().unwrap()).unwrap()
}

/// A mark verifying authority: a root, and the intermediate that issues marks.
pub struct Authority {
    pub root: Cert,
    pub intermediate: Cert,
    intermediate_key: p256::ecdsa::SigningKey,
    intermediate_name: Name,
}

/// An authority under the label `label`, its keys from `seed`.
pub fn authority(label: &str, seed: u64) -> Authority {
    let root_key = key(seed);
    let root_name = Name::from_str(&format!("CN={label} Mark Root,O=Example")).unwrap();
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
    let inter_key = key(seed + 1);
    let inter_name = Name::from_str(&format!("CN={label} Mark CA,O=Example")).unwrap();
    let inter_spki = SubjectPublicKeyInfoOwned::from_key(*inter_key.verifying_key()).unwrap();
    let intermediate = CertificateBuilder::new(
        Profile::SubCA {
            issuer: root_name,
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
    Authority {
        root: cert_of(root),
        intermediate: cert_of(intermediate),
        intermediate_key: inter_key,
        intermediate_name: inter_name,
    }
}

/// What a mark certificate says.
pub struct Mark<'a> {
    /// Its subject alternative names.
    pub domains: &'a [&'a str],
    /// The logo it carries, as the `data:` URL holds it.
    pub logo: &'a [u8],
    /// The SHA-256 it states for the logo; `None` for the true one.
    pub stated_hash: Option<Vec<u8>>,
    pub purposes: Vec<const_oid::ObjectIdentifier>,
    pub valid: (DateTime<Utc>, DateTime<Utc>),
}

impl<'a> Mark<'a> {
    /// A mark for `domains` carrying `logo`, valid 2026–2027.
    pub fn new(domains: &'a [&'a str], logo: &'a [u8]) -> Mark<'a> {
        Mark {
            domains,
            logo,
            stated_hash: None,
            purposes: vec![MARK_PURPOSE],
            valid: (at(2026, 1, 1), at(2027, 6, 1)),
        }
    }
}

/// The PEM file a BIMI record's `a=` names: the mark certificate, then the intermediate.
pub fn pem(by: &Authority, mark: &Mark<'_>) -> Vec<u8> {
    let subject_key = key(900);
    let spki = SubjectPublicKeyInfoOwned::from_key(*subject_key.verifying_key()).unwrap();
    let mut builder = CertificateBuilder::new(
        Profile::Leaf {
            issuer: by.intermediate_name.clone(),
            enable_key_agreement: false,
            enable_key_encipherment: false,
        },
        SerialNumber::from(77u32),
        validity(mark.valid.0, mark.valid.1),
        Name::from_str("CN=Brand,O=Brand Example").unwrap(),
        spki,
        &by.intermediate_key,
    )
    .unwrap();
    let names = mark
        .domains
        .iter()
        .map(|d| GeneralName::DnsName(Ia5String::new(d).unwrap()))
        .collect();
    builder.add_extension(&SubjectAltName(names)).unwrap();
    builder
        .add_extension(&ExtendedKeyUsage(mark.purposes.clone()))
        .unwrap();
    let hash = mark
        .stated_hash
        .clone()
        .unwrap_or_else(|| sha2::Sha256::digest(mark.logo).to_vec());
    builder
        .add_extension(&Logotype(logotype(mark.logo, &hash)))
        .unwrap();
    let leaf = cert_of(builder.build::<p256::ecdsa::DerSignature>().unwrap());
    format!("{}{}", leaf.pem(), by.intermediate.pem()).into_bytes()
}

/// The logotype extension's value, DER, already encoded.
struct Logotype(Vec<u8>);

impl const_oid::AssociatedOid for Logotype {
    const OID: const_oid::ObjectIdentifier =
        const_oid::ObjectIdentifier::new_unwrap("1.3.6.1.5.5.7.1.12");
}

impl Encode for Logotype {
    fn encoded_len(&self) -> der::Result<Length> {
        Length::try_from(self.0.len())
    }

    fn encode(&self, writer: &mut impl Writer) -> der::Result<()> {
        writer.write(&self.0)
    }
}

impl AsExtension for Logotype {
    fn critical(&self, _: &Name, _: &[x509_cert::ext::Extension]) -> bool {
        false
    }
}

fn element(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    let len = content.len();
    if len < 0x80 {
        out.push(len as u8);
    } else if len < 0x100 {
        out.extend_from_slice(&[0x81, len as u8]);
    } else {
        out.extend_from_slice(&[0x82, (len >> 8) as u8, len as u8]);
    }
    out.extend_from_slice(content);
    out
}

/// RFC 3709's LogotypeExtn with one subject logo: `logo` as a base64 `data:` URL, stated with
/// `sha256`.
fn logotype(logo: &[u8], sha256: &[u8]) -> Vec<u8> {
    use base64::Engine as _;
    let uri = format!(
        "data:image/svg+xml;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(logo)
    );
    let sha256_oid = [0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x01];
    let alg = element(
        0x30,
        &[element(0x06, &sha256_oid), vec![0x05, 0x00]].concat(),
    );
    let hash = element(0x30, &[alg, element(0x04, sha256)].concat());
    let details = element(
        0x30,
        &[
            element(0x16, b"image/svg+xml"),
            element(0x30, &hash),
            element(0x30, &element(0x16, uri.as_bytes())),
        ]
        .concat(),
    );
    let image = element(0x30, &details);
    let images = element(0x30, &image);
    let direct = element(0xa0, &images);
    let subject = element(0xa2, &direct);
    element(0x30, &subject)
}
