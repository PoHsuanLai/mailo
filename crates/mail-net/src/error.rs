//! What goes wrong on a connection, and the words that carry it.

use mail_domain::{Retry, Retryable};
use std::fmt;
use std::time::Duration;

/// What a failure carries as its cause, kept as the error it was rather than as its text.
pub type Source = Box<dyn std::error::Error + Send + Sync + 'static>;

/// What went wrong outside the protocol, in words, with the error underneath when there is one.
///
/// Reads as `doing: cause` (just `doing` or just the cause when the other is absent), the way
/// the text always read; a caller that wants the cause itself walks [`std::error::Error::source`]
/// rather than parsing the sentence.
#[derive(Debug)]
pub struct Failure {
    doing: String,
    source: Option<Source>,
}

impl Failure {
    /// What was being done, and the error that stopped it.
    pub fn new(doing: impl Into<String>, source: impl Into<Source>) -> Self {
        Self {
            doing: doing.into(),
            source: Some(source.into()),
        }
    }

    /// A failure with no cause beneath it, only words.
    pub fn said(words: impl Into<String>) -> Self {
        Self {
            doing: words.into(),
            source: None,
        }
    }

    /// An error with nothing to say about what was being done.
    pub fn caused(source: impl Into<Source>) -> Self {
        Self {
            doing: String::new(),
            source: Some(source.into()),
        }
    }

    /// The same failure with something more said after it, as an issuer says why it refused.
    pub fn add_detail(&mut self, said: &str) {
        *self = Failure::said(format!("{self}: {said}"));
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.doing.is_empty(), &self.source) {
            (_, None) => f.write_str(&self.doing),
            (true, Some(source)) => write!(f, "{source}"),
            (false, Some(source)) => write!(f, "{}: {source}", self.doing),
        }
    }
}

impl std::error::Error for Failure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_deref()
            .map(|source| source as &(dyn std::error::Error + 'static))
    }
}

impl From<String> for Failure {
    fn from(words: String) -> Self {
        Failure::said(words)
    }
}

impl From<&str> for Failure {
    fn from(words: &str) -> Self {
        Failure::said(words)
    }
}

/// Something went wrong on the wire: a connection, a handshake, a lookup, a request.
#[derive(Debug, thiserror::Error)]
pub enum NetError {
    #[error("cannot connect: {0}")]
    Connect(#[source] Failure),
    #[error("TLS: {0}")]
    Tls(#[source] Failure),
    #[error("io: {0}")]
    Io(#[source] Failure),
    /// The other end went quiet for longer than `doing` may take: no byte, no acknowledgement,
    /// no close. A NAT or a provider that drops a connection without a word leaves exactly
    /// this, a socket that looks open and never speaks again, so waiting on is only a hang.
    #[error("no answer from the server for {}s while {doing}", .after.as_secs())]
    Stalled {
        doing: &'static str,
        after: Duration,
    },
    /// What was asked cannot be done over this transport, or is not a thing to ask.
    #[error("unsupported io: {0}")]
    Unsupported(String),
}

impl Retryable for NetError {
    fn retry(&self) -> Retry {
        match self {
            // A dropped connection or a name that did not resolve this second. Reconnecting is
            // the whole remedy, and the cost is one round trip.
            // A connection that went silent is one that is gone, whatever the socket says: the
            // same remedy, a new one.
            NetError::Connect(_) | NetError::Io(_) | NetError::Stalled { .. } => {
                Retry::After(Duration::from_secs(5))
            }
            // Either the clock is wrong, the network is hostile, or the server really did
            // present a bad chain. None of those is fixed by trying again in a loop, and
            // retrying a rejected certificate is how a client teaches its user to ignore it.
            NetError::Tls(why) => Retry::Fatal(why.to_string()),
            NetError::Unsupported(why) => Retry::Fatal(why.clone()),
        }
    }
}

/// `work`, or [`NetError::Stalled`] once `limit` has passed without it finishing.
///
/// The one way a network await in this workspace is bounded, so that running out of time is always
/// the same typed error, which [`Retryable::retry`] answers with a reconnect. Generic over the
/// caller's error so the same bound serves a crate that wraps [`NetError`] in its own.
pub async fn within<T, E: From<NetError>>(
    limit: Duration,
    doing: &'static str,
    work: impl std::future::Future<Output = Result<T, E>>,
) -> Result<T, E> {
    tokio::time::timeout(limit, work).await.unwrap_or_else(|_| {
        Err(NetError::Stalled {
            doing,
            after: limit,
        }
        .into())
    })
}
