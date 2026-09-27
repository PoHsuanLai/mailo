//! The evidence behind a brand logo: a Verified Mark Certificate, and the logo it carries.
//!
//! A BIMI record's `a=` names a PEM file: the mark certificate first, then the issuers it chains
//! through. What makes the logo believable is a certification authority that checked the mark
//! and put it in the certificate, so all of this must hold before a logo is shown:
//!
//! - the chain reaches one of the given trust anchors, every link signed by the next and within
//!   its dates (the S/MIME path check, `smime::chain`; no revocation is asked);
//! - the certificate is for marks: its extended key usage names
//!   `id-kp-BrandIndicatorforMessageIdentification` (1.3.6.1.5.5.7.3.31);
//! - it names the domain: a subject alternative name `dNSName` equal to it, whole;
//! - it carries the logo (RFC 3709 logotype extension, the subject logo), as a `data:` URL of
//!   `image/svg+xml`, and one of the hashes the certificate states for it, SHA-256 or wider,
//!   matches the bytes.
//!
//! The logo returned is the certificate's. The caller also fetched the record's `l=`, and shows
//! nothing unless the two are the same image ([`same_logo`]).

use crate::smime::Cert;
use crate::smime::chain_to_anchor;
use base64::Engine as _;
use chrono::{DateTime, Utc};
use const_oid::ObjectIdentifier as Oid;
use sha2::Digest as _;
use std::io::Read as _;
use x509_cert::ext::pkix::name::GeneralName;
use x509_cert::ext::pkix::{ExtendedKeyUsage, SubjectAltName};

/// `id-kp-BrandIndicatorforMessageIdentification`.
pub const MARK_PURPOSE: Oid = Oid::new_unwrap("1.3.6.1.5.5.7.3.31");
/// `id-pe-logotype` (RFC 3709 §4.1).
pub const LOGOTYPE: Oid = Oid::new_unwrap("1.3.6.1.5.5.7.1.12");

const SHA256: Oid = Oid::new_unwrap("2.16.840.1.101.3.4.2.1");
const SHA384: Oid = Oid::new_unwrap("2.16.840.1.101.3.4.2.2");
const SHA512: Oid = Oid::new_unwrap("2.16.840.1.101.3.4.2.3");

/// The largest SVG this reads, compressed or not. The draft asks for logos of at most 32 KiB.
pub const MAX_SVG: usize = 32 * 1024;

/// Why a mark certificate does not vouch for a logo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarkProblem {
    /// Not a PEM certificate, or no certificate in it.
    Unreadable,
    /// Outside its dates, or an issuer is.
    Dated,
    /// Its extended key usage does not name marks.
    NotForMarks,
    /// No subject alternative name is the domain.
    NotForDomain,
    /// No chain from it reaches a trust anchor.
    Untrusted,
    /// It carries no SVG logo this reader can find.
    NoLogo,
    /// No hash it states for the logo is one this reader accepts, or none matches.
    LogoHash,
}

/// The SVG `pem`'s mark certificate vouches for, for mail from `domain` at `at`, decompressed.
pub fn verified_logo(
    pem: &[u8],
    domain: &str,
    anchors: &[Cert],
    at: DateTime<Utc>,
) -> Result<Vec<u8>, MarkProblem> {
    let certs = crate::smime::read_certs(pem).map_err(|_| MarkProblem::Unreadable)?;
    let (leaf, pool) = certs.split_first().ok_or(MarkProblem::Unreadable)?;
    let within = |cert: &Cert| cert.not_before() <= at && at <= cert.not_after();
    if !within(leaf) {
        return Err(MarkProblem::Dated);
    }
    if !for_marks(leaf) {
        return Err(MarkProblem::NotForMarks);
    }
    if !names(leaf, domain) {
        return Err(MarkProblem::NotForDomain);
    }
    let issuers = chain_to_anchor(leaf, pool, anchors).ok_or(MarkProblem::Untrusted)?;
    if !issuers.into_iter().all(within) {
        return Err(MarkProblem::Dated);
    }
    let extension = leaf
        .inner
        .tbs_certificate
        .extensions
        .as_deref()
        .unwrap_or_default()
        .iter()
        .find(|ext| ext.extn_id == LOGOTYPE)
        .ok_or(MarkProblem::NoLogo)?;
    let logo = subject_logo(extension.extn_value.as_bytes()).ok_or(MarkProblem::NoLogo)?;
    let data = svg_data_url(&logo.uri).ok_or(MarkProblem::NoLogo)?;
    let matched = logo.hashes.iter().any(|(alg, value)| {
        let computed: Vec<u8> = if *alg == SHA256 {
            sha2::Sha256::digest(&data).to_vec()
        } else if *alg == SHA384 {
            sha2::Sha384::digest(&data).to_vec()
        } else if *alg == SHA512 {
            sha2::Sha512::digest(&data).to_vec()
        } else {
            return false;
        };
        computed == *value
    });
    if !matched {
        return Err(MarkProblem::LogoHash);
    }
    svg_bytes(&data).ok_or(MarkProblem::NoLogo)
}

