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
const MAX_BYTES: usize = 16 * 1024 * 1024;

/// One admitted fetch, for the worker.
struct Job {
    url: String,
    done: Box<dyn FnOnce(Vec<u8>) + Send>,
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
    (job.done)(bytes);
}
