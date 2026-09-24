//! Private keys and the identities they make with their certificates.
//!
//! A private key lives in this process only as long as a call needs it: parsed from the PKCS#8
//! the keyring holds, used, dropped. The RustCrypto key types wipe themselves on drop, and the
//! encodings handed out are [`Zeroizing`].

use super::asn1;
use super::cert::Cert;
use super::sign::DigestAlg;
use super::smime_error;
use crate::MimeError;
use der::{Decode, Encode};
use spki::AlgorithmIdentifierOwned;
use zeroize::Zeroizing;

/// A private key of a kind S/MIME here signs with: RSA, or ECDSA on P-256 or P-384.
#[derive(Clone)]
pub enum PrivateKey {
    Rsa(Box<rsa::RsaPrivateKey>),
    P256(p256::SecretKey),
    P384(p384::SecretKey),
}

impl std::fmt::Debug for PrivateKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let kind = match self {
            PrivateKey::Rsa(_) => "RSA",
            PrivateKey::P256(_) => "P-256",
            PrivateKey::P384(_) => "P-384",
        };
        write!(f, "PrivateKey::{kind}(<redacted>)")
    }
}

impl PartialEq for PrivateKey {
    /// Compared by public half: two private keys are the same key when their public keys are.
    fn eq(&self, other: &Self) -> bool {
        self.public_spki_der() == other.public_spki_der()
    }
}

impl Eq for PrivateKey {}

impl PrivateKey {
    /// From PKCS#8 `PrivateKeyInfo`, DER.
    pub fn from_pkcs8_der(der: &[u8]) -> Result<PrivateKey, MimeError> {
        use pkcs8::DecodePrivateKey;
        let info = pkcs8::PrivateKeyInfo::from_der(der).map_err(|e| smime_error("a key", e))?;
        let oid = info.algorithm.oid;
        if oid == asn1::RSA_ENCRYPTION {
            return rsa::RsaPrivateKey::from_pkcs8_der(der)
                .map(|k| PrivateKey::Rsa(Box::new(k)))
                .map_err(|e| smime_error("an RSA key", e));
        }
        if oid == asn1::EC_PUBLIC_KEY {
            let curve = info
                .algorithm
                .parameters_oid()
                .map_err(|e| smime_error("an EC key", e))?;
            if curve == asn1::SECP256R1 {
                return p256::SecretKey::from_pkcs8_der(der)
                    .map(PrivateKey::P256)
                    .map_err(|e| smime_error("a P-256 key", e));
            }
            if curve == asn1::SECP384R1 {
                return p384::SecretKey::from_pkcs8_der(der)
                    .map(PrivateKey::P384)
                    .map_err(|e| smime_error("a P-384 key", e));
            }
            return Err(MimeError::Smime(format!(
                "keys on the curve {curve} are not supported"
            )));
        }
        Err(MimeError::Smime(format!(
            "{} keys are not supported",
            asn1::name(&oid)
        )))
    }

    /// From a PKCS#8 PEM block (`BEGIN PRIVATE KEY`), as the keyring holds it.
    pub fn from_pkcs8_pem(pem: &str) -> Result<PrivateKey, MimeError> {
        let (label, der) =
            der::pem::decode_vec(pem.as_bytes()).map_err(|e| smime_error("a PEM key", e))?;
        let der = Zeroizing::new(der);
        if label != "PRIVATE KEY" {
            return Err(MimeError::Smime(format!(
                "a PEM block labelled {label:?} is not a PKCS#8 private key"
            )));
        }
        PrivateKey::from_pkcs8_der(&der)
    }

    /// PKCS#8 `PrivateKeyInfo`, DER.
    pub fn to_pkcs8_der(&self) -> Result<Zeroizing<Vec<u8>>, MimeError> {
        use pkcs8::EncodePrivateKey;
        let document = match self {
            PrivateKey::Rsa(key) => key.to_pkcs8_der(),
            PrivateKey::P256(key) => key.to_pkcs8_der(),
            PrivateKey::P384(key) => key.to_pkcs8_der(),
        }
        .map_err(|e| smime_error("a key", e))?;
        Ok(Zeroizing::new(document.as_bytes().to_vec()))
    }

