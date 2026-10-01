//! The fixed address table and the one HTTP request it allows.

use super::{IconError, MAX_BYTES};
use crate::provider::Provider;
use std::time::Duration;

/// The icon address for `provider`.
///
/// [`Provider`] is the only input. A message's `From` domain is not an argument
/// this function can take: fetching an icon for whoever wrote would tell them
/// the message was opened, the same leak as a remote image. [`Provider::Imap`]
/// has no address. Adding a provider is a compile error until this match names one.
pub(crate) fn url(provider: Provider) -> Option<&'static str> {
    match provider {
        Provider::Google => Some("https://ssl.gstatic.com/ui/v1/icons/mail/rfr/gmail.ico"),
        Provider::Microsoft => Some("https://outlook.live.com/favicon.ico"),
        Provider::Fastmail => Some("https://www.fastmail.com/favicon.ico"),
        Provider::Icloud => Some("https://www.icloud.com/favicon.ico"),
        Provider::Yahoo => Some("https://mail.yahoo.com/favicon.ico"),
        Provider::Imap => None,
    }
}

/// The HTTP client an icon fetch uses.
///
/// Five seconds, at most three redirects, https only. There is no cookie jar:
/// this crate does not enable reqwest's `cookies` feature, and nothing here
/// installs one.
pub(crate) fn client() -> Result<reqwest::Client, IconError> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .connect_timeout(Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::limited(3))
        .https_only(true)
        .build()
        .map_err(|err| IconError::Fetch(err.to_string()))
}

/// Fetch the provider's icon bytes. The caller decodes and stores them.
pub(crate) async fn fetch(
    client: &reqwest::Client,
    provider: Provider,
) -> Result<Vec<u8>, IconError> {
    let Some(address) = url(provider) else {
        return Err(IconError::Unmapped);
    };
    if !address.starts_with("https://") {
        return Err(IconError::NotHttps);
    }
    let response = client
        .get(address)
        .send()
        .await
        .map_err(|err| IconError::Fetch(err.to_string()))?;
    if response.url().scheme() != "https" {
        return Err(IconError::NotHttps);
    }
    if !response.status().is_success() {
        return Err(IconError::Status(response.status().as_u16()));
    }
    if let Some(len) = response.content_length()
        && len > MAX_BYTES as u64
    {
        let bytes = usize::try_from(len).unwrap_or(usize::MAX);
        return Err(IconError::TooLarge { bytes });
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|err| IconError::Fetch(err.to_string()))?;
    if bytes.len() > MAX_BYTES {
        return Err(IconError::TooLarge { bytes: bytes.len() });
    }
    Ok(bytes.to_vec())
}
