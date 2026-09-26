//! URLs a block document is allowed to hold.
//!
//! Three schemes: `http`, `https`, `mailto`. Those are the ones a reader can
//! act on without executing script, fetching a document-as-data, or reading
//! the filesystem. `javascript:`, `data:`, and `vbscript:` are rejected here
//! even though the sanitizer should already have removed them — the sanitizer
//! is another crate, and this one does not trust its output with an `expect`.
//!
//! The renderer opens every link with `target="_blank"` and
//! `rel="noopener noreferrer"`. Those attributes are set on the sanitized
//! markup today and do not survive into a block, so they are constants the
//! renderer applies rather than fields of the link.

use super::tracking;
use ammonia::Url;

/// `rel` the renderer sets on every link. The sanitizer sets the same value.
pub const LINK_REL: &str = "noopener noreferrer";

/// `target` the renderer sets on every link. The sanitizer sets the same value.
pub const LINK_TARGET: &str = "_blank";

/// An `http`, `https`, or `mailto` URL.
///
/// Constructible only through [`SafeUrl::parse`]. The stored string is the
/// parser's own canonical form, so parsing it again yields the same value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SafeUrl(String);

impl SafeUrl {
    /// Parse `raw` into a URL this crate will keep.
    ///
    /// `None` when the scheme is not one of the three, the host is missing for
    /// `http`/`https`, or the string carries whitespace, a control character,
    /// or a bidi override (those reorder the host a person thinks they see).
    pub fn parse(raw: &str) -> Option<Self> {
        if raw.is_empty() || raw.chars().any(forbidden_in_url) {
            return None;
        }
        let url = Url::parse(raw).ok()?;
        match url.scheme() {
            "http" | "https" => {
                url.host_str()?;
            }
            "mailto" => {
                if url.path().is_empty() {
                    return None;
                }
            }
            _ => return None,
        }
        let canonical = url.as_str();
        if canonical.chars().any(forbidden_in_url) {
            return None;
        }
        Some(Self(canonical.to_owned()))
    }

    /// Parse `raw` as a link the reader shows and opens: [`SafeUrl::parse`], then without the
    /// query parameters that only tell the sender who clicked (`utm_*`, `fbclid`, `gclid`,
    /// `mc_eid` and the rest of [`super::tracking::TRACKING`]).
    ///
    /// Only named parameters come off; an unknown one stays. A redirect wrapper is not unwrapped,
    /// so the link goes where the sender pointed it. An image's address is not a link and goes
    /// through [`SafeUrl::parse`] alone, as does a link the user types into a draft.
    pub fn link(raw: &str) -> Option<Self> {
        let parsed = Self::parse(raw)?;
        let mut url = Url::parse(parsed.as_str()).ok()?;
        tracking::strip(&mut url);
        Some(Self(url.as_str().to_owned()))
    }

    /// The canonical URL.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// `http`, `https`, or `mailto`.
    pub fn scheme(&self) -> &str {
        match self.0.split_once(':') {
            Some((scheme, _)) => scheme,
            None => "",
        }
    }
}

/// Hostname of an `http`/`https` URL, lowercased, with no userinfo and no path.
///
/// Used when a remote image is blocked: the document records who would learn
/// that the mail was opened, and nothing that could be fetched.
pub(crate) fn image_host(raw: &str) -> Option<String> {
    if raw.chars().any(forbidden_in_url) {
        return None;
    }
    let url = Url::parse(raw).ok()?;
    match url.scheme() {
        "http" | "https" => url.host_str().map(|host| host.to_ascii_lowercase()),
        _ => None,
    }
}

fn forbidden_in_url(ch: char) -> bool {
    ch.is_whitespace()
        || ch.is_control()
        || matches!(ch, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}