    /// A PKCS#8 PEM block, as the keyring keeps it.
    pub fn to_pkcs8_pem(&self) -> Result<Zeroizing<String>, MimeError> {
        let der = self.to_pkcs8_der()?;
        der::pem::encode_string("PRIVATE KEY", der::pem::LineEnding::LF, &der)
            .map(Zeroizing::new)
            .map_err(|e| smime_error("a key", e))
    }

    /// The public half as a subject public key info, DER: what matches it to its certificate.
    pub(crate) fn public_spki_der(&self) -> Vec<u8> {
        use spki::EncodePublicKey;
        match self {
            PrivateKey::Rsa(key) => key.to_public_key().to_public_key_der(),
            PrivateKey::P256(key) => key.public_key().to_public_key_der(),
            PrivateKey::P384(key) => key.public_key().to_public_key_der(),
        }
        .map(|doc| doc.as_bytes().to_vec())
        .unwrap_or_default()
    }

    /// The digest this key signs mail with: SHA-384 for P-384, whose strength SHA-256 would
    /// cap, and SHA-256 for the rest.
    pub(crate) fn digest(&self) -> DigestAlg {
        match self {
            PrivateKey::P384(_) => DigestAlg::Sha384,
            PrivateKey::Rsa(_) | PrivateKey::P256(_) => DigestAlg::Sha256,
        }
    }

    /// A signature over `hash` (made with [`PrivateKey::digest`]), and the algorithm identifier
    /// a SignerInfo names it by.
    pub(crate) fn sign(
        &self,
        hash: &[u8],
    ) -> Result<(Vec<u8>, AlgorithmIdentifierOwned), MimeError> {
        use signature::hazmat::PrehashSigner;
        let failed = |e: &dyn std::fmt::Display| smime_error("signing", e);
        match self {
            PrivateKey::Rsa(key) => {
                let signature = key
                    .sign(rsa::Pkcs1v15Sign::new::<sha2::Sha256>(), hash)
                    .map_err(|e| failed(&e))?;
                // RFC 3370 §3.2: rsaEncryption, with NULL parameters, is what every reader knows.
                Ok((
                    signature,
                    AlgorithmIdentifierOwned {
                        oid: asn1::RSA_ENCRYPTION,
                        parameters: Some(der::Any::null()),
                    },
                ))
            }
            PrivateKey::P256(key) => {
                let signer = p256::ecdsa::SigningKey::from(key);
                let signature: p256::ecdsa::Signature =
                    signer.sign_prehash(hash).map_err(|e| failed(&e))?;
                Ok((
                    signature.to_der().as_bytes().to_vec(),
                    AlgorithmIdentifierOwned {
                        oid: asn1::ECDSA_WITH_SHA256,
                        parameters: None,
                    },
                ))
            }
            PrivateKey::P384(key) => {
                let signer = p384::ecdsa::SigningKey::from(key);
                let signature: p384::ecdsa::Signature =
                    signer.sign_prehash(hash).map_err(|e| failed(&e))?;
                Ok((
                    signature.to_der().as_bytes().to_vec(),
                    AlgorithmIdentifierOwned {
                        oid: asn1::ECDSA_WITH_SHA384,
                        parameters: None,
                    },
                ))
            }
        }
    }

