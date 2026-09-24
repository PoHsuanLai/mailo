//! PKCS#12 (RFC 7292) identity files: a private key and its certificate chain under a password,
//! as certificate authorities and other clients export them.
//!
//! Reading takes what current exporters write — PBES2 with PBKDF2 and AES (RFC 8018), an
//! HMAC-SHA-256 or HMAC-SHA-1 integrity check — and the triple-DES key encryption older ones
//! still use. RC2, which some exporters still use for the certificates, is refused with a word on
//! how to export again; so is PBMAC1 (RFC 9579), not yet read here. Writing produces PBES2 with
//! PBKDF2-HMAC-SHA-256 and AES-256-CBC, and an HMAC-SHA-256 check: what OpenSSL 3 writes and
//! every current reader opens.

use super::asn1::{self, CertBag, SafeBag};
use super::ber::{self, Implicit};
use super::cert::Cert;
use super::keys::{Identity, PrivateKey};
use super::smime_error;
use crate::MimeError;
use crate::openpgp::Rng;
use cms::content_info::ContentInfo;
use cms::encrypted_data::EncryptedData;
use der::asn1::{OctetString, SetOfVec};
use der::{Any, Decode, Encode};
use hmac::{Hmac, Mac};
use pkcs12::kdf::{Pkcs12KeyType, derive_key_utf8};
use pkcs12::mac_data::MacData;
use pkcs12::pfx::{Pfx, Version};
use spki::AlgorithmIdentifierOwned;
use x509_cert::attr::Attribute;
use zeroize::Zeroizing;

/// The iteration count written: the key derivation's work factor. Well above what exporters
/// use by default, and still a fraction of a second.
const ITERATIONS: u32 = 100_000;

/// The identity in a PKCS#12 file: its private key, the certificate that key belongs to, and
/// every other certificate in the file as the chain.
///
/// [`MimeError::WrongPassword`] when the password does not open it; an error naming the algorithm
/// when it is one this client does not read.
pub fn read_pkcs12(bytes: &[u8], password: &str) -> Result<Identity, MimeError> {
    let pfx = ber::decode::<Pfx>(bytes, Implicit::Keep)
        .ok_or_else(|| MimeError::Smime("that is not a PKCS#12 file".to_owned()))?;
    if pfx.auth_safe.content_type != asn1::DATA {
        return Err(MimeError::Smime(
            "a PKCS#12 file protected by a public key is not read here".to_owned(),
        ));
    }
    let safe = octets(&pfx.auth_safe.content)?;
    if let Some(mac) = &pfx.mac_data {
        check_mac(mac, &safe, password)?;
    }
    let contents = ber::decode::<Vec<ContentInfo>>(&safe, Implicit::Keep)
        .ok_or_else(|| MimeError::Smime("the PKCS#12 contents could not be read".to_owned()))?;
    let mut keys: Vec<PrivateKey> = Vec::new();
    let mut certs: Vec<Cert> = Vec::new();
    for info in &contents {
        let bags = match info.content_type {
            asn1::DATA => octets(&info.content)?,
            asn1::ENCRYPTED_DATA => {
                let der = info
                    .content
                    .to_der()
                    .map_err(|e| smime_error("PKCS#12", e))?;
                let data = ber::decode::<EncryptedData>(&der, Implicit::Keep)
                    .or_else(|| ber::decode::<EncryptedData>(&der, Implicit::Join))
                    .ok_or_else(|| MimeError::Smime("unreadable PKCS#12 contents".to_owned()))?;
                let content = &data.enc_content_info;
                let ciphertext = content
                    .encrypted_content
                    .as_ref()
                    .ok_or_else(|| MimeError::Smime("empty PKCS#12 contents".to_owned()))?;
                decrypt(&content.content_enc_alg, ciphertext.as_bytes(), password)?.to_vec()
            }
            other => {
                return Err(MimeError::Smime(format!(
                    "PKCS#12 contents of type {other} are not read here"
                )));
            }
        };
        let bags = ber::decode::<Vec<SafeBag>>(&bags, Implicit::Keep)
            .ok_or_else(|| MimeError::Smime("unreadable PKCS#12 bags".to_owned()))?;
        for bag in bags {
            read_bag(&bag, password, &mut keys, &mut certs)?;
        }
    }
    identity(keys, certs)
}

