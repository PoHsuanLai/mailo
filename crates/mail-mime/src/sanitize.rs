//! Untrusted HTML to something safe to put in a WebView.

use std::borrow::Cow;
use std::collections::HashSet;

/// Whether to let the message reach the network when it renders.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteImages {
    /// Default. A remote image is a read receipt the sender did not ask permission for.
    Blocked,
    Allowed,
}

/// How aggressively to sanitize, and which revision of that policy this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SanitizePolicy {
    pub remote_images: RemoteImages,
    /// Bumped whenever the policy or the `ammonia` major changes.
    ///
    /// Part of the render cache key. Sanitizer output is never persisted — an upgrade would
    /// otherwise leave every previously-ingested message sanitized under the old rules.
    pub version: u32,
}

impl SanitizePolicy {
    /// The current default: remote images blocked.
    pub const CURRENT: SanitizePolicy = SanitizePolicy {
        remote_images: RemoteImages::Blocked,
        version: 1,
    };
}

/// HTML that has been through [`sanitize`], and what had to be removed to get there.
///
/// The count is not bookkeeping. "Load remote images" is an offer to make network requests on
/// the sender's behalf, and a reader that shows it on every message — including the ones with
/// no images at all, and the plain-text ones — has trained its user to ignore the one place it
/// matters. Only [`sanitize`] knows whether anything was actually dropped, so only it can say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SafeHtml {
    html: String,
    blocked_remote: u32,
}

impl SafeHtml {
    /// The sanitized markup.
    pub fn as_str(&self) -> &str {
        &self.html
    }

    /// How many remote fetches were removed.
    ///
    /// Zero under [`RemoteImages::Allowed`] by construction: nothing is blocked when nothing
    /// is being blocked.
    pub fn blocked_remote(&self) -> u32 {
        self.blocked_remote
    }

    /// Wrap already-sanitized markup. Only [`sanitize`] should call this.
    pub(crate) fn new(html: String, blocked_remote: u32) -> Self {
        Self {
            html,
            blocked_remote,
        }
    }
}

/// Sanitize a message body for display.
///
/// Total: there is no error case. Anything that cannot be made safe is removed, because the
/// alternative — showing the user nothing — is worse than showing them the text.
pub fn sanitize(html: &str, policy: SanitizePolicy) -> SafeHtml {
    let remote = policy.remote_images;
    // Ammonia's defaults are an allowlist: script, style, iframe, object, embed, form,
    // input, button, base, meta, link, svg and every `on*` attribute are absent from it,
    // so they never reach the serializer. `style` stays off the list on purpose — ammonia's
    // CSS filter keeps `url()`, and a property allowlist would let a tracker through.
    // `url_schemes` is global, so it cannot express "cid always, http(s) only on images and
    // only when the reader opted in". The attribute filter applies that second rule to
    // attributes ammonia has already scheme-checked.
    // Shared with the attribute filter, which ammonia calls while cleaning. An `AtomicU32`
    // rather than a `Cell` because the filter must be `Send + Sync`.
    let blocked = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    let counter = blocked.clone();
    let cleaned = ammonia::Builder::new()
        .url_schemes(HashSet::from(["http", "https", "mailto", "cid"]))
        .url_relative(ammonia::UrlRelative::Deny)
        .link_rel(Some("noopener noreferrer"))
        .set_tag_attribute_value("a", "target", "_blank")
        .strip_comments(true)
        .rm_tag_attributes("blockquote", &["cite"])
        .rm_tag_attributes("del", &["cite"])
        .rm_tag_attributes("ins", &["cite"])
        .rm_tag_attributes("q", &["cite"])
        .attribute_filter(move |element, attribute, value| {
            if fetches_on_render(element, attribute) {
                let kept = keep_fetched_url(value, remote);
                if kept.is_none() && is_remote(value) {
                    counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
                kept
            } else {
                Some(Cow::Borrowed(value))
            }
        })
        .clean(html)
        .to_string();
    SafeHtml::new(cleaned, blocked.load(std::sync::atomic::Ordering::Relaxed))
}

/// Whether a dropped URL was one the reader could choose to load.
///
/// Only `http`/`https` are: a `javascript:` or `data:` src is not something to offer, and
/// counting it would put the button in front of a user whose answer can only make things worse.
fn is_remote(value: &str) -> bool {
    ammonia::Url::parse(value)
        .map(|url| matches!(url.scheme(), "http" | "https"))
        .unwrap_or(false)
}

/// Attributes that cause a fetch when the document is shown.
///
/// `href` on an anchor is a click, not a fetch. `srcset`, `poster` and `background` are not
/// on the allowlist; matching them here drops them if a later edit allows the attribute
/// without also scheme-checking it. Ammonia does not treat `srcset` as a URL.
fn fetches_on_render(element: &str, attribute: &str) -> bool {
    match attribute {
        "src" | "srcset" | "poster" | "background" => true,
        "href" | "xlink:href" => matches!(element, "image" | "use" | "feimage"),
        _ => false,
    }
}

/// Keep a URL ammonia has already accepted, or drop it.
///
/// `cid:` is this message's own part and must survive in both modes. `http` and `https`
/// are a network fetch, kept only when the reader allowed remote images. Anything that is
/// not a single parseable URL — a `srcset` candidate list, in particular — is dropped
/// rather than split with a hand-rolled scanner.
fn keep_fetched_url(value: &str, remote: RemoteImages) -> Option<Cow<'_, str>> {
    let url = ammonia::Url::parse(value).ok()?;
    let keep = match url.scheme() {
        "cid" => true,
        "http" | "https" => remote == RemoteImages::Allowed,
        _ => false,
    };
    keep.then_some(Cow::Borrowed(value))
}
