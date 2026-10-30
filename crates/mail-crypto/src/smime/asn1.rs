//! The object identifiers S/MIME names things by, and the few ASN.1 structures the `cms` crate
//! does not define: AuthEnvelopedData (RFC 5083), its AES-GCM parameters (RFC 5084), RSAES-OAEP
//! parameters (RFC 4055), and PKCS#12's safe bags (RFC 7292 §4.2) in a form that encodes and
//! decodes the same way.
//!
//! The identifiers are spelled out here, one per line with its name, rather than looked up in a
//! database of thousands: the list is what this module understands, and reading it says so.

use cms::content_info::CmsVersion;
use cms::enveloped_data::{EncryptedContentInfo, OriginatorInfo, RecipientInfos};
use const_oid::ObjectIdentifier as Oid;
use der::asn1::OctetString;
use der::{Any, Sequence};
use spki::AlgorithmIdentifierOwned;
use x509_cert::attr::Attributes;

const fn oid(text: &str) -> Oid {
    Oid::new_unwrap(text)
}

// CMS content types (RFC 5652 §4, §5.1, §6.1; RFC 5083 §1).
pub(crate) const DATA: Oid = oid("1.2.840.113549.1.7.1");
pub(crate) const SIGNED_DATA: Oid = oid("1.2.840.113549.1.7.2");
pub(crate) const ENVELOPED_DATA: Oid = oid("1.2.840.113549.1.7.3");
pub(crate) const ENCRYPTED_DATA: Oid = oid("1.2.840.113549.1.7.6");
pub(crate) const AUTH_ENVELOPED_DATA: Oid = oid("1.2.840.113549.1.9.16.1.23");

// Signed attributes (RFC 5652 §11; RFC 8551 §2.5.2).
pub(crate) const CONTENT_TYPE: Oid = oid("1.2.840.113549.1.9.3");
pub(crate) const MESSAGE_DIGEST: Oid = oid("1.2.840.113549.1.9.4");
pub(crate) const SIGNING_TIME: Oid = oid("1.2.840.113549.1.9.5");
pub(crate) const SMIME_CAPABILITIES: Oid = oid("1.2.840.113549.1.9.15");

// Digests.
pub(crate) const MD5: Oid = oid("1.2.840.113549.2.5");
pub(crate) const SHA1: Oid = oid("1.3.14.3.2.26");
pub(crate) const SHA256: Oid = oid("2.16.840.1.101.3.4.2.1");
pub(crate) const SHA384: Oid = oid("2.16.840.1.101.3.4.2.2");
pub(crate) const SHA512: Oid = oid("2.16.840.1.101.3.4.2.3");

// Public keys and signatures.
pub(crate) const RSA_ENCRYPTION: Oid = oid("1.2.840.113549.1.1.1");
pub(crate) const MD5_WITH_RSA: Oid = oid("1.2.840.113549.1.1.4");
pub(crate) const SHA1_WITH_RSA: Oid = oid("1.2.840.113549.1.1.5");
pub(crate) const RSAES_OAEP: Oid = oid("1.2.840.113549.1.1.7");
pub(crate) const MGF1: Oid = oid("1.2.840.113549.1.1.8");
pub(crate) const SHA256_WITH_RSA: Oid = oid("1.2.840.113549.1.1.11");
pub(crate) const SHA384_WITH_RSA: Oid = oid("1.2.840.113549.1.1.12");
pub(crate) const SHA512_WITH_RSA: Oid = oid("1.2.840.113549.1.1.13");
pub(crate) const EC_PUBLIC_KEY: Oid = oid("1.2.840.10045.2.1");
pub(crate) const ECDSA_WITH_SHA1: Oid = oid("1.2.840.10045.4.1");
pub(crate) const ECDSA_WITH_SHA256: Oid = oid("1.2.840.10045.4.3.2");
pub(crate) const ECDSA_WITH_SHA384: Oid = oid("1.2.840.10045.4.3.3");
pub(crate) const ECDSA_WITH_SHA512: Oid = oid("1.2.840.10045.4.3.4");
pub(crate) const SECP256R1: Oid = oid("1.2.840.10045.3.1.7");
pub(crate) const SECP384R1: Oid = oid("1.3.132.0.34");

// Content encryption.
pub(crate) const AES128_CBC: Oid = oid("2.16.840.1.101.3.4.1.2");
pub(crate) const AES192_CBC: Oid = oid("2.16.840.1.101.3.4.1.22");
pub(crate) const AES256_CBC: Oid = oid("2.16.840.1.101.3.4.1.42");
pub(crate) const AES128_GCM: Oid = oid("2.16.840.1.101.3.4.1.6");
pub(crate) const AES192_GCM: Oid = oid("2.16.840.1.101.3.4.1.26");
pub(crate) const AES256_GCM: Oid = oid("2.16.840.1.101.3.4.1.46");
pub(crate) const DES_EDE3_CBC: Oid = oid("1.2.840.113549.3.7");
pub(crate) const RC2_CBC: Oid = oid("1.2.840.113549.3.2");
pub(crate) const DES_CBC: Oid = oid("1.3.14.3.2.7");

// Certificates (RFC 5280 §4.2.1; RFC 8550 §4.4).
pub(crate) const EMAIL_ADDRESS: Oid = oid("1.2.840.113549.1.9.1");
pub(crate) const EMAIL_PROTECTION: Oid = oid("1.3.6.1.5.5.7.3.4");
pub(crate) const ANY_EXTENDED_KEY_USAGE: Oid = oid("2.5.29.37.0");

