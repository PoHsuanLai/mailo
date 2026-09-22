//! MIME parsing, building, and HTML sanitization. Pure: bytes in, domain values out.
//!
//! Its own crate because these are not protocol machines — they have no [`IoNeed`] and no
//! state — and because `mail-app` must be able to sanitize without depending on `mail-proto`.
//!
//! [`IoNeed`]: https://docs.rs/mail-proto

pub mod build;
pub mod inline;
pub mod parse;
pub mod reconstruct;
pub mod sanitize;

pub use build::{Disclosure, Posting, build, posting};
pub use inline::{INLINE_BUDGET, embed_inline};
pub use parse::{Parsed, ParsedPart, RemotePart, parse, parse_reconstructed};
pub use reconstruct::{decode_part, reconstruct, sections_for};
pub use sanitize::{RemoteImages, SafeHtml, SanitizePolicy, sanitize};

/// Something in the message, or in what we were asked to build, did not hold together.
///
/// Never a panic: every byte reaching this crate came off a network a stranger controls.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MimeError {
    #[error("message could not be parsed: {0}")]
    Unparseable(String),
    #[error("required header missing: {0}")]
    MissingHeader(&'static str),
    #[error("attachment {0} was referenced but not supplied")]
    MissingPart(String),
    #[error("cannot build a message with no recipients")]
    NoRecipients,
}

impl mail_domain::Retryable for MimeError {
    fn retry(&self) -> mail_domain::Retry {
        // All four are facts about the bytes, not about the network. Retrying re-runs the
        // same failure on the same input forever.
        mail_domain::Retry::Fatal(self.to_string())
    }
}