    /// The content-encryption key a key-transport recipient info carries, decrypted.
    ///
    /// RSA only: PKCS#1 v1.5 (RFC 8017 §7.2) or RSAES-OAEP (RFC 8017 §7.1, RFC 4055). The `rsa`
    /// crate's PKCS#1 v1.5 decryption is not constant-time (RUSTSEC-2023-0071, the Marvin
    /// attack); that matters to a server answering strangers' ciphertexts, and here the only
    /// ciphertexts decrypted are the user's own mail, opened on the user's own machine, with no
    /// answer sent anywhere.
    pub(crate) fn unwrap_key(
        &self,
        algorithm: &AlgorithmIdentifierOwned,
        wrapped: &[u8],
    ) -> Result<Zeroizing<Vec<u8>>, String> {
        let PrivateKey::Rsa(key) = self else {
            return Err("that key cannot decrypt a key-transport recipient".to_owned());
        };
        let clear = if algorithm.oid == asn1::RSA_ENCRYPTION {
            key.decrypt(rsa::Pkcs1v15Encrypt, wrapped)
        } else if algorithm.oid == asn1::RSAES_OAEP {
            let params = match &algorithm.parameters {
                Some(any) => any
                    .to_der()
                    .ok()
                    .and_then(|der| asn1::OaepParams::from_der(&der).ok())
                    .ok_or("unreadable RSAES-OAEP parameters")?,
                None => asn1::OaepParams {
                    hash: None,
                    mask_gen: None,
                    label: None,
                },
            };
            key.decrypt(oaep(&params)?, wrapped)
        } else {
            return Err(format!(
                "keys wrapped with {} are not read here",
                asn1::name(&algorithm.oid)
            ));
        };
        clear
            .map(Zeroizing::new)
            .map_err(|_| "the message key could not be decrypted".to_owned())
    }
}

/// The OAEP padding `params` names. The label is always empty in S/MIME (RFC 4055 §4.1).
fn oaep(params: &asn1::OaepParams) -> Result<rsa::Oaep, String> {
    let hash = params
        .hash
        .as_ref()
        .map_or(Ok(DigestAlg::Sha1), |h| DigestAlg::of(&h.oid))?;
    let mgf = match &params.mask_gen {
        None => DigestAlg::Sha1,
        Some(mgf) if mgf.oid == asn1::MGF1 => mgf
            .parameters
            .as_ref()
            .and_then(|p| p.decode_as::<AlgorithmIdentifierOwned>().ok())
            .map_or(Ok(DigestAlg::Sha1), |h| DigestAlg::of(&h.oid))?,
        Some(other) => return Err(format!("mask generation {} is not read here", other.oid)),
    };
    use sha2::{Sha256, Sha384, Sha512};
    Ok(match (hash, mgf) {
        (DigestAlg::Sha1, DigestAlg::Sha1) => rsa::Oaep::new::<sha1::Sha1>(),
        (DigestAlg::Sha256, DigestAlg::Sha256) => rsa::Oaep::new::<Sha256>(),
        (DigestAlg::Sha384, DigestAlg::Sha384) => rsa::Oaep::new::<Sha384>(),
        (DigestAlg::Sha512, DigestAlg::Sha512) => rsa::Oaep::new::<Sha512>(),
        (DigestAlg::Sha256, DigestAlg::Sha1) => {
            rsa::Oaep::new_with_mgf_hash::<Sha256, sha1::Sha1>()
        }
        _ => return Err("that RSAES-OAEP combination is not read here".to_owned()),
    })
}

/// One of the user's S/MIME identities: a private key, its certificate, and the issuers that
/// came with it — what a PKCS#12 file holds, and what signing and decrypting need.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub key: PrivateKey,
    pub cert: Cert,
    /// The issuers' certificates, nearest first. Sent with every signature so the recipient can
    /// build the chain.
    pub chain: Vec<Cert>,
}

impl Identity {
    /// An identity from its parts. Refused when the key is not the certificate's.
    pub fn new(key: PrivateKey, cert: Cert, chain: Vec<Cert>) -> Result<Identity, MimeError> {
        if key.public_spki_der() != cert.spki_der() {
            return Err(MimeError::Smime(
                "that private key does not belong to that certificate".to_owned(),
            ));
        }
        Ok(Identity { key, cert, chain })
    }
}
