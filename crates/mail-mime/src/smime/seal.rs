//! Sending: a built message made S/MIME — signed as `multipart/signed` (RFC 8551 §3.5.3),
//! encrypted as `application/pkcs7-mime` enveloped-data (§3.3), or signed and then encrypted
//! (§3.6).
//!
//! Only the content entity is signed or encrypted — its `Content-*` fields and body — never the
//! message's own header, for the reason `openpgp::seal` gives: the outbox rewrites `Date` as the
//! message leaves.
//!
//! Signing: SHA-256 (SHA-384 for a P-384 key), with the signer's certificate and the issuers
//! that came with it included, and the content type, signing time, digest and S/MIME
//! capabilities as signed attributes. Encrypting: AES-256-CBC, the content key wrapped with
//! RSA PKCS#1 v1.5 for each recipient's certificate — what every S/MIME reader since RFC 3851
//! opens. Nothing weaker is ever produced.

use super::asn1;
use super::cert::Cert;
use super::keys::Identity;
use super::sign::DigestAlg;
use super::smime_error;
use crate::MimeError;
use crate::openpgp::Rng;
use crate::openpgp::entity::{Entity, crlf, field_name, fields, is_content_field};
use chrono::{DateTime, Utc};
use cms::builder::{
    ContentEncryptionAlgorithm, EnvelopedDataBuilder, KeyEncryptionInfo,
    KeyTransRecipientInfoBuilder,
};
use cms::cert::{CertificateChoices, IssuerAndSerialNumber};
use cms::content_info::{CmsVersion, ContentInfo};
use cms::enveloped_data::RecipientIdentifier;
use cms::signed_data::{
    CertificateSet, EncapsulatedContentInfo, SignedData, SignerIdentifier, SignerInfo, SignerInfos,
};
use der::asn1::{OctetString, SetOfVec};
use der::{Any, Decode, Encode};
use mail_domain::Smime;
use x509_cert::attr::Attribute;

/// What to do to a message as it is sent.
#[derive(Debug, Clone, Copy)]
pub struct Sealing<'a> {
    pub mode: Smime,
    /// The sender's identity. Required to sign.
    pub signer: Option<&'a Identity>,
    /// Everyone to encrypt to — every recipient *and the sender*, so the sent copy can be read.
    /// Required to encrypt.
    pub recipients: &'a [Cert],
    /// The signing time.
    pub now: DateTime<Utc>,
}

/// `frozen` — a whole message as `crate::build` writes it — signed, encrypted, or both, as
/// `how.mode` says. [`Smime::None`] returns it as it was.
pub fn seal(frozen: &[u8], how: &Sealing<'_>, rng: &mut impl Rng) -> Result<Vec<u8>, MimeError> {
    if how.mode == Smime::None {
        return Ok(frozen.to_vec());
    }
    let message = Entity::of(frozen);
    let (outer, content): (Vec<&[u8]>, Vec<&[u8]>) = fields(message.head)
        .into_iter()
        .partition(|field| !is_content_field(field));
    let content: Vec<&[u8]> = content
        .into_iter()
        .filter(|field| !field_name(field).eq_ignore_ascii_case(b"MIME-Version"))
        .collect();
    let mut entity = crlf(&[content.concat(), b"\r\n".to_vec(), message.body.to_vec()].concat());
    if how.mode.signs() {
        let signer = how.signer.ok_or_else(|| {
            MimeError::Smime("there is no S/MIME identity to sign with".to_owned())
        })?;
        entity = signed(&entity, signer, how.now, rng)?;
    }
    if how.mode.encrypts() {
        if how.recipients.is_empty() {
            return Err(MimeError::Smime(
                "there is no certificate to encrypt to".to_owned(),
            ));
        }
        entity = enveloped(&entity, how.recipients, rng)?;
    }
    let mut out = outer.concat();
    out.extend_from_slice(b"MIME-Version: 1.0\r\n");
    out.extend_from_slice(&entity);
    Ok(out)
}

/// A boundary no content will contain: 128 random bits, hex.
fn boundary(rng: &mut impl Rng) -> String {
    let mut bytes = [0u8; 16];
    rng.fill_bytes(&mut bytes);
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!("mailo-smime-{hex}")
}

