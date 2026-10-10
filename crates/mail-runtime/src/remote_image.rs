//! Fetching a remote image for the reader.
//!
//! A message's remote pictures are fetched only when the reader's consent admits them, and by
//! mailo rather than by any document: the front end asks, with the URL it has already checked,
//! and shows what comes back only if it is a raster image of a kind `mail-mime` embeds
//! ([`Image::data_uri`]).
//!
//! The client sends no cookies (reqwest's `cookies` feature is off) and no `Referer`; it follows
//! at most three redirects, which reqwest keeps to `http` and `https`; it gives up after twenty
//! seconds. A failed fetch leaves the image missing and says nothing to the person: it is logged.
//!
//! [`ImageFetcher`] is owned state: one client, and the runtime handle it spawns its fetches on.
//! Nothing is kept between fetches, so a front end holds as many as it has readers.

use crate::RuntimeError;
use crate::error::Failure;
use std::time::Duration;
use tokio::runtime::Handle;

/// The largest image a fetch returns. A picture in a mail is kilobytes; past this it is not a
/// picture.
pub const MAX_BYTES: usize = 16 * 1024 * 1024;

/// A fetched image: what the server said it is, and its bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    /// The `Content-Type` header, as sent.
    pub content_type: Option<String>,
    pub bytes: Vec<u8>,
}

impl Image {
    /// The image as a `data:` URI for an `<img>`, or `None` when it is not one to show: past
    /// [`MAX_BYTES`], declared as something `mail-mime` would not embed (`mail_mime::embeddable`:
    /// PNG, JPEG, GIF, WebP; never SVG), or with first bytes that are not one of those. The type
    /// written is the one the bytes are, in the allowlist's spelling, never the server's string.
    pub fn data_uri(&self) -> Option<String> {
        use base64::Engine as _;
        if self.bytes.is_empty() || self.bytes.len() > MAX_BYTES {
            return None;
        }
        mail_mime::embeddable(self.content_type.as_deref()?)?;
        let kind = mail_mime::embeddable(sniff(&self.bytes)?)?;
        Some(format!(
            "data:{kind};base64,{}",
            base64::engine::general_purpose::STANDARD.encode(&self.bytes)
        ))
    }
}

/// The raster type `bytes` begin as, by their magic number.
fn sniff(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

/// The web, for images: one client, and the runtime its fetches run on.
#[derive(Debug, Clone)]
pub struct ImageFetcher {
    client: reqwest::Client,
    runtime: Handle,
}

impl ImageFetcher {
    /// A fetcher that spawns its fetches on `runtime`.
    pub fn new(runtime: Handle) -> Result<Self, RuntimeError> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .connect_timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::limited(3))
            .referer(false)
            .build()
            .map_err(|e| RuntimeError::Connect(Failure::new("cannot build an HTTP client", e)))?;
        Ok(ImageFetcher { client, runtime })
    }

    /// Fetch `url` on the runtime and call `done` with the image, from a runtime thread. Not
    /// calling it (a failure) leaves the image missing, as a browser shows a broken one.
    pub fn fetch(&self, url: String, done: Box<dyn FnOnce(Image) + Send>) {
        let client = self.client.clone();
        self.runtime.spawn(async move {
            if let Some(image) = get(&client, &url).await {
                done(image);
            }
        });
    }

    /// Fetch `url`, or `None` when it does not come back as an image within [`MAX_BYTES`].
    pub async fn get(&self, url: &str) -> Option<Image> {
        get(&self.client, url).await
    }
}

async fn get(client: &reqwest::Client, url: &str) -> Option<Image> {
    let mut response = match client.get(url).send().await {
        Ok(response) => response,
        Err(e) => {
            log::debug!("an image could not be fetched: {e}");
            return None;
        }
    };
    if !response.status().is_success() {
        log::debug!("an image was refused: {}", response.status());
        return None;
    }
    if response
        .content_length()
        .is_some_and(|len| len > MAX_BYTES as u64)
    {
        log::debug!("an image is past the {MAX_BYTES} bytes a picture may be");
        return None;
    }
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let mut bytes = Vec::new();
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) if bytes.len() + chunk.len() <= MAX_BYTES => {
                bytes.extend_from_slice(&chunk);
            }
            Ok(Some(_)) => {
                log::debug!("an image grew past the {MAX_BYTES} bytes a picture may be");
                return None;
            }
            Err(e) => {
                log::debug!("an image stopped arriving: {e}");
                return None;
            }
            Ok(None) => break,
        }
    }
    Some(Image {
        content_type,
        bytes,
    })
}

#[cfg(test)]
mod tests;
