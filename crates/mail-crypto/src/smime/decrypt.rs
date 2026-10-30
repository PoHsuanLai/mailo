//! Encryption opened: enveloped and auth-enveloped data decrypted with the user's key, and the
//! content algorithms refused or read.

use super::asn1;
use super::open::{Keys, auth_enveloped, enveloped, ids};
use chrono::{DateTime, TimeZone, Utc};
use cms::content_info::ContentInfo;
use cms::enveloped_data::RecipientInfo;
use der::Encode;
use der::asn1::OctetString;
use mail_domain::SmimeEncryption;
use spki::AlgorithmIdentifierOwned;
use zeroize::Zeroizing;

/// Enveloped or auth-enveloped data decrypted with whichever of the user's identities it names.
pub(super) fn decrypt(
    info: &ContentInfo,
    keys: &Keys<'_>,
    date: Option<DateTime<Utc>>,
) -> Result<Zeroizing<Vec<u8>>, SmimeEncryption> {
    let unreadable = |why: &str| SmimeEncryption::Unreadable {
        why: why.to_owned(),
    };
    let (infos, content, auth) = if info.content_type == asn1::AUTH_ENVELOPED_DATA {
        let data = auth_enveloped(info).ok_or_else(|| unreadable("unreadable encryption"))?;
        let aad = match &data.auth_attrs {
            Some(attrs) => attrs
                .to_der()
                .map_err(|_| unreadable("unreadable authenticated attributes"))?,
            None => Vec::new(),
        };
        (
            data.recip_infos,
            data.auth_encrypted_content_info,
            Some((data.mac.as_bytes().to_vec(), aad)),
        )
    } else {
        let data = enveloped(info).ok_or_else(|| unreadable("unreadable encryption"))?;
        (data.recip_infos, data.encrypted_content, None)
    };
    let to = ids(&infos);
    let mut why = None;
    for (ri, id) in infos.0.iter().zip(&to) {
        let RecipientInfo::Ktri(ktri) = ri else {
            continue;
        };
        for identity in keys.identities.iter().filter(|i| id.names(&i.cert)) {
            match identity
                .key
                .unwrap_key(&ktri.key_enc_alg, ktri.enc_key.as_bytes())
            {
                Ok(cek) => {
                    let ciphertext = content
                        .encrypted_content
                        .as_ref()
                        .map(OctetString::as_bytes)
                        .ok_or_else(|| unreadable("the encrypted content is not in the message"))?;
                    return content_decrypt(
                        &content.content_enc_alg,
                        &cek,
                        ciphertext,
                        auth.as_ref(),
                        date,
                    )
                    .map_err(|why| unreadable(&why));
                }
                Err(e) => why = Some(e),
            }
        }
    }
    Err(match why {
        Some(why) => unreadable(&why),
        None => SmimeEncryption::CannotDecrypt {
            to: to.iter().map(ToString::to_string).collect(),
        },
    })
}

/// When triple DES stops being read: NIST disallowed it for encryption after 2023 (SP 800-131A
/// rev. 2), so mail dated later that uses it was made by something that should not have.
fn tdes_cutoff() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0)
        .single()
        .unwrap_or(DateTime::<Utc>::MIN_UTC)
}

fn content_decrypt(
    algorithm: &AlgorithmIdentifierOwned,
    key: &[u8],
    ciphertext: &[u8],
    auth: Option<&(Vec<u8>, Vec<u8>)>,
    date: Option<DateTime<Utc>>,
) -> Result<Zeroizing<Vec<u8>>, String> {
    use cbc::cipher::{BlockDecryptMut, KeyIvInit, block_padding::Pkcs7};
    let damaged = |_| "the content could not be decrypted: damaged, or not for this key".to_owned();
    let iv = || -> Result<Vec<u8>, String> {
        algorithm
            .parameters
            .as_ref()
            .and_then(|p| p.decode_as::<OctetString>().ok())
            .map(|o| o.as_bytes().to_vec())
            .ok_or_else(|| "the encryption states no initialisation vector".to_owned())
    };
    macro_rules! cbc {
        ($cipher:ty) => {
            cbc::Decryptor::<$cipher>::new_from_slices(key, &iv()?)
                .map_err(|_| "the message key is the wrong size".to_owned())?
                .decrypt_padded_vec_mut::<Pkcs7>(ciphertext)
                .map(Zeroizing::new)
                .map_err(damaged)
        };
    }
    match algorithm.oid {
        asn1::AES128_CBC => cbc!(aes::Aes128),
        asn1::AES192_CBC => cbc!(aes::Aes192),
        asn1::AES256_CBC => cbc!(aes::Aes256),
        asn1::DES_EDE3_CBC => {
            if date.is_none_or(|d| d >= tdes_cutoff()) {
                return Err(
                    "encrypted with triple DES, which is refused for mail sent after 2023"
                        .to_owned(),
                );
            }
            cbc!(des::TdesEde3)
        }
        asn1::AES128_GCM | asn1::AES192_GCM | asn1::AES256_GCM => {
            let (mac, aad) = auth.ok_or("AES-GCM outside authenticated enveloped data")?;
            gcm(algorithm, key, ciphertext, mac, aad)
        }
        other => Err(format!(
            "encrypted with {}, which is refused as too weak or not read here",
            asn1::name(&other)
        )),
    }
}

/// AES-GCM (RFC 5084): the ciphertext and its tag checked together.
fn gcm(
    algorithm: &AlgorithmIdentifierOwned,
    key: &[u8],
    ciphertext: &[u8],
    mac: &[u8],
    aad: &[u8],
) -> Result<Zeroizing<Vec<u8>>, String> {
    use aes_gcm::aead::consts::{U12, U16};
    use aes_gcm::aead::{Aead, KeyInit, Payload};
    use aes_gcm::{AesGcm, Nonce};
    let params = algorithm
        .parameters
        .as_ref()
        .and_then(|p| p.decode_as::<asn1::GcmParameters>().ok())
        .ok_or("unreadable AES-GCM parameters")?;
    if params.nonce.as_bytes().len() != 12 {
        return Err("an AES-GCM nonce of other than 12 bytes is not read here".to_owned());
    }
    if usize::from(params.icv_len) != mac.len() {
        return Err("the AES-GCM tag is not the length its parameters state".to_owned());
    }
    let nonce = Nonce::<U12>::from_slice(params.nonce.as_bytes());
    let mut sealed = ciphertext.to_vec();
    sealed.extend_from_slice(mac);
    let payload = Payload { msg: &sealed, aad };
    let failed = |_| "the content failed its integrity check: damaged or tampered with".to_owned();
    macro_rules! open {
        ($cipher:ty, $tag:ty) => {
            AesGcm::<$cipher, U12, $tag>::new_from_slice(key)
                .map_err(|_| "the message key is the wrong size".to_owned())?
                .decrypt(nonce, payload)
                .map(Zeroizing::new)
                .map_err(failed)
        };
    }
    match (algorithm.oid, mac.len()) {
        (asn1::AES128_GCM, 16) => open!(aes::Aes128, U16),
        (asn1::AES192_GCM, 16) => open!(aes::Aes192, U16),
        (asn1::AES256_GCM, 16) => open!(aes::Aes256, U16),
        (asn1::AES128_GCM, 12) => open!(aes::Aes128, U12),
        (asn1::AES192_GCM, 12) => open!(aes::Aes192, U12),
        (asn1::AES256_GCM, 12) => open!(aes::Aes256, U12),
        _ => Err("that AES-GCM tag length is not read here".to_owned()),
    }
}