/// `bytes` as base64 in lines of 76, CRLF-ended.
fn base64_lines(bytes: &[u8]) -> Vec<u8> {
    use base64::Engine;
    let text = base64::engine::general_purpose::STANDARD.encode(bytes);
    let mut out = Vec::with_capacity(text.len() + text.len() / 38 + 2);
    for line in text.as_bytes().chunks(76) {
        out.extend_from_slice(line);
        out.extend_from_slice(b"\r\n");
    }
    out
}

/// The `multipart/signed` entity: `entity` as it was signed, then its detached signature.
fn signed(
    entity: &[u8],
    signer: &Identity,
    now: DateTime<Utc>,
    rng: &mut impl Rng,
) -> Result<Vec<u8>, MimeError> {
    let digest = signer.key.digest();
    let signature = signed_data(entity, signer, digest, now)?;
    let boundary = boundary(rng);
    let mut out = format!(
        "Content-Type: multipart/signed; protocol=\"application/pkcs7-signature\";\r\n \
         micalg={}; boundary=\"{boundary}\"\r\n\r\n\
         This is an S/MIME signed message\r\n--{boundary}\r\n",
        digest.micalg()
    )
    .into_bytes();
    out.extend_from_slice(entity);
    out.extend_from_slice(
        format!(
            "\r\n--{boundary}\r\nContent-Type: application/pkcs7-signature; name=\"smime.p7s\"\r\n\
             Content-Transfer-Encoding: base64\r\n\
             Content-Disposition: attachment; filename=\"smime.p7s\"\r\n\
             Content-Description: S/MIME Cryptographic Signature\r\n\r\n"
        )
        .as_bytes(),
    );
    out.extend_from_slice(&base64_lines(&signature));
    out.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    Ok(out)
}

fn attribute(oid: const_oid::ObjectIdentifier, value: Any) -> Result<Attribute, MimeError> {
    let mut values = SetOfVec::new();
    values
        .insert(value)
        .map_err(|e| smime_error("an attribute", e))?;
    Ok(Attribute { oid, values })
}

fn any<T: Encode>(value: &T) -> Result<Any, MimeError> {
    let der = value.to_der().map_err(|e| smime_error("an attribute", e))?;
    Any::from_der(&der).map_err(|e| smime_error("an attribute", e))
}

/// The S/MIME capabilities attribute (RFC 8551 §2.5.2): what this client reads, strongest first,
/// so a correspondent replying encrypted picks something it can open.
fn capabilities() -> Result<Any, MimeError> {
    let preferred: Vec<spki::AlgorithmIdentifierOwned> = [
        asn1::AES256_GCM,
        asn1::AES128_GCM,
        asn1::AES256_CBC,
        asn1::AES128_CBC,
    ]
    .into_iter()
    .map(|oid| spki::AlgorithmIdentifierOwned {
        oid,
        parameters: None,
    })
    .collect();
    any(&preferred)
}