/// Whether the logo fetched from the record's `l=` is the certificate's: the same SVG once
/// either is decompressed.
pub fn same_logo(fetched: &[u8], certified: &[u8]) -> bool {
    svg_bytes(fetched).is_some_and(|svg| svg == certified)
}

/// `bytes` as SVG text: gunzipped when gzipped (`.svgz`, which is how a certificate usually
/// holds it), at most [`MAX_SVG`] either way.
pub fn svg_bytes(bytes: &[u8]) -> Option<Vec<u8>> {
    if bytes.len() > MAX_SVG {
        return None;
    }
    if !bytes.starts_with(&[0x1f, 0x8b]) {
        return Some(bytes.to_vec());
    }
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(bytes)
        .take(MAX_SVG as u64 + 1)
        .read_to_end(&mut out)
        .ok()?;
    (out.len() <= MAX_SVG).then_some(out)
}

fn for_marks(cert: &Cert) -> bool {
    matches!(
        cert.inner.tbs_certificate.get::<ExtendedKeyUsage>(),
        Ok(Some((_, eku))) if eku.0.contains(&MARK_PURPOSE)
    )
}

fn names(cert: &Cert, domain: &str) -> bool {
    let domain = domain.trim_end_matches('.');
    match cert.inner.tbs_certificate.get::<SubjectAltName>() {
        Ok(Some((_, san))) => san.0.iter().any(|name| {
            matches!(name, GeneralName::DnsName(dns)
                if dns.as_str().trim_end_matches('.').eq_ignore_ascii_case(domain))
        }),
        _ => false,
    }
}

/// The bytes of a `data:image/svg+xml;base64,…` URL. Any other URL is not followed: a
/// certificate pointing elsewhere for its logo is not carrying it.
fn svg_data_url(uri: &str) -> Option<Vec<u8>> {
    let rest = uri.strip_prefix("data:")?;
    let (head, body) = rest.split_once(',')?;
    let mut params = head.split(';').map(str::trim);
    if !params.next()?.eq_ignore_ascii_case("image/svg+xml") {
        return None;
    }
    if !params.any(|p| p.eq_ignore_ascii_case("base64")) {
        return None;
    }
    // Four characters of base64 carry three bytes; a larger body cannot hold an allowed SVG.
    if body.len() > MAX_SVG / 3 * 4 + 8 {
        return None;
    }
    base64::engine::general_purpose::STANDARD
        .decode(body.trim())
        .ok()
}

/// The subject logo's first image: its URL and its stated hashes.
#[derive(Debug, PartialEq, Eq)]
struct Logo {
    uri: String,
    hashes: Vec<(Oid, Vec<u8>)>,
}

/// RFC 3709 §4.1, read as far as the subject logo's first image:
///
/// ```text
/// LogotypeExtn ::= SEQUENCE { communityLogos [0] EXPLICIT …, issuerLogo [1] EXPLICIT …,
///                             subjectLogo [2] EXPLICIT LogotypeInfo OPTIONAL, … }
/// LogotypeInfo ::= CHOICE { direct [0] LogotypeData, indirect [1] LogotypeReference }
/// LogotypeData ::= SEQUENCE { image SEQUENCE OF LogotypeImage OPTIONAL, … }
/// LogotypeImage ::= SEQUENCE { imageDetails LogotypeDetails, imageInfo … OPTIONAL }
/// LogotypeDetails ::= SEQUENCE { mediaType IA5String,
///                                logotypeHash SEQUENCE OF HashAlgAndValue,
///                                logotypeURI SEQUENCE OF IA5String }
/// HashAlgAndValue ::= SEQUENCE { hashAlg AlgorithmIdentifier, hashValue OCTET STRING }
/// ```
///
/// The tags are implicit in RFC 3709's module, so `direct` is `[0]` constructed around
/// LogotypeData's fields.
fn subject_logo(value: &[u8]) -> Option<Logo> {
    let (tag, extn, _) = tlv(value)?;
    if tag != SEQUENCE {
        return None;
    }
    let subject = children(extn)?.into_iter().find(|(tag, _)| *tag == 0xa2)?.1;
    let (tag, data, _) = tlv(subject)?;
    if tag != 0xa0 {
        return None;
    }
    let (tag, images, _) = tlv(data)?;
    if tag != SEQUENCE {
        return None;
    }
    let (tag, image, _) = tlv(images)?;
    if tag != SEQUENCE {
        return None;
    }
    let (tag, details, _) = tlv(image)?;
    if tag != SEQUENCE {
        return None;
    }
    let parts = children(details)?;
    let [(IA5, media), (SEQUENCE, hashes), (SEQUENCE, uris)] = parts.as_slice() else {
        return None;
    };
    if !std::str::from_utf8(media)
        .ok()?
        .eq_ignore_ascii_case("image/svg+xml")
    {
        return None;
    }
    let hashes = children(hashes)?
        .into_iter()
        .filter(|(tag, _)| *tag == SEQUENCE)
        .filter_map(|(_, pair)| {
            let fields = children(pair)?;
            let [(SEQUENCE, alg), (OCTET_STRING, value)] = fields.as_slice() else {
                return None;
            };
            let (tag, oid, _) = tlv(alg)?;
            (tag == OID).then_some(())?;
            Some((Oid::from_bytes(oid).ok()?, value.to_vec()))
        })
        .collect();
    let uri = children(uris)?
        .into_iter()
        .find(|(tag, _)| *tag == IA5)
        .and_then(|(_, uri)| std::str::from_utf8(uri).ok())?
        .to_owned();
    Some(Logo { uri, hashes })
}

