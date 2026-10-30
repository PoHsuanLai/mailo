//! Why bytes could not be read, or a message could not be built from what was given.
//!
//! Every type here is a fact about the bytes, never the network, so each answers
//! [`Retryable`] with a fatal verdict: asking again re-runs the same failure on the same input.

use mail_domain::{Retry, Retryable};

/// Something in the message, or in what we were asked to build, did not hold together.
///
/// Never a panic: every byte reaching this crate came off a network a stranger controls.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
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

impl Retryable for MimeError {
    fn retry(&self) -> Retry {
        // All of these are facts about the bytes or the keys, not about the network. Retrying re-runs the
        // same failure on the same input forever.
        Retry::Fatal(self.to_string())
    }
}

/// Why a TXT string is not a BIMI record this client will use.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum RecordError {
    /// It does not open with `v=BIMI1`.
    #[error("not a BIMI record")]
    NotBimi,
    /// A tag appears twice.
    #[error("the tag {0} appears twice")]
    Repeated(String),
    /// `l=` or `a=` is not a single `https:` URL.
    #[error("{0:?} is not a single https URL")]
    NotHttps(String),
}

impl Retryable for RecordError {
    fn retry(&self) -> Retry {
        Retry::Fatal(self.to_string())
    }
}

/// Text that is not a `mailto:` URI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("not a mailto: URI")]
pub struct NotMailto;

/// Text that is not a link this crate will keep: see [`SafeUrl`](crate::SafeUrl).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("not an http, https or mailto link that is safe to show")]
pub struct UnsafeUrl;

/// Text that is not an `https:` URL a one-click unsubscribe may `POST` to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("not an https URL without credentials")]
pub struct NotHttpsUrl;
