//! Runtime failures.

use mail_domain::{Retry, Retryable};
use std::time::Duration;

/// Something went wrong outside the protocol machines.
#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error("cannot connect: {0}")]
    Connect(String),
    #[error("TLS: {0}")]
    Tls(String),
    #[error("io: {0}")]
    Io(String),
    #[error("protocol: {0}")]
    Proto(#[from] mail_proto::ProtoError),
    #[error("store: {0}")]
    Store(#[from] mail_store::StoreError),
    #[error("composing: {0}")]
    Compose(#[from] mail_mime::MimeError),
    #[error("secret store: {0}")]
    Secrets(String),
    /// The machine wanted something the transport in use cannot provide.
    #[error("unsupported io: {0}")]
    UnsupportedIo(String),
    /// The account keeps its mail on this computer ([`mail_domain::Incoming::Local`]): there is
    /// no server to do this with. The field says what was attempted: "send from", "connect to".
    #[error("this account keeps its mail on this computer; there is no server to {0}")]
    NoServer(&'static str),
    /// The engine was asked to stop.
    #[error("cancelled")]
    Cancelled,
    /// Microsoft Graph answered, and not with acceptance. Classified where the status is read.
    #[error("{why}")]
    Graph { why: String, retry: Retry },
    /// A one-click unsubscribe did not go through. See [`crate::unsubscribe`].
    #[error("{0}")]
    Unsubscribe(crate::unsubscribe::UnsubscribeFailure),
    /// A CardDAV exchange did not work. See [`crate::carddav`].
    #[error("{0}")]
    CardDav(crate::carddav::CardDavFailure),
}

impl Retryable for RuntimeError {
    fn retry(&self) -> Retry {
        match self {
            // A dropped connection or a name that did not resolve this second. Reconnecting is
            // the whole remedy, and the cost is one round trip.
            RuntimeError::Connect(_) | RuntimeError::Io(_) => Retry::After(Duration::from_secs(5)),
            // Either the clock is wrong, the network is hostile, or the server really did
            // present a bad chain. None of those is fixed by trying again in a loop, and
            // retrying a rejected certificate is how a client teaches its user to ignore it.
            RuntimeError::Tls(why) => Retry::Fatal(why.clone()),
            // A fact about the message, not about the network. Retrying rebuilds the same
            // bytes and fails the same way, forever.
            RuntimeError::Compose(e) => e.retry(),
            // Delegate: the machines already classify their own failures, including the
            // transient/permanent split and rate limiting.
            RuntimeError::Proto(e) => e.retry(),
            RuntimeError::Store(e) => e.retry(),
            RuntimeError::Secrets(_) => Retry::NeedsReauth,
            RuntimeError::UnsupportedIo(why) => Retry::Fatal(why.clone()),
            // Nothing will change: the account has no server and never will.
            RuntimeError::NoServer(_) => Retry::Fatal(self.to_string()),
            // Not a failure: the user closed the app or switched accounts.
            RuntimeError::Cancelled => Retry::Fatal("cancelled".to_owned()),
            RuntimeError::Graph { retry, .. } => retry.clone(),
            RuntimeError::Unsubscribe(failure) => failure.retry(),
            RuntimeError::CardDav(failure) => failure.retry(),
        }
    }
}

/// A JMAP method's refusal is a protocol failure like any other, classified where its type is
/// read (`mail_proto::jmap::MethodError`).
impl From<mail_proto::jmap::MethodError> for RuntimeError {
    fn from(error: mail_proto::jmap::MethodError) -> Self {
        RuntimeError::Proto(error.into())
    }
}
