//! Brand logos (BIMI): the user's switch, and whether a message may ask for its sender's logo.
//!
//! Off unless turned on: the window's switch (`reading.brand_logos` in mail-app's
//! `mailo/settings.toml`; `bimi.json` before it). While it is off nothing about logos is looked up, fetched or read from the
//! cache: [`brand_logo`] returns before anything else, and the window checks it before it even
//! makes a resolver.
//!
//! With it on, a message asks only when the receiving server's believed
//! `Authentication-Results` (`crate::auth`, FINDINGS F161) says DMARC passed for its From domain.
//! The rest — the record, the policy at enforcement, the mark certificate, the drawing and the
//! cache — is `mail_runtime::bimi`'s. The certificate must chain to a mark verifying authority's
//! root: those shipped in `roots.pem`, and any the user keeps in `bimi-roots.pem` beside
//! `bimi.json`.

use mail_mime::AuthResults;
use mail_mime::smime::{Cert, read_certs};
/// What a front end needs to ask for a logo: the lookup's seams, the cache's reader and the HTTP
/// client builder, and the system resolver to fill the DNS seam with.
pub use mail_runtime::bimi::{Cached, Lookup, cached, client_builder};
use mail_runtime::bimi::{Txt, logo};
pub use mail_runtime::lookup::SystemDns;
use std::path::Path;

/// Whether brand logos are shown. Off unless someone turned it on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Setting {
    On,
    #[default]
    Off,
}

const FILE_NAME: &str = "bimi.json";

/// The roots a user adds, in the config directory.
pub const USER_ROOTS: &str = "bimi-roots.pem";

/// The roots shipped with mailo.
const SHIPPED_ROOTS: &str = include_str!("roots.pem");

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct Stored {
    #[serde(default)]
    bimi: Setting,
}

/// What `bimi.json` said, or off when there is none or it cannot be read. Never written:
/// mail-app keeps the switch (`reading.brand_logos`) and read this once to start from it.
pub fn load(dir: &Path) -> Setting {
    crate::config::read_json::<Stored>(dir, FILE_NAME).bimi
}

/// The mark verifying authorities' roots: the shipped ones and those in `dir`'s
/// [`USER_ROOTS`]. A file that does not read adds none.
pub fn anchors(dir: &Path) -> Vec<Cert> {
    let mut out = pem_certs(SHIPPED_ROOTS.as_bytes());
    if let Ok(bytes) = std::fs::read(dir.join(USER_ROOTS)) {
        out.extend(pem_certs(&bytes));
    }
    out
}

fn pem_certs(bytes: &[u8]) -> Vec<Cert> {
    if !bytes
        .windows(b"-----BEGIN CERTIFICATE-----".len())
        .any(|w| w == b"-----BEGIN CERTIFICATE-----")
    {
        return Vec::new();
    }
    read_certs(bytes).unwrap_or_default()
}

/// The domain of `address`, when it has one: after the last `@`.
pub fn domain_of(address: &str) -> Option<&str> {
    let (_, domain) = address.trim().rsplit_once('@')?;
    (!domain.is_empty()).then_some(domain)
}

/// The logo of the sender at `from`, as a PNG, when every condition holds: the setting is on,
/// the believed `results` say DMARC passed for `from`'s domain, and `mail_runtime::bimi::logo`
/// finds a certified one (from the cache under `cache_dir`, or asked of `lookup`).
///
/// Nothing is looked up, fetched or read unless the first two hold.
pub async fn brand_logo<D: Txt + Sync>(
    setting: Setting,
    results: Option<&AuthResults>,
    from: &str,
    lookup: &Lookup<'_, D>,
    cache_dir: &Path,
) -> Option<Vec<u8>> {
    if setting == Setting::Off {
        return None;
    }
    let domain = domain_of(from)?;
    if !mail_mime::bimi::dmarc_passed_for(results?, domain) {
        return None;
    }
    logo(lookup, cache_dir, domain).await
}

#[cfg(test)]
mod tests;
