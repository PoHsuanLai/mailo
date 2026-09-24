//! Autocrypt Level 1 headers: `Autocrypt:` on the outside of a message, `Autocrypt-Gossip:`
//! inside an encrypted one.
//!
//! A header is `attribute=value` pairs separated by `;`: `addr`, optionally `prefer-encrypt`,
//! and `keydata`, the sender's public key in base64. An attribute whose name begins with `_` is
//! optional and ignored; any other unknown attribute makes the whole header unusable, which is
//! how the spec leaves room to add critical ones later.

use super::entity::{Entity, fields, is_named, value};
use super::keys::{Cert, ReadKey, read_keys};
use crate::stamp;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use mail_domain::PreferEncrypt;

/// One usable Autocrypt or Autocrypt-Gossip header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutocryptHeader {
    /// The address the key is for, lower-cased.
    pub addr: String,
    pub prefer_encrypt: PreferEncrypt,
    pub key: Cert,
}

/// The one usable `Autocrypt:` header of a message from `from`, if it has exactly one.
///
/// Ignored, as the spec says, when there are several, when its `addr` is not the `From`
/// address, and on a `multipart/report` — a bounce or a receipt carries the original's headers
/// and speaks for nobody.
pub fn autocrypt_of(raw: &[u8], from: &str) -> Option<AutocryptHeader> {
    let entity = Entity::of(raw);
    if entity.content_type().mime == "multipart/report" {
        return None;
    }
    let found: Vec<AutocryptHeader> = fields(entity.head)
        .into_iter()
        .filter(|field| is_named(field, "Autocrypt"))
        .filter_map(|field| attributes(&value(field)))
        .collect();
    let [only] = <[AutocryptHeader; 1]>::try_from(found).ok()?;
    only.addr.eq_ignore_ascii_case(from.trim()).then_some(only)
}

/// Every usable `Autocrypt-Gossip:` header in a header block — the decrypted part's.
pub(super) fn gossip_of(head: &[u8]) -> Vec<AutocryptHeader> {
    fields(head)
        .into_iter()
        .filter(|field| is_named(field, "Autocrypt-Gossip"))
        .filter_map(|field| attributes(&value(field)))
        .collect()
}

fn attributes(text: &str) -> Option<AutocryptHeader> {
    let mut addr = None;
    let mut prefer_encrypt = PreferEncrypt::NoPreference;
    let mut keydata = None;
    for pair in text.split(';') {
        let pair = pair.trim();
        if pair.is_empty() {
            continue;
        }
        let (name, value) = pair.split_once('=')?;
        let (name, value) = (name.trim(), value.trim());
        match name.to_ascii_lowercase().as_str() {
            "addr" => addr = Some(value.to_ascii_lowercase()),
            "prefer-encrypt" if value.eq_ignore_ascii_case("mutual") => {
                prefer_encrypt = PreferEncrypt::Mutual;
            }
            "prefer-encrypt" => {}
            "keydata" => keydata = Some(value),
            other if other.starts_with('_') => {}
            _ => return None,
        }
    }
    let compact: String = keydata?.chars().filter(|c| !c.is_whitespace()).collect();
    let bytes = STANDARD.decode(compact).ok()?;
    let key = match read_keys(&bytes).ok()?.into_iter().next()? {
        ReadKey::Public(cert) => *cert,
        // A secret key in a header is a sender's accident; taking only its public half is the
        // most that should be done with it.
        ReadKey::Secret(secret) => secret.public(),
    };
    Some(AutocryptHeader {
        addr: addr.filter(|a| a.contains('@'))?,
        prefer_encrypt,
        key,
    })
}

/// The header field `name: addr=…; [prefer-encrypt=mutual;] keydata=…`, folded, ending CRLF.
///
/// `name` is `Autocrypt` or `Autocrypt-Gossip`. The key data is folded into 76-column lines so
/// the field stays inside RFC 5322's line limit whatever the key's size.
pub fn autocrypt_field(
    name: &str,
    addr: &str,
    prefer_encrypt: PreferEncrypt,
    key: &Cert,
) -> Vec<u8> {
    let mut out = format!("{name}: addr={}; ", addr.trim());
    if prefer_encrypt == PreferEncrypt::Mutual {
        out.push_str("prefer-encrypt=mutual; ");
    }
    out.push_str("keydata=");
    let encoded = STANDARD.encode(key.to_bytes());
    for chunk in encoded.as_bytes().chunks(76) {
        out.push_str("\r\n ");
        // Base64 is ASCII, so any byte boundary is a character boundary.
        out.push_str(std::str::from_utf8(chunk).unwrap_or_default());
    }
    out.push_str("\r\n");
    out.into_bytes()
}

/// `raw` with `field` added as the last field of its header block. Nothing else moves.
pub fn with_field(raw: &[u8], field: &[u8]) -> Vec<u8> {
    let (head, rest) = stamp::split_head(raw);
    let mut out = Vec::with_capacity(raw.len() + field.len());
    out.extend_from_slice(head);
    out.extend_from_slice(field);
    out.extend_from_slice(rest);
    out
}