/// A detached SignedData over `entity`, DER.
fn signed_data(
    entity: &[u8],
    signer: &Identity,
    digest: DigestAlg,
    now: DateTime<Utc>,
) -> Result<Vec<u8>, MimeError> {
    let content_digest = digest.hash(entity);
    let since = std::time::Duration::from_secs(u64::try_from(now.timestamp()).unwrap_or(0));
    // UTCTime through 2049, GeneralizedTime after (RFC 5652 §11.3).
    let time = match der::asn1::UtcTime::from_unix_duration(since) {
        Ok(utc) => x509_cert::time::Time::UtcTime(utc),
        Err(_) => x509_cert::time::Time::GeneralTime(
            der::asn1::GeneralizedTime::from_unix_duration(since)
                .map_err(|e| smime_error("the signing time", e))?,
        ),
    };
    let attrs: Vec<Attribute> = vec![
        attribute(asn1::CONTENT_TYPE, any(&asn1::DATA)?)?,
        attribute(asn1::SIGNING_TIME, any(&time)?)?,
        attribute(
            asn1::MESSAGE_DIGEST,
            any(&OctetString::new(content_digest).map_err(|e| smime_error("a digest", e))?)?,
        )?,
        attribute(asn1::SMIME_CAPABILITIES, capabilities()?)?,
    ];
    let signed_attrs =
        SetOfVec::try_from(attrs).map_err(|e| smime_error("the signed attributes", e))?;
    let to_sign = signed_attrs
        .to_der()
        .map_err(|e| smime_error("the signed attributes", e))?;
    let (signature, signature_algorithm) = signer.key.sign(&digest.hash(&to_sign))?;
    let tbs = &signer.cert.inner.tbs_certificate;
    let signer_info = SignerInfo {
        version: CmsVersion::V1,
        sid: SignerIdentifier::IssuerAndSerialNumber(IssuerAndSerialNumber {
            issuer: tbs.issuer.clone(),
            serial_number: tbs.serial_number.clone(),
        }),
        digest_alg: digest.identifier(),
        signed_attrs: Some(signed_attrs),
        signature_algorithm,
        signature: OctetString::new(signature).map_err(|e| smime_error("a signature", e))?,
        unsigned_attrs: None,
    };
    let certificates: Vec<CertificateChoices> = std::iter::once(&signer.cert)
        .chain(&signer.chain)
        .map(|c| CertificateChoices::Certificate(c.inner.clone()))
        .collect();
    let data = SignedData {
        version: CmsVersion::V1,
        digest_algorithms: SetOfVec::try_from(vec![digest.identifier()])
            .map_err(|e| smime_error("a signature", e))?,
        encap_content_info: EncapsulatedContentInfo {
            econtent_type: asn1::DATA,
            econtent: None,
        },
        certificates: Some(
            CertificateSet::try_from(certificates).map_err(|e| smime_error("a signature", e))?,
        ),
        crls: None,
        signer_infos: SignerInfos::try_from(vec![signer_info])
            .map_err(|e| smime_error("a signature", e))?,
    };
    wrap(asn1::SIGNED_DATA, &data)
}

/// `value` as the content of a ContentInfo of type `kind`, DER.
fn wrap<T: Encode>(kind: const_oid::ObjectIdentifier, value: &T) -> Result<Vec<u8>, MimeError> {
    ContentInfo {
        content_type: kind,
        content: any(value)?,
    }
    .to_der()
    .map_err(|e| smime_error("the S/MIME structure", e))
}

/// The `application/pkcs7-mime` enveloped-data entity holding `entity`.
fn enveloped(entity: &[u8], recipients: &[Cert], rng: &mut impl Rng) -> Result<Vec<u8>, MimeError> {
    // Each recipient needs its own draw from the generator for its key wrapping, and the builder
    // holds each one's until it is built; the content key and IV come from `rng` itself.
    let mut draws: Vec<rand::rngs::StdRng> = Vec::with_capacity(recipients.len());
    for _ in recipients {
        let mut seed = [0u8; 32];
        rng.fill_bytes(&mut seed);
        draws.push(rand::SeedableRng::from_seed(seed));
    }
    let mut builder =
        EnvelopedDataBuilder::new(None, entity, ContentEncryptionAlgorithm::Aes256Cbc, None)
            .map_err(|e| smime_error("encrypting", e))?;
    for (cert, draw) in recipients.iter().zip(draws.iter_mut()) {
        let public = cert.rsa_public().ok_or_else(|| {
            MimeError::Smime(format!(
                "the certificate for {} has no RSA key, and mail is only encrypted to RSA here",
                cert.emails().join(", ")
            ))
        })?;
        let tbs = &cert.inner.tbs_certificate;
        let recipient = KeyTransRecipientInfoBuilder::new(
            RecipientIdentifier::IssuerAndSerialNumber(IssuerAndSerialNumber {
                issuer: tbs.issuer.clone(),
                serial_number: tbs.serial_number.clone(),
            }),
            KeyEncryptionInfo::Rsa(public),
            draw,
        )
        .map_err(|e| smime_error("encrypting", e))?;
        builder
            .add_recipient_info(recipient)
            .map_err(|e| smime_error("encrypting", e))?;
    }
    let data = builder
        .build_with_rng(rng)
        .map_err(|e| smime_error("encrypting", e))?;
    let der = wrap(asn1::ENVELOPED_DATA, &data)?;
    let mut out = b"Content-Type: application/pkcs7-mime; smime-type=enveloped-data;\r\n \
          name=\"smime.p7m\"\r\nContent-Transfer-Encoding: base64\r\n\
          Content-Disposition: attachment; filename=\"smime.p7m\"\r\n\
          Content-Description: S/MIME Encrypted Message\r\n\r\n"
        .to_vec();
    out.extend_from_slice(&base64_lines(&der));
    Ok(out)
}
