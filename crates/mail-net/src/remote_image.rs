//! Fetching a remote image for the reader.
//!
//! A message's remote pictures are fetched only when the reader's consent admits them, and by
//! mailo rather than by any document: the front end asks, with the URL it has already checked,
//! and shows what comes back only if it is a raster image of a kind `mail-mime` embeds
//! ([`Image::data_uri`]).
//!
//! An image is fetched over `https` alone: an `http:` URL is asked for at the same address over
//! `https`, which most image hosts also serve, and is missing where it is not.
//!
//! The client sends no cookies (reqwest's `cookies` feature is off) and no `Referer`; fetching
//! straight or through a proxy it follows at most three redirects, each to a public `https`
//! address; it gives up after twenty seconds. A failed fetch leaves the image missing and says
//! nothing to the person: it is logged.
//!
//! [`ImageFetcher`] is owned state: one client, the [`Route`] its fetches take, and the runtime
//! handle it spawns them on. Nothing is kept between fetches, so a front end holds as many as it
//! has readers.
//!
//! The route is the person's choice of how much an image's host learns ([`route`]): straight,
//! through a relay, or through a proxy. Whichever it is, the image's URL is checked as above
//! first, and what comes back is shown by the same rule.

use crate::NetError;
use crate::error::Failure;
use std::time::Duration;
use tokio::runtime::Handle;

mod route;

pub use route::Route;

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

/// The web, for images: one client, the way its fetches go, and the runtime they run on.
#[derive(Debug, Clone)]
pub struct ImageFetcher {
    client: reqwest::Client,
    route: Route,
    runtime: Handle,
}

impl ImageFetcher {
    /// A fetcher whose fetches go by `route`, on `runtime`.
    pub fn new(runtime: Handle, route: Route) -> Result<Self, NetError> {
        let builder = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .connect_timeout(Duration::from_secs(10))
            .referer(false);
        let builder = match &route {
            Route::Direct => crate::public::guarded(builder)
                .https_only(true)
                .redirect(follow_public()),
            // The proxy resolves the image's host, so the public-only resolver is never asked;
            // the URL and each redirect are still checked by name and address.
            Route::Proxy { url } => builder
                .proxy(reqwest::Proxy::all(url.as_str()).map_err(|e| {
                    NetError::Connect(Failure::new("the proxy is not one to use", e))
                })?)
                .https_only(true)
                .redirect(follow_public()),
            // The relay is the person's, as their mail server is: it is reached as they wrote
            // it, it is checked against the system's authorities as well (`trusts_system`), and
            // it answers with the image, never a redirect to follow.
            Route::Camo { .. } | Route::Token { .. } => builder
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none()),
        };
        let builder = if trusts_system(&route) {
            crate::roots::trusting_system(builder)
        } else {
            builder
        };
        let client = builder
            .build()
            .map_err(|e| NetError::Connect(Failure::new("cannot build an HTTP client", e)))?;
        Ok(ImageFetcher {
            client,
            route,
            runtime,
        })
    }

    /// Fetch `url` on the runtime and call `done` with the image, from a runtime thread. Not
    /// calling it (a failure) leaves the image missing, as a browser shows a broken one.
    pub fn fetch(&self, url: String, done: Box<dyn FnOnce(Image) + Send>) {
        let client = self.client.clone();
        let route = self.route.clone();
        self.runtime.spawn(async move {
            if let Some(image) = get(&client, &route, &url).await {
                done(image);
            }
        });
    }

    /// Fetch `url`, or `None` when it does not come back as an image within [`MAX_BYTES`].
    pub async fn get(&self, url: &str) -> Option<Image> {
        get(&self.client, &self.route, url).await
    }
}

/// Whether a fetch by `route` is checked against the system's certificate authorities as well as
/// the bundled ones: a relay is a server the person set up, so a company's own authority
/// installed on the machine is believed for it as for their mail server. An image's own host, and
/// a proxy that reaches it, are on the public internet, where the bundled roots are the whole
/// story.
fn trusts_system(route: &Route) -> bool {
    matches!(route, Route::Camo { .. } | Route::Token { .. })
}

/// At most three redirects, each to a public `https` address.
fn follow_public() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() >= 3 || !public_https(attempt.url()) {
            attempt.stop()
        } else {
            attempt.follow()
        }
    })
}

/// Whether `url` is one an image may be fetched from: `https:`, on the public internet.
fn public_https(url: &reqwest::Url) -> bool {
    url.scheme() == "https" && crate::public::url_is_public(url)
}

/// `url` as it is fetched: an `http:` one moved to `https:` at the same address, its port `80`
/// dropped for `https`'s own. `None` when it is neither, or not on the public internet.
fn upgraded(url: &str) -> Option<reqwest::Url> {
    // What is not a URL names nothing to fetch.
    let mut url = reqwest::Url::parse(url).ok()?;
    if url.scheme() == "http" {
        if url.port() == Some(80) {
            // Refused only for a URL with no host, which an `http:` one never is.
            url.set_port(None).ok()?;
        }
        // Refused only between a special scheme and another kind, and both are special.
        url.set_scheme("https").ok()?;
    }
    public_https(&url).then_some(url)
}

async fn get(client: &reqwest::Client, route: &Route, url: &str) -> Option<Image> {
    let Some(url) = upgraded(url) else {
        log::debug!("an image is not on a public web address, and was not fetched");
        return None;
    };
    let mut request = client.get(route.asked(&url));
    if let Some(token) = route.bearer() {
        request = request.bearer_auth(token);
    }
    let response = match request.send().await {
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
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let bytes = match crate::http::body_up_to(response, MAX_BYTES).await {
        Ok(bytes) => bytes,
        Err(e) => {
            log::debug!("an image was not read: {e}");
            return None;
        }
    };
    Some(Image {
        content_type,
        bytes,
    })
}

#[cfg(test)]
mod tests;
