//! The Original frame's network on Blitz: quire's `NetPolicy::Custom`, answered by mailo.
//!
//! ds-native puts every request a frame makes (anything but inline `data:`) to [`MailNet`], and
//! every request the window's own document makes beyond its `file:` and `data:`. The answer:
//! - **The window's own document: refused.** mailo's markup names no remote resource, and a
//!   request from the app document cannot be told apart by who drew it: a hover card and the
//!   reader are the same document. A remote image is never fetched because something was hovered.
//! - **A frame: refused unless the reader's [`Consent`] admits it**: `http`/`https`, on the
//!   consented thread's allowlist, for one message per frame (`consent.rs`). `file:`, `cid:` and
//!   every other scheme are refused whatever a list says.
//! - **Admitted: fetched by [`Fetch`]**, and handed to the frame only if the consent still stands
//!   when the bytes land.
//!
//! The Reader view's remote images are in the window's own document, so the refusal above holds
//! them too. mailo fetches those itself instead (`reading/remote.rs`), through [`FetchImage`] on
//! the same client, and hands the window each as a `data:` URI ([`data_uri`]) only when it is a
//! raster image of a kind `mail-mime` embeds, by its declared type and by its first bytes.

use super::consent::Consent;
use ds_native::{AppNet, FrameId, NetDecision, NetReply, NetRequest, RequestOrigin};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

/// Whatever fetches an admitted image: the web in the window, a recorder in a test.
pub trait Fetch: Send + Sync + 'static {
    /// Fetch `url` and call `done` with its bytes, from any thread. Not calling it (a failure)
    /// leaves the image missing, as a browser shows a broken one.
    fn get(&self, url: String, done: Box<dyn FnOnce(Vec<u8>) + Send>);
}

/// A fetched image: what the server said it is, and its bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Got {
    /// The `Content-Type` header, as sent.
    pub content_type: Option<String>,
    pub bytes: Vec<u8>,
}

/// Whatever fetches the Reader view's consented images: the web in the window, a recorder in a
/// test. mailo asks; no document does.
pub trait FetchImage: Send + Sync + 'static {
    /// Fetch `url` and call `done` with what came back, from any thread. Not calling it (a
    /// failure) leaves the image unshown.
    fn get(&self, url: String, done: Box<dyn FnOnce(Got) + Send>);
}

/// Fetches nothing: the Reader view's images for a harness that did not ask for a fetcher.
pub(crate) struct Refuse;

impl FetchImage for Refuse {
    fn get(&self, _: String, _: Box<dyn FnOnce(Got) + Send>) {}
}

/// `got` as a `data:` URI for an `<img>`, or `None` when it is not one to show: past
/// [`MAX_BYTES`], declared as something `mail-mime` would not embed (`mail_mime::embeddable`:
/// PNG, JPEG, GIF, WebP; never SVG), or with first bytes that are not one of those. The type
/// written is the one the bytes are, in the allowlist's spelling, never the server's string.
pub(crate) fn data_uri(got: &Got) -> Option<String> {
    use base64::Engine as _;
    if got.bytes.is_empty() || got.bytes.len() > MAX_BYTES {
        return None;
    }
    mail_mime::embeddable(got.content_type.as_deref()?)?;
    let kind = mail_mime::embeddable(sniff(&got.bytes)?)?;
    Some(format!(
        "data:{kind};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(&got.bytes)
    ))
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

/// mailo's `AppNet`.
pub(crate) struct MailNet {
    consent: Consent,
    fetch: Arc<dyn Fetch>,
}

impl MailNet {
    pub(crate) fn new(consent: Consent, fetch: Arc<dyn Fetch>) -> Self {
        MailNet { consent, fetch }
    }

    /// The frame that asked, as the consent names frames. Nothing else asks.
    fn frame(request: &NetRequest) -> Option<u64> {
        match request.origin() {
            RequestOrigin::Top => None,
            RequestOrigin::Frame(frame) => Some(key(frame)),
        }
    }
}

/// A frame's document as a number. `FrameId` is opaque; its hash is the one thing it gives out
/// that distinguishes two frames.
fn key(frame: FrameId) -> u64 {
    let mut hasher = DefaultHasher::new();
    frame.hash(&mut hasher);
    hasher.finish()
}

impl AppNet for MailNet {
    fn decide(&self, request: &NetRequest) -> NetDecision {
        match Self::frame(request) {
            Some(frame) if self.consent.admits(frame, request.url()).is_some() => {
                NetDecision::Allow
            }
            _ => NetDecision::Deny,
        }
    }

    fn fetch(&self, request: NetRequest, reply: NetReply) {
        // Asked again rather than trusted from `decide`: the same answer for the same request,
        // and the ticket that says whether the bytes may still be shown when they land.
        let Some(frame) = Self::frame(&request) else {
            return;
        };
        let Some(ticket) = self.consent.admits(frame, request.url()) else {
            return;
        };
        let consent = self.consent.clone();
        self.fetch.get(
            request.url().to_owned(),
            Box::new(move |bytes| {
                if consent.stands(ticket) {
                    reply.bytes(bytes);
                }
            }),
        );
    }
}

/// The largest image the frame is handed. A picture in a mail is kilobytes; past this it is
/// not a picture.
pub(crate) const MAX_BYTES: usize = 16 * 1024 * 1024;

/// One admitted fetch, for the worker.
struct Job {
    url: String,
    done: Box<dyn FnOnce(Got) + Send>,
}

/// The web: one worker thread, started by the first admitted image, fetching each on its own
/// task with one client.
///
/// The client sends no cookies (reqwest's `cookies` feature is off) and no `Referer`; it
/// follows at most three redirects, which reqwest keeps to `http` and `https`; it gives up after
/// twenty seconds. A failed fetch leaves the image missing and says nothing.
#[derive(Default)]
pub(crate) struct Web {
    jobs: OnceLock<Option<tokio::sync::mpsc::UnboundedSender<Job>>>,
}

impl Web {
    fn worker() -> Option<tokio::sync::mpsc::UnboundedSender<Job>> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .connect_timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::limited(3))
            .referer(false)
            .build()
            .ok()?;
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel::<Job>();
        std::thread::Builder::new()
            .name("mailo-images".to_owned())
            .spawn(move || {
                let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                else {
                    return;
                };
                runtime.block_on(async move {
                    while let Some(job) = receiver.recv().await {
                        tokio::spawn(fetch_one(client.clone(), job));
                    }
                });
            })
            .ok()?;
        Some(sender)
    }
}

impl Fetch for Web {
    fn get(&self, url: String, done: Box<dyn FnOnce(Vec<u8>) + Send>) {
        FetchImage::get(self, url, Box::new(move |got| done(got.bytes)));
    }
}

impl FetchImage for Web {
    fn get(&self, url: String, done: Box<dyn FnOnce(Got) + Send>) {
        if let Some(jobs) = self.jobs.get_or_init(Web::worker) {
            let _ = jobs.send(Job { url, done });
        }
    }
}

async fn fetch_one(client: reqwest::Client, job: Job) {
    let Ok(mut response) = client.get(&job.url).send().await else {
        return;
    };
    if !response.status().is_success() {
        return;
    }
    if response
        .content_length()
        .is_some_and(|len| len > MAX_BYTES as u64)
    {
        return;
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
            Ok(Some(_)) | Err(_) => return,
            Ok(None) => break,
        }
    }
    (job.done)(Got {
        content_type,
        bytes,
    });
}
