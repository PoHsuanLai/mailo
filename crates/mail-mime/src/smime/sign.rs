//! Digests and signature checks: over a message's signed attributes, and over a certificate by
//! its issuer.

use super::asn1;
use der::Encode;
use mail_domain::BadSignature;
use sha2::Digest;
use spki::{AlgorithmIdentifierOwned, SubjectPublicKeyInfoOwned};

/// A digest algorithm this module computes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DigestAlg {
    Sha1,
    Sha256,
    Sha384,
    Sha512,
}

impl DigestAlg {
    /// The algorithm `oid` names. MD5 is refused as weak; anything else unknown as unsupported.
    pub fn of(oid: &const_oid::ObjectIdentifier) -> Result<DigestAlg, String> {
        Ok(match *oid {
            asn1::SHA1 => DigestAlg::Sha1,
            asn1::SHA256 => DigestAlg::Sha256,
            asn1::SHA384 => DigestAlg::Sha384,
            asn1::SHA512 => DigestAlg::Sha512,
            other => return Err(asn1::name(&other)),
        })
    }

    pub fn oid(self) -> const_oid::ObjectIdentifier {
        match self {
            DigestAlg::Sha1 => asn1::SHA1,
            DigestAlg::Sha256 => asn1::SHA256,
            DigestAlg::Sha384 => asn1::SHA384,
            DigestAlg::Sha512 => asn1::SHA512,
        }
    }

    /// `micalg` for a `multipart/signed` (RFC 8551 §3.5.3.2).
    pub fn micalg(self) -> &'static str {
        match self {
            DigestAlg::Sha1 => "sha-1",
            DigestAlg::Sha256 => "sha-256",
            DigestAlg::Sha384 => "sha-384",
            DigestAlg::Sha512 => "sha-512",
        }
    }

    pub fn hash(self, data: &[u8]) -> Vec<u8> {
        match self {
            DigestAlg::Sha1 => sha1::Sha1::digest(data).to_vec(),
            DigestAlg::Sha256 => sha2::Sha256::digest(data).to_vec(),
            DigestAlg::Sha384 => sha2::Sha384::digest(data).to_vec(),
            DigestAlg::Sha512 => sha2::Sha512::digest(data).to_vec(),
        }
    }

    pub fn identifier(self) -> AlgorithmIdentifierOwned {
        AlgorithmIdentifierOwned {
            oid: self.oid(),
            parameters: None,
        }
    }
}

/// What a signature algorithm identifier says: which kind of key, and the digest when it names
/// one itself.
enum Scheme {
    Rsa(Option<DigestAlg>),
    Ecdsa(Option<DigestAlg>),
}

fn scheme(algorithm: &AlgorithmIdentifierOwned) -> Result<Scheme, BadSignature> {
    let weak = |name: &str| BadSignature::Weak {
        algorithm: name.to_owned(),
    };
    Ok(match algorithm.oid {
        asn1::RSA_ENCRYPTION => Scheme::Rsa(None),
        asn1::SHA1_WITH_RSA => Scheme::Rsa(Some(DigestAlg::Sha1)),
        asn1::SHA256_WITH_RSA => Scheme::Rsa(Some(DigestAlg::Sha256)),
        asn1::SHA384_WITH_RSA => Scheme::Rsa(Some(DigestAlg::Sha384)),
        asn1::SHA512_WITH_RSA => Scheme::Rsa(Some(DigestAlg::Sha512)),
        asn1::MD5_WITH_RSA => return Err(weak("MD5")),
        asn1::EC_PUBLIC_KEY => Scheme::Ecdsa(None),
        asn1::ECDSA_WITH_SHA1 => Scheme::Ecdsa(Some(DigestAlg::Sha1)),
        asn1::ECDSA_WITH_SHA256 => Scheme::Ecdsa(Some(DigestAlg::Sha256)),
        asn1::ECDSA_WITH_SHA384 => Scheme::Ecdsa(Some(DigestAlg::Sha384)),
        asn1::ECDSA_WITH_SHA512 => Scheme::Ecdsa(Some(DigestAlg::Sha512)),
        other => {
            return Err(BadSignature::Unsupported {
                algorithm: asn1::name(&other),
            });
        }
    })
}

/// The digest a signature with `algorithm` is computed with: the one the algorithm names, or
/// `otherwise` for an algorithm that names none (plain `rsaEncryption`).
pub(crate) fn signature_digest(
    algorithm: &AlgorithmIdentifierOwned,
    otherwise: DigestAlg,
) -> Result<DigestAlg, BadSignature> {
    Ok(match scheme(algorithm)? {
        Scheme::Rsa(named) | Scheme::Ecdsa(named) => named.unwrap_or(otherwise),
    })
}

/// Whether `signature` is `key`'s over the digest `hash`, made as `algorithm` says.
pub(crate) fn verifies(
    key: &SubjectPublicKeyInfoOwned,
    algorithm: &AlgorithmIdentifierOwned,
    digest: DigestAlg,
    hash: &[u8],
    signature: &[u8],
) -> Result<bool, BadSignature> {
    use signature::hazmat::PrehashVerifier;
    use spki::DecodePublicKey;
    let key_der = key
        .to_der()
        .map_err(|e| BadSignature::Malformed { why: e.to_string() })?;
    match scheme(algorithm)? {
        Scheme::Rsa(_) => {
            let Ok(public) = rsa::RsaPublicKey::from_public_key_der(&key_der) else {
                return Ok(false);
            };
            let padding = match digest {
                DigestAlg::Sha1 => rsa::Pkcs1v15Sign::new::<sha1::Sha1>(),
                DigestAlg::Sha256 => rsa::Pkcs1v15Sign::new::<sha2::Sha256>(),
                DigestAlg::Sha384 => rsa::Pkcs1v15Sign::new::<sha2::Sha384>(),
                DigestAlg::Sha512 => rsa::Pkcs1v15Sign::new::<sha2::Sha512>(),
            };
            Ok(public.verify(padding, hash, signature).is_ok())
        }
        Scheme::Ecdsa(_) => {
            if let Ok(public) = p256::ecdsa::VerifyingKey::from_public_key_der(&key_der) {
                let Ok(sig) = p256::ecdsa::Signature::from_der(signature) else {
                    return Ok(false);
                };
                return Ok(public.verify_prehash(hash, &sig).is_ok());
            }
            if let Ok(public) = p384::ecdsa::VerifyingKey::from_public_key_der(&key_der) {
                let Ok(sig) = p384::ecdsa::Signature::from_der(signature) else {
                    return Ok(false);
                };
                return Ok(public.verify_prehash(hash, &sig).is_ok());
            }
            Ok(false)
        }
    }
}

/// Whether `issuer` signed `subject`: the certificate's signature, checked with the issuer's
/// key. A certificate signed with MD5 or SHA-1 is not believed, whenever it was made.
pub(crate) fn issued(subject: &super::Cert, issuer: &super::Cert) -> bool {
    let certificate = &subject.inner;
    let Ok(tbs) = certificate.tbs_certificate.to_der() else {
        return false;
    };
    let Ok(digest) = signature_digest(&certificate.signature_algorithm, DigestAlg::Sha256) else {
        return false;
    };
    if digest == DigestAlg::Sha1 {
        return false;
    }
    let Some(signature) = certificate.signature.as_bytes() else {
        return false;
    };
    verifies(
        &issuer.inner.tbs_certificate.subject_public_key_info,
        &certificate.signature_algorithm,
        digest,
        &digest.hash(&tbs),
        signature,
    )
    .unwrap_or(false)
}
