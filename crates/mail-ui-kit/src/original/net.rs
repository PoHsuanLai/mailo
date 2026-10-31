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
//! fetching and that rule are `mail_client::remote_image`'s; this is the window's use of them.

use super::consent::{Consent, Holder};
use ds_blitz::{AppNet, NetDecision, NetReply, NetRequest};
use mail_client::remote_image::{ImageFetcher, Route};
use mail_domain::MessageId;
use std::sync::{Arc, Mutex, PoisonError};

/// Whatever fetches an admitted image: the web in the window, a recorder in a test.
pub trait Fetch: Send + Sync + 'static {
    /// Fetch `url` and call `done` with its bytes, from any thread. Not calling it (a failure)
    /// leaves the image missing, as a browser shows a broken one.
    fn get(&self, url: String, done: Box<dyn FnOnce(Vec<u8>) + Send>);
}

/// A fetched image: what the server said it is, and its bytes.
pub use mail_client::remote_image::Image as Got;

/// Whatever fetches the Reader view's consented images: the web in the window, a recorder in a
/// test. mailo asks; no document does.
pub trait FetchImage: Send + Sync + 'static {
    /// Fetch `url` and call `done` with what came back, from any thread. Not calling it (a
    /// failure) leaves the image unshown.
    fn get(&self, url: String, done: Box<dyn FnOnce(Got) + Send>);
}

/// Fetches nothing: the Reader view's images for a harness that did not ask for a fetcher.
pub struct Refuse;

impl FetchImage for Refuse {
    fn get(&self, _: String, _: Box<dyn FnOnce(Got) + Send>) {}
}

/// `got` as a `data:` URI for an `<img>`, or `None` when it is not one to show
/// ([`Got::data_uri`]).
pub fn data_uri(got: &Got) -> Option<String> {
    got.data_uri()
}

/// mailo's `AppNet`.
pub(crate) struct MailNet {
    consent: Consent,
    fetch: Arc<dyn Fetch>,
}

impl MailNet {
    pub fn new(consent: Consent, fetch: Arc<dyn Fetch>) -> Self {
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

/// The way the window's images are fetched, as `privacy.*` says ([`follow`]): `None` until the
/// settings have been read, and while they cannot be (a damaged file), or the person has chosen a
/// relay or a proxy that cannot be used, when nothing is fetched at all rather than fetched
/// straight from the image's host.
static ROUTE: Mutex<Option<Route>> = Mutex::new(None);

/// The way `privacy` says images are fetched, or `None` when they are not to be: `privacy` is
/// `None` when the settings cannot be trusted (`settings::privacy_in_doubt`), and a relay or a
/// proxy that cannot be used is not taken as fetching straight.
fn route_of(
    privacy: Option<&mail_ui_model::settings::PrivacySettings>,
    relay_key: Option<&str>,
) -> Option<Route> {
    privacy?.route(relay_key).ok()
}

/// Fetch the window's images the way `privacy` says from now on; `None` for settings that cannot
/// be trusted fetches none.
pub fn follow(privacy: Option<&mail_ui_model::settings::PrivacySettings>, relay_key: Option<&str>) {
    *ROUTE.lock().unwrap_or_else(PoisonError::into_inner) = route_of(privacy, relay_key);
}

/// The web: one fetcher for the way images are fetched now, made by the first admitted image
/// and again when the way changes, on the runtime servers are reached on. See [`ImageFetcher`]
/// for what its client sends and how it gives up; a failed fetch leaves the image missing and
/// says nothing.
pub struct Web {
    runtime: tokio::runtime::Handle,
    fetcher: Mutex<Option<(Route, ImageFetcher)>>,
}

impl Web {
    pub fn on(runtime: tokio::runtime::Handle) -> Web {
        Web {
            runtime,
            fetcher: Mutex::new(None),
        }
    }

    fn fetcher(&self) -> Option<ImageFetcher> {
        let route = ROUTE.lock().unwrap_or_else(PoisonError::into_inner).clone();
        self.fetcher_for(route)
    }

    /// The fetcher for `route`; none for no route, which fetches nothing.
    fn fetcher_for(&self, route: Option<Route>) -> Option<ImageFetcher> {
        let route = route?;
        let mut held = self.fetcher.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((made_for, fetcher)) = held.as_ref()
            && *made_for == route
        {
            return Some(fetcher.clone());
        }
        let fetcher = ImageFetcher::new(self.runtime.clone(), route.clone()).ok()?;
        *held = Some((route, fetcher.clone()));
        Some(fetcher)
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

#[cfg(test)]
mod tests {
    use super::*;
    use mail_ui_model::settings::{PrivacySettings, Protect};

    #[test]
    fn settings_that_are_not_known_or_cannot_be_used_fetch_nothing() {
        let off = PrivacySettings::default();
        assert_eq!(route_of(Some(&off), None), Some(Route::Direct));
        // Damaged or unread: unknown, never "Off".
        assert_eq!(route_of(None, None), None);
        // A relay chosen with no address or key, and a proxy with no address.
        for protect in [Protect::Relay, Protect::Proxy] {
            let chosen = PrivacySettings {
                protect,
                ..PrivacySettings::default()
            };
            assert_eq!(route_of(Some(&chosen), None), None, "{protect:?}");
        }
    }

    #[tokio::test]
    async fn no_route_makes_no_fetcher() {
        let web = Web::on(tokio::runtime::Handle::current());
        assert!(web.fetcher_for(None).is_none());
        assert!(web.fetcher_for(Some(Route::Direct)).is_some());
    }
}