/// An OCTET STRING's contents, from the ANY it is carried as.
fn octets(any: &Any) -> Result<Vec<u8>, MimeError> {
    let der = any.to_der().map_err(|e| smime_error("PKCS#12", e))?;
    ber::decode::<OctetString>(&der, Implicit::Keep)
        .map(|o| o.as_bytes().to_vec())
        .ok_or_else(|| MimeError::Smime("unreadable PKCS#12 contents".to_owned()))
}

fn read_bag(
    bag: &SafeBag,
    password: &str,
    keys: &mut Vec<PrivateKey>,
    certs: &mut Vec<Cert>,
) -> Result<(), MimeError> {
    let value = bag
        .bag_value
        .to_der()
        .map_err(|e| smime_error("PKCS#12", e))?;
    match bag.bag_id {
        asn1::SHROUDED_KEY_BAG => {
            let shrouded = pkcs12::pbe_params::EncryptedPrivateKeyInfo::from_der(&value)
                .map_err(|e| smime_error("a PKCS#12 key", e))?;
            let clear = decrypt(
                &shrouded.encryption_algorithm,
                shrouded.encrypted_data.as_bytes(),
                password,
            )?;
            keys.push(PrivateKey::from_pkcs8_der(&clear)?);
        }
        asn1::KEY_BAG => keys.push(PrivateKey::from_pkcs8_der(&value)?),
        asn1::CERT_BAG => {
            let bag = CertBag::from_der(&value).map_err(|e| smime_error("a PKCS#12 bag", e))?;
            if bag.cert_id == asn1::X509_CERTIFICATE {
                certs.push(Cert::from_der(bag.cert_value.as_bytes())?);
            }
        }
        // CRLs, secrets, nested contents: nothing an identity needs.
        _ => {}
    }
    Ok(())
}

/// The key and the certificate it belongs to; the other certificates are its chain, nearest
/// issuer first as far as their names say.
fn identity(keys: Vec<PrivateKey>, certs: Vec<Cert>) -> Result<Identity, MimeError> {
    let Some(key) = keys.into_iter().next() else {
        return Err(MimeError::Smime(
            "the PKCS#12 file holds no private key".to_owned(),
        ));
    };
    let public = key.public_spki_der();
    let Some(at) = certs.iter().position(|c| c.spki_der() == public) else {
        return Err(MimeError::Smime(
            "the PKCS#12 file holds no certificate for its key".to_owned(),
        ));
    };
    let mut rest = certs;
    let cert = rest.remove(at);
    // Nearest first: follow issuer names up from the certificate.
    let mut chain = Vec::with_capacity(rest.len());
    let mut current = cert.inner.tbs_certificate.issuer.clone();
    while let Some(next) = rest
        .iter()
        .position(|c| c.inner.tbs_certificate.subject == current)
    {
        let issuer = rest.remove(next);
        let self_signed =
            issuer.inner.tbs_certificate.issuer == issuer.inner.tbs_certificate.subject;
        current = issuer.inner.tbs_certificate.issuer.clone();
        chain.push(issuer);
        if self_signed {
            break;
        }
    }
    chain.extend(rest);
    Identity::new(key, cert, chain)
}

/// The integrity check: an HMAC over the contents, keyed from the password (RFC 7292 App. B).
fn check_mac(mac: &MacData, data: &[u8], password: &str) -> Result<(), MimeError> {
    let oid = mac.mac.algorithm.oid;
    let iterations = mac.iterations;
    let salt = mac.mac_salt.as_bytes();
    let expected = mac.mac.digest.as_bytes();
    macro_rules! check {
        ($digest:ty, $len:expr) => {{
            let key = Zeroizing::new(
                derive_key_utf8::<$digest>(password, salt, Pkcs12KeyType::Mac, iterations, $len)
                    .map_err(|e| smime_error("the PKCS#12 password", e))?,
            );
            let mut hmac = <Hmac<$digest> as Mac>::new_from_slice(&key)
                .map_err(|e| smime_error("PKCS#12", e))?;
            hmac.update(data);
            hmac.verify_slice(expected)
                .map_err(|_| MimeError::WrongPassword)
        }};
    }
    match oid {
        asn1::SHA1 => check!(sha1::Sha1, 20),
        asn1::SHA256 => check!(sha2::Sha256, 32),
        asn1::SHA384 => check!(sha2::Sha384, 48),
        asn1::SHA512 => check!(sha2::Sha512, 64),
        other => Err(MimeError::Smime(format!(
            "a PKCS#12 file checked with {} is not read here",
            asn1::name(&other)
        ))),
    }
}

