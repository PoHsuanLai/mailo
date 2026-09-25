//! The reader's Original view: the sender's sanitized markup in a document of its own.
//!
//! On either renderer the Original is an `<iframe srcdoc>` (`reading/blocks.rs`'s `Sandbox`),
//! never inline HTML. On the webview it is a sandboxed frame with an opaque origin. On Blitz
//! (`native`) the same element builds a separate, sealed Blitz document: no shared DOM, no shared
//! cascade, no script engine anywhere in the program. What it may reach is mailo's to say:
//! - [`Consent`]: the reader's consent to remote images, as the network reads it (both builds);
//! - `net.rs`: the frame's network on Blitz, refusing everything the consent does not admit;
//! - `links.rs`: a link clicked in the frame, opened in the browser and never in the frame, and
//!   the link under the pointer in a frame, for the reader's link pill ([`FramePill`]);
//! - [`ReaderNet`]: on Blitz, how mailo itself fetches the Reader view's consented images, which
//!   the window's own document may not (`reading/remote.rs`).
//!
//! FINDINGS F157 has the design and why.

mod consent;
#[cfg(feature = "native")]
mod links;
#[cfg(feature = "native")]
mod net;

pub use consent::Consent;
#[cfg(feature = "native")]
pub use links::{Browse, FramePill};
#[cfg(feature = "native")]
pub(crate) use net::data_uri;
#[cfg(feature = "native")]
pub use net::{Fetch, FetchImage, Got};

/// The Reader view's image fetcher, as the window's root context. Cloning shares it.
#[cfg(feature = "native")]
#[derive(Clone)]
pub struct ReaderNet(pub(crate) std::sync::Arc<dyn FetchImage>);

#[cfg(feature = "native")]
impl std::fmt::Debug for ReaderNet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ReaderNet")
    }
}

/// What the window's Original frames are allowed: the consent they are held to, their network
/// and what their links do. Built once per window, and the same for a test's harness.
#[cfg(feature = "native")]
#[derive(Clone)]
pub struct Original {
    consent: Consent,
    net: std::sync::Arc<net::MailNet>,
    links: ds_native::FrameLinks,
    pill: FramePill,
    images: ReaderNet,
}

#[cfg(feature = "native")]
impl std::fmt::Debug for Original {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Original")
            .field("consent", &self.consent)
            .field("links", &self.links)
            .finish()
    }
}

#[cfg(feature = "native")]
impl Original {
    /// Frames whose admitted images `fetch` fetches and whose links `browse` opens. The Reader
    /// view fetches nothing until [`Original::with_images`] says what fetches for it.
    pub fn new(fetch: std::sync::Arc<dyn Fetch>, browse: std::sync::Arc<dyn Browse>) -> Self {
        let consent = Consent::new();
        let (pill, hovered) = links::FramePill::new();
        Original {
            net: std::sync::Arc::new(net::MailNet::new(consent.clone(), fetch)),
            links: links::frame_links(browse, hovered),
            pill,
            images: ReaderNet(std::sync::Arc::new(net::Refuse)),
            consent,
        }
    }

    /// These frames, with `images` fetching the Reader view's consented images.
    pub fn with_images(mut self, images: std::sync::Arc<dyn FetchImage>) -> Self {
        self.images = ReaderNet(images);
        self
    }

    /// The window's: the web, for the frames and the Reader view alike, and the system browser.
    pub fn window() -> Self {
        let web = std::sync::Arc::new(net::Web::default());
        Original::new(web.clone(), std::sync::Arc::new(links::System)).with_images(web)
    }

    /// The Reader view's image fetcher, as the window's root context.
    pub fn images(&self) -> ReaderNet {
        self.images.clone()
    }

    /// The consent, as the window's root context: the reader writes it.
    pub fn consent(&self) -> Consent {
        self.consent.clone()
    }

    /// The network policy: every request beyond `data:` and the app's own `file:` is mailo's.
    pub fn net(&self) -> ds_native::NetPolicy {
        ds_native::NetPolicy::Custom(self.net.clone())
    }

    /// What a link clicked in a frame does, and where the pointer crossing one is reported.
    pub fn links(&self) -> ds_native::FrameLinks {
        self.links.clone()
    }

    /// The link under the pointer in a frame, as the window's root context: the reader's link
    /// pill reads it.
    pub fn pill(&self) -> FramePill {
        self.pill.clone()
    }

    /// `config` with all of these: how a test's harness gets what the window gets.
    pub fn harness(&self, config: ds_native::HarnessConfig) -> ds_native::HarnessConfig {
        config
            .with_net(self.net())
            .with_frame_links(self.links())
            .with_context(self.consent())
            .with_context(self.pill())
            .with_context(self.images())
    }
}