// PKCS#12 (RFC 7292) and PKCS#5 (RFC 8018).
pub(crate) const PBES2: Oid = oid("1.2.840.113549.1.5.13");
pub(crate) const PBMAC1: Oid = oid("1.2.840.113549.1.5.14");
pub(crate) const PBE_SHA1_3DES: Oid = oid("1.2.840.113549.1.12.1.3");
pub(crate) const PBE_SHA1_RC2_128: Oid = oid("1.2.840.113549.1.12.1.5");
pub(crate) const PBE_SHA1_RC2_40: Oid = oid("1.2.840.113549.1.12.1.6");
pub(crate) const KEY_BAG: Oid = oid("1.2.840.113549.1.12.10.1.1");
pub(crate) const SHROUDED_KEY_BAG: Oid = oid("1.2.840.113549.1.12.10.1.2");
pub(crate) const CERT_BAG: Oid = oid("1.2.840.113549.1.12.10.1.3");
pub(crate) const X509_CERTIFICATE: Oid = oid("1.2.840.113549.1.9.22.1");
pub(crate) const LOCAL_KEY_ID: Oid = oid("1.2.840.113549.1.9.21");

/// A human name for an algorithm identifier, for messages: its name when this module knows it,
/// its dotted form otherwise.
pub(crate) fn name(id: &Oid) -> String {
    const NAMES: &[(Oid, &str)] = &[
        (MD5, "MD5"),
        (SHA1, "SHA-1"),
        (SHA256, "SHA-256"),
        (SHA384, "SHA-384"),
        (SHA512, "SHA-512"),
        (MD5_WITH_RSA, "MD5 with RSA"),
        (SHA1_WITH_RSA, "SHA-1 with RSA"),
        (ECDSA_WITH_SHA1, "ECDSA with SHA-1"),
        (DES_EDE3_CBC, "triple DES"),
        (RC2_CBC, "RC2"),
        (DES_CBC, "DES"),
        (PBE_SHA1_RC2_40, "40-bit RC2"),
        (PBE_SHA1_RC2_128, "128-bit RC2"),
        (PBMAC1, "PBMAC1"),
    ];
    NAMES
        .iter()
        .find(|(known, _)| known == id)
        .map_or_else(|| id.to_string(), |(_, n)| (*n).to_owned())
}

/// `AuthEnvelopedData` (RFC 5083 §2.1): content encrypted and authenticated in one step.
#[derive(Clone, Debug, Eq, PartialEq, Sequence)]
pub(crate) struct AuthEnvelopedData {
    pub version: CmsVersion,
    #[asn1(
        context_specific = "0",
        tag_mode = "IMPLICIT",
        constructed = "true",
        optional = "true"
    )]
    pub originator_info: Option<OriginatorInfo>,
    pub recip_infos: RecipientInfos,
    pub auth_encrypted_content_info: EncryptedContentInfo,
    #[asn1(
        context_specific = "1",
        tag_mode = "IMPLICIT",
        constructed = "true",
        optional = "true"
    )]
    pub auth_attrs: Option<Attributes>,
    pub mac: OctetString,
    #[asn1(
        context_specific = "2",
        tag_mode = "IMPLICIT",
        constructed = "true",
        optional = "true"
    )]
    pub unauth_attrs: Option<Attributes>,
}

/// `GCMParameters` (RFC 5084 §3.2).
#[derive(Clone, Debug, Eq, PartialEq, Sequence)]
pub(crate) struct GcmParameters {
    pub nonce: OctetString,
    #[asn1(default = "default_icv_len")]
    pub icv_len: u8,
}

fn default_icv_len() -> u8 {
    12
}

/// `RSAES-OAEP-params` (RFC 4055 §4.1). Every field has a default: SHA-1, MGF1 with SHA-1, and
/// an empty label.
#[derive(Clone, Debug, Eq, PartialEq, Sequence)]
pub(crate) struct OaepParams {
    #[asn1(context_specific = "0", tag_mode = "EXPLICIT", optional = "true")]
    pub hash: Option<AlgorithmIdentifierOwned>,
    #[asn1(context_specific = "1", tag_mode = "EXPLICIT", optional = "true")]
    pub mask_gen: Option<AlgorithmIdentifierOwned>,
    #[asn1(context_specific = "2", tag_mode = "EXPLICIT", optional = "true")]
    pub label: Option<AlgorithmIdentifierOwned>,
}

/// `SafeBag` (RFC 7292 §4.2), with the value kept as the ANY it is. The `pkcs12` crate's own
/// `SafeBag` decodes its value with the `[0]` tag and encodes it without, so a bag it read cannot
/// be written back; this one round-trips.
#[derive(Clone, Debug, Eq, PartialEq, Sequence)]
pub(crate) struct SafeBag {
    pub bag_id: Oid,
    #[asn1(context_specific = "0", tag_mode = "EXPLICIT")]
    pub bag_value: Any,
    pub bag_attributes: Option<Attributes>,
}

/// `CertBag` (RFC 7292 §4.2.3), likewise with an ANY so it round-trips.
#[derive(Clone, Debug, Eq, PartialEq, Sequence)]
pub(crate) struct CertBag {
    pub cert_id: Oid,
    #[asn1(context_specific = "0", tag_mode = "EXPLICIT")]
    pub cert_value: OctetString,
}
