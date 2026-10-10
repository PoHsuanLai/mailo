//! The Original frame's network on Blitz: quire's `NetPolicy::Custom`, answered by mailo.
//!
//! ds-blitz puts every request a frame makes (anything but inline `data:`) to [`MailNet`], and
//! every request the window's own document makes beyond its `file:` and `data:`. The answer:
//! - **The window's own document: refused.** mailo's markup names no remote resource, and a
//!   request from the app document cannot be told apart by who drew it: a hover card and the
//!   reader are the same document. A remote image is never fetched because something was hovered.
//! - **A frame: refused unless the reader's [`Consent`] admits it**: `http`/`https`, on the
//!   allowlist of the consented message the frame shows, which its `data-frame-tag` names
//!   (`NetRequest::frame_tag`; `consent.rs`). An untagged frame is refused. `file:`, `cid:` and
//!   every other scheme are refused whatever a list says.
//! - **Admitted: fetched by [`Fetch`]**, and handed to the frame only if the consent still stands
//!   when the bytes land.
//!
//! The Reader view's remote images are in the window's own document, so the refusal above holds
//! them too. mailo fetches those itself instead (`reading/remote.rs`), through [`FetchImage`] on
//! the same client, and hands the window each as a `data:` URI ([`data_uri`]) only when it is a
//! raster image of a kind `mail-mime` embeds, by its declared type and by its first bytes. The
//! fetching and that rule are `mail_core::remote_image`'s; this is the window's use of them.

use super::consent::{Consent, Holder};
use ds_blitz::{AppNet, NetDecision, NetReply, NetRequest};
use mail_core::remote_image::ImageFetcher;
use mail_domain::MessageId;
use std::sync::{Arc, OnceLock};

/// Whatever fetches an admitted image: the web in the window, a recorder in a test.
pub trait Fetch: Send + Sync + 'static {
    /// Fetch `url` and call `done` with its bytes, from any thread. Not calling it (a failure)
    /// leaves the image missing, as a browser shows a broken one.
    fn get(&self, url: String, done: Box<dyn FnOnce(Vec<u8>) + Send>);
}

/// A fetched image: what the server said it is, and its bytes.
pub use mail_core::remote_image::Image as Got;

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

/// `got` as a `data:` URI for an `<img>`, or `None` when it is not one to show
/// ([`Got::data_uri`]).
pub(crate) fn data_uri(got: &Got) -> Option<String> {
    got.data_uri()
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

    /// The reader that drew the frame that asked, and the message it shows, by its
    /// `data-frame-tag`. The window's own document and an untagged frame show none.
    fn frame(request: &NetRequest) -> Option<(Holder, MessageId)> {
        Holder::of_tag(request.frame_tag()?.as_str())
    }
}

impl AppNet for MailNet {
    fn decide(&self, request: &NetRequest) -> NetDecision {
        match Self::frame(request) {
            Some((holder, message))
                if self
                    .consent
                    .admits(holder, message, request.url())
                    .is_some() =>
            {
                NetDecision::Allow
            }
            _ => NetDecision::Deny,
        }
    }

    fn fetch(&self, request: NetRequest, reply: NetReply) {
        // Asked again rather than trusted from `decide`: the same answer for the same request,
        // and the ticket that says whether the bytes may still be shown when they land.
        let Some((holder, message)) = Self::frame(&request) else {
            return;
        };
        let Some(ticket) = self.consent.admits(holder, message, request.url()) else {
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

/// The web: one fetcher, made by the first admitted image, on the application's runtime. See
/// [`ImageFetcher`] for what its client sends and how it gives up; a failed fetch leaves the
/// image missing and says nothing.
#[derive(Default)]
pub(crate) struct Web {
    fetcher: OnceLock<Option<ImageFetcher>>,
}

impl Web {
    fn fetcher(&self) -> Option<&ImageFetcher> {
        self.fetcher
            .get_or_init(|| ImageFetcher::new(crate::edge::runtime().handle().clone()).ok())
            .as_ref()
    }
}

impl Fetch for Web {
    fn get(&self, url: String, done: Box<dyn FnOnce(Vec<u8>) + Send>) {
        FetchImage::get(self, url, Box::new(move |got| done(got.bytes)));
    }
}

impl FetchImage for Web {
    fn get(&self, url: String, done: Box<dyn FnOnce(Got) + Send>) {
        if let Some(fetcher) = self.fetcher() {
            fetcher.fetch(url, done);
        }
    }
}