/// Contents encrypted under the password: PBES2, or PKCS#12's own triple-DES scheme.
fn decrypt(
    algorithm: &AlgorithmIdentifierOwned,
    ciphertext: &[u8],
    password: &str,
) -> Result<Zeroizing<Vec<u8>>, MimeError> {
    match algorithm.oid {
        asn1::PBES2 => {
            let der = algorithm
                .to_der()
                .map_err(|e| smime_error("PKCS#12 encryption", e))?;
            let scheme = pkcs5::EncryptionScheme::try_from(der.as_slice())
                .map_err(|e| smime_error("PKCS#12 encryption", e))?;
            scheme
                .decrypt(password.as_bytes(), ciphertext)
                .map(Zeroizing::new)
                .map_err(|_| MimeError::WrongPassword)
        }
        asn1::PBE_SHA1_3DES => {
            use cbc::cipher::{BlockDecryptMut, KeyIvInit, block_padding::Pkcs7};
            let params = algorithm
                .parameters
                .as_ref()
                .and_then(|p| p.decode_as::<pkcs12::pbe_params::Pkcs12PbeParams>().ok())
                .ok_or_else(|| MimeError::Smime("unreadable PKCS#12 parameters".to_owned()))?;
            let derive = |kind, len| {
                derive_key_utf8::<sha1::Sha1>(
                    password,
                    params.salt.as_bytes(),
                    kind,
                    params.iterations,
                    len,
                )
                .map(Zeroizing::new)
                .map_err(|e| smime_error("the PKCS#12 password", e))
            };
            let key = derive(Pkcs12KeyType::EncryptionKey, 24)?;
            let iv = derive(Pkcs12KeyType::Iv, 8)?;
            cbc::Decryptor::<des::TdesEde3>::new_from_slices(&key, &iv)
                .map_err(|e| smime_error("PKCS#12", e))?
                .decrypt_padded_vec_mut::<Pkcs7>(ciphertext)
                .map(Zeroizing::new)
                .map_err(|_| MimeError::WrongPassword)
        }
        other => Err(MimeError::Smime(format!(
            "this PKCS#12 file is encrypted with {}, which is not read here; export it again \
             with AES (OpenSSL 3's default, or `openssl pkcs12 -export -certpbe AES-256-CBC \
             -keypbe AES-256-CBC`)",
            asn1::name(&other)
        ))),
    }
}

