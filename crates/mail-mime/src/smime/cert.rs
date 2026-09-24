//! X.509 certificates as S/MIME uses them: whose they are, when, for what, and the record the
//! store keeps.

use super::asn1;
use super::smime_error;
use crate::MimeError;
use chrono::{DateTime, Utc};
use der::{Decode, Encode};
use mail_domain::{CertFingerprint, CertSource, KeyTrust, SecretHeld, SmimeCert};
use sha2::{Digest, Sha256};
use x509_cert::Certificate;
use x509_cert::ext::pkix::name::GeneralName;
use x509_cert::ext::pkix::{BasicConstraints, ExtendedKeyUsage, KeyUsage, SubjectAltName};

/// One certificate, parsed, with the exact bytes it was read from — which is what its
/// fingerprint and every signature over it are computed on.
#[derive(Clone, PartialEq, Eq)]
pub struct Cert {
    pub(crate) inner: Certificate,
    der: Vec<u8>,
}

impl std::fmt::Debug for Cert {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Cert")
            .field("subject", &self.subject())
            .field("fingerprint", &self.fingerprint().to_string())
            .finish()
    }
}

/// Every certificate in `bytes`: one DER certificate, or any number of PEM blocks.
pub fn read_certs(bytes: &[u8]) -> Result<Vec<Cert>, MimeError> {
    let text = String::from_utf8_lossy(bytes);
    if text.contains("-----BEGIN CERTIFICATE-----") {
        let mut out = Vec::new();
        let mut rest = text.as_ref();
        while let Some(start) = rest.find("-----BEGIN CERTIFICATE-----") {
            let end_marker = "-----END CERTIFICATE-----";
            let Some(end) = rest[start..].find(end_marker) else {
                return Err(MimeError::Smime(
                    "a PEM certificate is cut short".to_owned(),
                ));
            };
            let block = &rest[start..start + end + end_marker.len()];
            let (_, der) = der::pem::decode_vec(block.as_bytes())
                .map_err(|e| smime_error("a PEM certificate", e))?;
            out.push(Cert::from_der(&der)?);
            rest = &rest[start + end + end_marker.len()..];
        }
        return Ok(out);
    }
    Ok(vec![Cert::from_der(bytes)?])
}

impl Cert {
    /// A certificate from its DER encoding.
    pub fn from_der(der: &[u8]) -> Result<Cert, MimeError> {
        let inner = Certificate::from_der(der).map_err(|e| smime_error("a certificate", e))?;
        // Kept as re-encoded, which for a DER certificate is the bytes given.
        let der = inner
            .to_der()
            .map_err(|e| smime_error("a certificate", e))?;
        Ok(Cert { inner, der })
    }

    pub(crate) fn from_parsed(inner: Certificate) -> Option<Cert> {
        let der = inner.to_der().ok()?;
        Some(Cert { inner, der })
    }

    /// The certificate, DER.
    pub fn der(&self) -> &[u8] {
        &self.der
    }

    /// The certificate as a PEM block.
    pub fn pem(&self) -> String {
        der::pem::encode_string("CERTIFICATE", der::pem::LineEnding::LF, &self.der)
            .unwrap_or_default()
    }

    /// SHA-256 over the DER encoding.
    pub fn fingerprint(&self) -> CertFingerprint {
        CertFingerprint(Sha256::digest(&self.der).into())
    }

    /// The subject's distinguished name, RFC 4514.
    pub fn subject(&self) -> String {
        self.inner.tbs_certificate.subject.to_string()
    }

    /// The issuer's distinguished name, RFC 4514.
    pub fn issuer(&self) -> String {
        self.inner.tbs_certificate.issuer.to_string()
    }

    /// The serial number, upper-case hex.
    pub fn serial(&self) -> String {
        self.inner
            .tbs_certificate
            .serial_number
            .as_bytes()
            .iter()
            .map(|b| format!("{b:02X}"))
            .collect()
    }

    pub fn not_before(&self) -> DateTime<Utc> {
        instant(self.inner.tbs_certificate.validity.not_before)
    }

    pub fn not_after(&self) -> DateTime<Utc> {
        instant(self.inner.tbs_certificate.validity.not_after)
    }

    /// The addresses it is for, lower-cased: the subject alternative name's `rfc822Name`s, and
    /// only when there are none, the subject's `emailAddress` attributes (RFC 8550 §3).
    pub fn emails(&self) -> Vec<String> {
        let mut found: Vec<String> = Vec::new();
        if let Ok(Some((_, san))) = self.inner.tbs_certificate.get::<SubjectAltName>() {
            for name in &san.0 {
                if let GeneralName::Rfc822Name(email) = name {
                    found.push(email.as_str().to_owned());
                }
            }
        }
        if found.is_empty() {
            for rdn in self.inner.tbs_certificate.subject.0.iter() {
                for attribute in rdn.0.iter() {
                    if attribute.oid == asn1::EMAIL_ADDRESS
                        && let Ok(text) = attribute.value.decode_as::<der::asn1::Ia5String>()
                    {
                        found.push(text.as_str().to_owned());
                    }
                }
            }
        }
        let mut out: Vec<String> = Vec::new();
        for email in found {
            let email = email.trim().to_ascii_lowercase();
            if !email.is_empty() && !out.contains(&email) {
                out.push(email);
            }
        }
        out
    }

