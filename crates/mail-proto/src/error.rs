//! Why a protocol exchange failed, and what to do about it.
//!
//! One enum for the whole crate: IMAP, POP3, SMTP and JMAP all end in the same decision at the
//! outbox, so the split that matters is not which protocol but whether trying again can help.
//! [`Retryable`] is where that is answered. The network half (a dropped connection, a
//! throttle, a 4xx) backs off and retries; the protocol half (a refusal that will not change, a
//! rejected login, a capability the server lacks) is fatal or needs the person.

use mail_domain::{Retry, Retryable};
use std::time::Duration;

/// Whether a refusal may succeed if repeated.
///
/// Load-bearing: [`Retryable`] maps `Transient` to a delay and `Permanent` to giving up and
/// undoing the local change, so getting this wrong either discards the user's mail or retries
/// a hopeless operation forever.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// SMTP 4xx, or an equivalent "try again later". Greylisting lives here.
    Transient,
    /// SMTP 5xx, IMAP `NO` on a malformed request, or anything that will not change on its own.
    Permanent,
}

/// A protocol-level failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProtoError {
    /// The server's response did not parse. Never a panic: every byte here is hostile.
    #[error("malformed response: {0}")]
    Malformed(String),
    /// A well-formed refusal: IMAP `NO`, SMTP 4xx/5xx, POP3 `-ERR`.
    ///
    /// The class is a field rather than something encoded in `text`, because the outbox has to
    /// branch on it and string-sniffing a prefix breaks the moment a message is reworded.
    #[error("server refused ({kind:?}): {text}")]
    Refused { kind: Refusal, text: String },
    #[error("authentication rejected: {0}")]
    AuthRejected(String),
    #[error("connection closed unexpectedly")]
    UnexpectedEof,
    /// The server is rate-limiting or over quota: IMAP `[LIMIT]`, `[OVERQUOTA]`, Gmail's
    /// "too many simultaneous connections", SMTP 421.
    ///
    /// A distinct variant because it must **not** be fatal. Gmail caps daily IMAP transfer and
    /// simultaneous connections and answers excess with a lockout measured in hours; treating
    /// that as a permanent refusal would apply the undo patch and discard work that would have
    /// succeeded tomorrow. `retry_after` is the server's hint when it gives one.
    #[error("server is rate-limiting: {reason}")]
    Throttled {
        reason: String,
        retry_after: Option<Duration>,
    },
    /// The server lacks something the operation needs.
    #[error("server does not support {0}")]
    Unsupported(String),
}

impl Retryable for ProtoError {
    fn retry(&self) -> Retry {
        match self {
            // A dropped connection is the common case and costs one reconnect.
            ProtoError::UnexpectedEof => Retry::Now,
            // Retrying cannot help; the user has to reauthenticate.
            ProtoError::AuthRejected(_) => Retry::NeedsReauth,
            // The message is gone, or the server will refuse this forever. Undo the local
            // change rather than retrying into a loop.
            // 4xx and its equivalents mean "not now". Greylisting is the common case and many
            // servers do it deliberately on first contact, so treating a transient refusal as
            // fatal would apply the undo patch and DISCARD the user's outgoing message.
            ProtoError::Refused {
                kind: Refusal::Transient,
                ..
            } => Retry::After(Duration::from_secs(60)),
            ProtoError::Refused {
                kind: Refusal::Permanent,
                text,
            }
            | ProtoError::Unsupported(text) => Retry::Fatal(text.clone()),
            // Backing off is the entire remedy, and hammering makes the lockout longer.
            // An hour is the floor when the server offers no hint, because Gmail's own
            // lockouts are measured in hours.
            ProtoError::Throttled { retry_after, .. } => {
                Retry::After(retry_after.unwrap_or(Duration::from_secs(3600)))
            }
            // Possibly our parser, possibly a transient truncation. Back off rather than
            // hammering, and keep the operation so a fix ships without data loss.
            ProtoError::Malformed(_) => Retry::After(Duration::from_secs(60)),
        }
    }
}
