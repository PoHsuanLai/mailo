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
    #[error("secret store: {0}")]
    Secrets(String),
    /// The machine wanted something the transport in use cannot provide.
    #[error("unsupported io: {0}")]
    UnsupportedIo(String),
    /// The engine was asked to stop.
    #[error("cancelled")]
    Cancelled,
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
            // Delegate: the machines already classify their own failures, including the
            // transient/permanent split and rate limiting.
            RuntimeError::Proto(e) => e.retry(),
            RuntimeError::Store(e) => e.retry(),
            RuntimeError::Secrets(_) => Retry::NeedsReauth,
            RuntimeError::UnsupportedIo(why) => Retry::Fatal(why.clone()),
            // Not a failure: the user closed the app or switched accounts.
            RuntimeError::Cancelled => Retry::Fatal("cancelled".to_owned()),
        }
    }
}