    /// Whether it names `address`, compared whole and without regard to case.
    pub fn is_for(&self, address: &str) -> bool {
        let address = address.trim();
        self.emails()
            .iter()
            .any(|e| e.eq_ignore_ascii_case(address))
    }

    /// Whether it is a certification authority's: basic constraints say so.
    pub(crate) fn is_ca(&self) -> bool {
        matches!(
            self.inner.tbs_certificate.get::<BasicConstraints>(),
            Ok(Some((_, BasicConstraints { ca: true, .. })))
        )
    }

    /// Whether its key may sign certificates: a CA whose key usage, if it states one, includes
    /// `keyCertSign`.
    pub(crate) fn may_issue(&self) -> bool {
        self.is_ca()
            && match self.inner.tbs_certificate.get::<KeyUsage>() {
                Ok(Some((_, usage))) => usage.key_cert_sign(),
                Ok(None) => true,
                Err(_) => false,
            }
    }

    /// Whether, trusted as an anchor, it may vouch for others: unless it says it is not a CA.
    /// Old root certificates carry no extensions at all, and the system trusts them to issue;
    /// a correspondent's own certificate that the user trusts says `ca: false`, and vouches only
    /// for itself.
    pub(crate) fn may_issue_as_anchor(&self) -> bool {
        match self.inner.tbs_certificate.get::<BasicConstraints>() {
            Ok(None) => true,
            Ok(Some((_, constraints))) => constraints.ca,
            Err(_) => false,
        }
    }

    /// Whether it may sign mail (RFC 8550 §4.4.2, §4.4.4): a key usage, if stated, with
    /// `digitalSignature` or `nonRepudiation`, and an extended key usage, if stated, with
    /// `emailProtection` or `anyExtendedKeyUsage`.
    pub(crate) fn may_sign_mail(&self) -> bool {
        let usage = match self.inner.tbs_certificate.get::<KeyUsage>() {
            Ok(Some((_, usage))) => usage.digital_signature() || usage.non_repudiation(),
            Ok(None) => true,
            Err(_) => false,
        };
        usage && self.email_purpose()
    }

    /// Whether mail may be encrypted to it: an RSA key (the only kind this client encrypts to),
    /// a key usage, if stated, with `keyEncipherment`, and the mail purpose.
    pub fn can_encrypt(&self) -> bool {
        let usage = match self.inner.tbs_certificate.get::<KeyUsage>() {
            Ok(Some((_, usage))) => usage.key_encipherment(),
            Ok(None) => true,
            Err(_) => false,
        };
        usage && self.email_purpose() && self.rsa_public().is_some()
    }

    fn email_purpose(&self) -> bool {
        match self.inner.tbs_certificate.get::<ExtendedKeyUsage>() {
            Ok(Some((_, eku))) => eku
                .0
                .iter()
                .any(|p| *p == asn1::EMAIL_PROTECTION || *p == asn1::ANY_EXTENDED_KEY_USAGE),
            Ok(None) => true,
            Err(_) => false,
        }
    }

    /// Its RSA public key, when it has one.
    pub(crate) fn rsa_public(&self) -> Option<rsa::RsaPublicKey> {
        use rsa::pkcs8::DecodePublicKey;
        let spki = &self.inner.tbs_certificate.subject_public_key_info;
        if spki.algorithm.oid != asn1::RSA_ENCRYPTION {
            return None;
        }
        rsa::RsaPublicKey::from_public_key_der(&spki.to_der().ok()?).ok()
    }

    /// Its subject public key info, DER: what a private key is matched to it by.
    pub(crate) fn spki_der(&self) -> Vec<u8> {
        self.inner
            .tbs_certificate
            .subject_public_key_info
            .to_der()
            .unwrap_or_default()
    }

    /// Its subject key identifier, when it states one.
    pub(crate) fn subject_key_id(&self) -> Option<Vec<u8>> {
        self.inner
            .tbs_certificate
            .get::<x509_cert::ext::pkix::SubjectKeyIdentifier>()
            .ok()
            .flatten()
            .map(|(_, id)| id.0.as_bytes().to_vec())
    }

    /// The store's record of it: `chain` is the issuers that came with it.
    pub fn record(&self, chain: &[Cert], source: CertSource, now: DateTime<Utc>) -> SmimeCert {
        SmimeCert {
            fingerprint: self.fingerprint(),
            subject: self.subject(),
            issuer: self.issuer(),
            serial: self.serial(),
            emails: self.emails(),
            not_before: self.not_before(),
            not_after: self.not_after(),
            der: self.der.clone(),
            chain: chain.iter().map(|c| c.der.clone()).collect(),
            source,
            first_seen: now,
            last_seen: now,
            trust: KeyTrust::Unverified,
            secret: SecretHeld::Absent,
        }
    }
}

fn instant(time: x509_cert::time::Time) -> DateTime<Utc> {
    let since = time.to_unix_duration();
    DateTime::from_timestamp(
        i64::try_from(since.as_secs()).unwrap_or(i64::MAX),
        since.subsec_nanos(),
    )
    .unwrap_or(DateTime::<Utc>::MAX_UTC)
}
