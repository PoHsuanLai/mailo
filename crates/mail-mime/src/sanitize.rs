//! Untrusted HTML to something safe to put in a WebView.

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

/// HTML that has been through [`sanitize`]. The only kind `mail-app` will render.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SafeHtml(String);

impl SafeHtml {
    /// The sanitized markup.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Wrap already-sanitized markup. Only [`sanitize`] should call this.
    // Unused only because `sanitize` is still `todo!()`.
    #[allow(dead_code)]
    pub(crate) fn new(html: String) -> Self {
        Self(html)
    }
}

/// Sanitize a message body for display.
///
/// Total: there is no error case. Anything that cannot be made safe is removed, because the
/// alternative — showing the user nothing — is worse than showing them the text.
pub fn sanitize(_html: &str, _policy: SanitizePolicy) -> SafeHtml {
    todo!("wave 2")
}
