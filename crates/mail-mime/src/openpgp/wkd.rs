//! Web Key Directory (draft-koch-openpgp-webkey-service): where an address's domain publishes
//! the address's key, over HTTPS.
//!
//! Two places to look. The advanced method asks a host of its own,
//! `openpgpkey.<domain>`, under `/.well-known/openpgpkey/<domain>/hu/<hash>`; the direct method
//! asks the domain itself under `/.well-known/openpgpkey/hu/<hash>`. `<hash>` is the z-base-32
//! encoding of the SHA-1 of the address's local part, lower-cased. The fetching is
//! `mail-runtime`'s; this is only where to fetch from, and what the answer means.

use super::keys::{Cert, ReadKey, read_keys};
use sha1::{Digest, Sha1};

/// The two URLs to try for one address, advanced method first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WkdUrls {
    /// `https://openpgpkey.<domain>/.well-known/openpgpkey/<domain>/hu/<hash>?l=<local>`
    pub advanced: String,
    /// `https://<domain>/.well-known/openpgpkey/hu/<hash>?l=<local>`
    pub direct: String,
    /// The host the advanced method asks: `openpgpkey.<domain>`.
    pub advanced_host: String,
    /// The host the direct method asks: the domain.
    pub direct_host: String,
}

/// Where `address`'s key would be published. `None` for something that is not an address.
///
/// The domain is lower-cased, and must be plain DNS labels — an address naming an IP literal or
/// carrying characters a host name cannot is not one to go asking the web about.
pub fn urls(address: &str) -> Option<WkdUrls> {
    let (local, domain) = address.trim().rsplit_once('@')?;
    let domain = domain.to_ascii_lowercase();
    if local.is_empty() || !crate::build::is_domain(&domain) || !domain.contains('.') {
        return None;
    }
    let hash = zbase32(&Sha1::digest(local.to_lowercase().as_bytes()));
    let query = percent(local);
    Some(WkdUrls {
        advanced: format!(
            "https://openpgpkey.{domain}/.well-known/openpgpkey/{domain}/hu/{hash}?l={query}"
        ),
        direct: format!("https://{domain}/.well-known/openpgpkey/hu/{hash}?l={query}"),
        advanced_host: format!("openpgpkey.{domain}"),
        direct_host: domain,
    })
}

/// The key a Web Key Directory answered with, for `address`.
///
/// The answer is binary and may hold several keys; only one with a user id for `address` is
/// the address's (the draft's §3.1 says the client must check). `None` when none is.
pub fn key_from_answer(body: &[u8], address: &str) -> Option<Cert> {
    let address = address.trim().to_ascii_lowercase();
    read_keys(body)
        .ok()?
        .into_iter()
        .map(|key| match key {
            ReadKey::Public(cert) => *cert,
            // A directory serving secret keys is badly broken; the public half is all that is
            // taken from it.
            ReadKey::Secret(secret) => secret.public(),
        })
        .find(|cert| cert.emails().contains(&address))
}

/// z-base-32 (Zooko's human-oriented base-32), as the draft uses it: no padding.
pub fn zbase32(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 32] = b"ybndrfg8ejkmcpqxot1uwisza345h769";
    let mut out = String::with_capacity(bytes.len() * 8 / 5 + 1);
    let mut buffer: u32 = 0;
    let mut bits = 0;
    for &byte in bytes {
        buffer = (buffer << 8) | u32::from(byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(char::from(ALPHABET[((buffer >> bits) & 31) as usize]));
        }
    }
    if bits > 0 {
        out.push(char::from(ALPHABET[((buffer << (5 - bits)) & 31) as usize]));
    }
    out
}

/// The local part as a query value: every byte outside RFC 3986's unreserved set escaped.
fn percent(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                char::from(b).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_drafts_example_address_hashes_to_the_drafts_example() {
        // draft-koch-openpgp-webkey-service §3.1: "Joe.Doe@Example.ORG" is looked up by the
        // hash of "joe.doe", which is "iy9q119eutrkn8s1mk4r39qejnbu3n5q".
        let found = urls("Joe.Doe@Example.ORG").unwrap();
        assert_eq!(
            found.advanced,
            "https://openpgpkey.example.org/.well-known/openpgpkey/example.org/hu/\
             iy9q119eutrkn8s1mk4r39qejnbu3n5q?l=Joe.Doe"
        );
        assert_eq!(
            found.direct,
            "https://example.org/.well-known/openpgpkey/hu/iy9q119eutrkn8s1mk4r39qejnbu3n5q?l=Joe.Doe"
        );
        assert_eq!(found.advanced_host, "openpgpkey.example.org");
        assert_eq!(found.direct_host, "example.org");
    }

    #[test]
    fn z_base_32_matches_known_encodings() {
        // Vectors from the z-base-32 description: a single zero byte and a single 0xF0.
        assert_eq!(zbase32(&[0x00]), "yy");
        assert_eq!(zbase32(&[0xF0, 0xBF, 0xC7]), "6n9hq");
        assert_eq!(zbase32(&[]), "");
    }

    #[test]
    fn what_is_not_an_address_asks_no_one() {
        for bad in [
            "nobody",
            "@example.org",
            "a@",
            "a@localhost",
            "a@[127.0.0.1]",
            "a@ex ample.org",
        ] {
            assert_eq!(urls(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_local_part_is_escaped_in_the_query() {
        let found = urls("a+b@example.org").unwrap();
        assert!(found.direct.ends_with("?l=a%2Bb"), "{}", found.direct);
    }
}