/// `identity` as a PKCS#12 file under `password`: the key encrypted, the certificates in the
/// clear beside it (they are public), and an integrity check over both.
pub fn write_pkcs12(
    identity: &Identity,
    password: &str,
    rng: &mut impl Rng,
) -> Result<Vec<u8>, MimeError> {
    let local_id = attribute_set(identity.cert.fingerprint().0.to_vec())?;
    let pkcs8 = identity.key.to_pkcs8_der()?;
    let mut salt = [0u8; 16];
    let mut iv = [0u8; 16];
    rng.fill_bytes(&mut salt);
    rng.fill_bytes(&mut iv);
    let params = pkcs5::pbes2::Parameters::pbkdf2_sha256_aes256cbc(ITERATIONS, &salt, &iv)
        .map_err(|e| smime_error("PKCS#12", e))?;
    let scheme = pkcs5::EncryptionScheme::from(params);
    let encrypted = scheme
        .encrypt(password.as_bytes(), &pkcs8)
        .map_err(|e| smime_error("PKCS#12", e))?;
    let scheme_der = scheme.to_der().map_err(|e| smime_error("PKCS#12", e))?;
    let shrouded = pkcs12::pbe_params::EncryptedPrivateKeyInfo {
        encryption_algorithm: AlgorithmIdentifierOwned::from_der(&scheme_der)
            .map_err(|e| smime_error("PKCS#12", e))?,
        encrypted_data: OctetString::new(encrypted).map_err(|e| smime_error("PKCS#12", e))?,
    };
    let key_bag = SafeBag {
        bag_id: asn1::SHROUDED_KEY_BAG,
        bag_value: any(&shrouded)?,
        bag_attributes: Some(local_id.clone()),
    };
    let mut cert_bags = Vec::new();
    for (index, cert) in std::iter::once(&identity.cert)
        .chain(&identity.chain)
        .enumerate()
    {
        let bag = CertBag {
            cert_id: asn1::X509_CERTIFICATE,
            cert_value: OctetString::new(cert.der().to_vec())
                .map_err(|e| smime_error("PKCS#12", e))?,
        };
        cert_bags.push(SafeBag {
            bag_id: asn1::CERT_BAG,
            bag_value: any(&bag)?,
            bag_attributes: (index == 0).then(|| local_id.clone()),
        });
    }
    let data = |bags: &Vec<SafeBag>| -> Result<ContentInfo, MimeError> {
        let der = bags.to_der().map_err(|e| smime_error("PKCS#12", e))?;
        Ok(ContentInfo {
            content_type: asn1::DATA,
            content: any(&OctetString::new(der).map_err(|e| smime_error("PKCS#12", e))?)?,
        })
    };
    let safe = vec![data(&cert_bags)?, data(&vec![key_bag])?]
        .to_der()
        .map_err(|e| smime_error("PKCS#12", e))?;
    let mut mac_salt = [0u8; 16];
    rng.fill_bytes(&mut mac_salt);
    let iterations = i32::try_from(ITERATIONS).unwrap_or(i32::MAX);
    let key = Zeroizing::new(
        derive_key_utf8::<sha2::Sha256>(password, &mac_salt, Pkcs12KeyType::Mac, iterations, 32)
            .map_err(|e| smime_error("the PKCS#12 password", e))?,
    );
    let mut hmac =
        <Hmac<sha2::Sha256> as Mac>::new_from_slice(&key).map_err(|e| smime_error("PKCS#12", e))?;
    hmac.update(&safe);
    let digest = hmac.finalize().into_bytes().to_vec();
    let pfx = Pfx {
        version: Version::V3,
        auth_safe: ContentInfo {
            content_type: asn1::DATA,
            content: any(&OctetString::new(safe).map_err(|e| smime_error("PKCS#12", e))?)?,
        },
        mac_data: Some(MacData {
            mac: pkcs12::digest_info::DigestInfo {
                algorithm: AlgorithmIdentifierOwned {
                    oid: asn1::SHA256,
                    parameters: None,
                },
                digest: OctetString::new(digest).map_err(|e| smime_error("PKCS#12", e))?,
            },
            mac_salt: OctetString::new(mac_salt.to_vec()).map_err(|e| smime_error("PKCS#12", e))?,
            iterations,
        }),
    };
    pfx.to_der().map_err(|e| smime_error("PKCS#12", e))
}

fn any<T: Encode>(value: &T) -> Result<Any, MimeError> {
    let der = value.to_der().map_err(|e| smime_error("PKCS#12", e))?;
    Any::from_der(&der).map_err(|e| smime_error("PKCS#12", e))
}

/// The bag attributes naming which key goes with which certificate (`localKeyId`).
fn attribute_set(id: Vec<u8>) -> Result<SetOfVec<Attribute>, MimeError> {
    let mut values = SetOfVec::new();
    values
        .insert(any(
            &OctetString::new(id).map_err(|e| smime_error("PKCS#12", e))?
        )?)
        .map_err(|e| smime_error("PKCS#12", e))?;
    let mut set = SetOfVec::new();
    set.insert(Attribute {
        oid: asn1::LOCAL_KEY_ID,
        values,
    })
    .map_err(|e| smime_error("PKCS#12", e))?;
    Ok(set)
}