const SEQUENCE: u8 = 0x30;
const IA5: u8 = 0x16;
const OCTET_STRING: u8 = 0x04;
const OID: u8 = 0x06;

/// One DER element: its tag, its contents, and what follows it. Definite lengths only (DER),
/// single-byte tags only (all RFC 3709 uses).
fn tlv(bytes: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    let (&tag, rest) = bytes.split_first()?;
    if tag & 0x1f == 0x1f {
        return None;
    }
    let (&first, rest) = rest.split_first()?;
    let (len, rest) = if first < 0x80 {
        (usize::from(first), rest)
    } else {
        let count = usize::from(first & 0x7f);
        if count == 0 || count > 4 || rest.len() < count {
            return None;
        }
        let (digits, rest) = rest.split_at(count);
        let len = digits
            .iter()
            .fold(0usize, |acc, b| (acc << 8) | usize::from(*b));
        (len, rest)
    };
    if rest.len() < len {
        return None;
    }
    let (content, rest) = rest.split_at(len);
    Some((tag, content, rest))
}

/// The elements inside a constructed one's contents, each as its tag and contents.
fn children(mut bytes: &[u8]) -> Option<Vec<(u8, &[u8])>> {
    let mut out = Vec::new();
    while !bytes.is_empty() {
        let (tag, content, rest) = tlv(bytes)?;
        out.push((tag, content));
        bytes = rest;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_data_url_is_the_only_logo_location_read() {
        let svg = b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>";
        let b64 = base64::engine::general_purpose::STANDARD.encode(svg);
        let cases: Vec<(String, Option<Vec<u8>>)> = vec![
            (
                format!("data:image/svg+xml;base64,{b64}"),
                Some(svg.to_vec()),
            ),
            (format!("DATA:Image/SVG+XML ; BASE64,{b64}"), None),
            (
                format!("data:Image/SVG+XML;BASE64,{b64}"),
                Some(svg.to_vec()),
            ),
            (format!("data:image/png;base64,{b64}"), None),
            ("data:image/svg+xml,<svg/>".to_owned(), None),
            ("https://brand.example/logo.svg".to_owned(), None),
        ];
        for (uri, want) in cases {
            assert_eq!(svg_data_url(&uri), want, "{uri}");
        }
    }

    #[test]
    fn a_gzipped_logo_is_read_and_a_large_one_is_not() {
        use flate2::write::GzEncoder;
        use std::io::Write as _;
        let svg = b"<svg xmlns=\"http://www.w3.org/2000/svg\" baseProfile=\"tiny-ps\"/>".to_vec();
        let mut gz = GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(&svg).unwrap();
        let gz = gz.finish().unwrap();
        assert_eq!(svg_bytes(&gz), Some(svg.clone()));
        assert!(same_logo(&gz, &svg), "the same logo, one of them gzipped");
        assert!(!same_logo(b"<svg/>", &svg));

        let bomb = vec![b' '; MAX_SVG * 4];
        let mut gz = GzEncoder::new(Vec::new(), flate2::Compression::best());
        gz.write_all(&bomb).unwrap();
        let gz = gz.finish().unwrap();
        assert!(gz.len() < MAX_SVG, "small compressed");
        assert_eq!(svg_bytes(&gz), None, "large once inflated");
        assert_eq!(svg_bytes(&bomb), None, "large as it is");
    }

    #[test]
    fn truncated_der_is_refused() {
        assert_eq!(tlv(&[0x30, 0x05, 0x01]), None);
        assert_eq!(tlv(&[0x30, 0x84, 0xff, 0xff, 0xff, 0xff]), None);
        assert_eq!(tlv(&[0x1f, 0x01, 0x00]), None, "multi-byte tags");
        assert_eq!(subject_logo(&[0x30, 0x00]), None);
    }
}
