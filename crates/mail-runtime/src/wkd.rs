//! Web Key Directory lookup: an address's key, asked of the address's own domain over HTTPS.
//!
//! Where to ask is `mail_mime::openpgp::wkd::urls`; this does the asking. The advanced method
//! first, then the direct method — the draft says to fall back when the `openpgpkey` host does
//! not exist, and a host that exists but has no key for the address is treated the same way,
//! since a domain that moved its directory leaves the old one empty rather than gone. HTTPS
//! only, redirects included: a key fetched in the clear could be anyone's.

use crate::RuntimeError;
use mail_mime::openpgp::Cert;
use mail_mime::openpgp::wkd::{WkdUrls, key_from_answer, urls};
use reqwest::redirect::Policy;
use std::time::Duration;

/// How long one request may take. Someone is composing and waiting on the answer.
pub const TIMEOUT: Duration = Duration::from_secs(10);

/// The largest answer read: a directory serving more than this is not serving one person's key.
const MAX_ANSWER: usize = 1024 * 1024;

/// A client for WKD requests: HTTPS only, [`TIMEOUT`], at most three redirects.
///
/// A builder so a test can add a root it trusts and point the names at a local listener.
pub fn client_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .timeout(TIMEOUT)
        .connect_timeout(TIMEOUT)
        .https_only(true)
        .redirect(Policy::limited(3))
}

/// [`client_builder`], built.
pub fn client() -> Result<reqwest::Client, RuntimeError> {
    client_builder()
        .build()
        .map_err(|e| RuntimeError::Connect(format!("cannot build an HTTP client: {e}")))
}

/// `address`'s key from its domain's Web Key Directory. `Ok(None)` when neither method has
/// one; an error only when the address cannot be looked up at all.
pub async fn lookup(http: &reqwest::Client, address: &str) -> Result<Option<Cert>, RuntimeError> {
    let places = urls(address).ok_or_else(|| {
        RuntimeError::UnsupportedIo(format!("{address:?} is not an address to look up"))
    })?;
    Ok(fetch(http, &places, address).await)
}

/// The key at `places` for `address`: the advanced method, then the direct one.
///
/// Every failure — no such host, a refused connection, a `404`, an answer holding no key for the
/// address — is "not here", and the next place is tried. The caller learns only whether a key
/// was found, which is the one thing it can act on.
pub async fn fetch(http: &reqwest::Client, places: &WkdUrls, address: &str) -> Option<Cert> {
    for url in [&places.advanced, &places.direct] {
        if let Some(cert) = one(http, url, address).await {
            return Some(cert);
        }
    }
    None
}

async fn one(http: &reqwest::Client, url: &str, address: &str) -> Option<Cert> {
    let response = http.get(url).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    if response
        .content_length()
        .is_some_and(|len| len > MAX_ANSWER as u64)
    {
        return None;
    }
    let body = response.bytes().await.ok()?;
    if body.len() > MAX_ANSWER {
        return None;
    }
    key_from_answer(&body, address)
}
