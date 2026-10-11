//! Runtime failures.

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
    pub(crate) fn add_detail(&mut self, said: &str) {
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

/// Something went wrong outside the protocol machines.
#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error("cannot connect: {0}")]
    Connect(#[source] Failure),
    #[error("TLS: {0}")]
    Tls(#[source] Failure),
    #[error("io: {0}")]
    Io(#[source] Failure),
    #[error("protocol: {0}")]
    Proto(#[from] mail_proto::ProtoError),
    #[error("store: {0}")]
    Store(#[from] mail_store::StoreError),
    #[error("composing: {0}")]
    Compose(#[from] mail_mime::MimeError),
    #[error("secret store: {0}")]
    Secrets(#[source] Failure),
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
    /// The other end went quiet for longer than `doing` may take: no byte, no acknowledgement,
    /// no close. A NAT or a provider that drops a connection without a word leaves exactly
    /// this, a socket that looks open and never speaks again, so waiting on is only a hang.
    #[error("no answer from the server for {}s while {doing}", .after.as_secs())]
    Stalled {
        doing: &'static str,
        after: Duration,
    },
    /// Microsoft Graph answered, and not with acceptance. Classified where the status is read.
    #[error("{why}")]
    Graph { why: String, retry: Retry },
    /// A one-click unsubscribe did not go through. See [`crate::unsubscribe`].
    #[error("{0}")]
    Unsubscribe(crate::unsubscribe::UnsubscribeFailure),
    /// A CardDAV exchange did not work. See [`crate::carddav`].
    #[error("{0}")]
    CardDav(crate::carddav::CardDavFailure),
    /// An IMAP server's folder list, or Graph's, named no inbox. RFC 3501 gives every account
    /// one, and Graph every mailbox, so the list is not one to keep: stored, it would read as an
    /// account with no folders at all.
    #[error("the server listed no Inbox among its folders")]
    NoInbox,
    /// The server refused a token the issuer had just issued, or one it had refused already
    /// after a renewal. The issuer stands by the sign-in, so signing in again would mint the
    /// same token: the server will not let this account in over this protocol, which is the
    /// mailbox's or the organisation's setting (IMAP or SMTP switched off), not the password —
    /// or the token is another account's than the address it is presented for.
    #[error(
        "the server refused a sign-in the issuer has just renewed, so signing in again would not \
         help: the server may have this protocol switched off for the account, or the sign-in \
         was to a different account than this address ({0})"
    )]
    RefusedFresh(Box<RuntimeError>),
}

impl Retryable for RuntimeError {
    fn retry(&self) -> Retry {
        match self {
            // A dropped connection or a name that did not resolve this second. Reconnecting is
            // the whole remedy, and the cost is one round trip.
            // A connection that went silent is one that is gone, whatever the socket says: the
            // same remedy, a new one.
            RuntimeError::Connect(_) | RuntimeError::Io(_) | RuntimeError::Stalled { .. } => {
                Retry::After(Duration::from_secs(5))
            }
            // Either the clock is wrong, the network is hostile, or the server really did
            // present a bad chain. None of those is fixed by trying again in a loop, and
            // retrying a rejected certificate is how a client teaches its user to ignore it.
            RuntimeError::Tls(why) => Retry::Fatal(why.to_string()),
            // A fact about the message, not about the network. Retrying rebuilds the same
            // bytes and fails the same way, forever.
            RuntimeError::Compose(e) => e.retry(),
            // Delegate: the machines already classify their own failures, including the
            // transient/permanent split and rate limiting.
            RuntimeError::Proto(e) => e.retry(),
            RuntimeError::Store(e) => e.retry(),
            // Missing is a sign-in to ask for; a locked keyring is a wait (`SecretTrouble`).
            RuntimeError::Secrets(_) => crate::SecretTrouble::of(self).retry(),
            RuntimeError::UnsupportedIo(why) => Retry::Fatal(why.clone()),
            // Nothing will change: the account has no server and never will.
            RuntimeError::NoServer(_) => Retry::Fatal(self.to_string()),
            // Not a failure: the user closed the app or switched accounts.
            RuntimeError::Cancelled => Retry::Fatal("cancelled".to_owned()),
            RuntimeError::Graph { retry, .. } => retry.clone(),
            RuntimeError::Unsubscribe(failure) => failure.retry(),
            RuntimeError::CardDav(failure) => failure.retry(),
            // The server's answer, and it will give the same one next time.
            RuntimeError::NoInbox | RuntimeError::RefusedFresh(_) => Retry::Fatal(self.to_string()),
        }
    }
}

/// `work`, or [`RuntimeError::Stalled`] once `limit` has passed without it finishing.
///
/// The one way a network await in this crate is bounded, so that running out of time is always
/// the same typed error, which [`Retryable::retry`] answers with a reconnect.
pub(crate) async fn within<T>(
    limit: Duration,
    doing: &'static str,
    work: impl std::future::Future<Output = Result<T, RuntimeError>>,
) -> Result<T, RuntimeError> {
    tokio::time::timeout(limit, work).await.unwrap_or_else(|_| {
        Err(RuntimeError::Stalled {
            doing,
            after: limit,
        })
    })
}

/// A JMAP method's refusal is a protocol failure like any other, classified where its type is
/// read (`mail_proto::jmap::MethodError`).
impl From<mail_proto::jmap::MethodError> for RuntimeError {
    fn from(error: mail_proto::jmap::MethodError) -> Self {
        RuntimeError::Proto(error.into())
    }
}

/// A failure dropped on purpose, and not in silence: it goes to the log with what was being done.
///
/// For the places where carrying on is right (a window with an unreadable preference still
/// opens; a list that cannot be read is empty and says so), but where `.ok()` or
/// `unwrap_or_default()` would have left nobody any way to learn why.
pub trait Logged<T> {
    /// The value, or `None` after saying why there is none.
    fn or_log(self, doing: &str) -> Option<T>;

    /// The value, or the default after saying why.
    fn or_log_default(self, doing: &str) -> T
    where
        T: Default;
}

impl<T, E: std::fmt::Display> Logged<T> for Result<T, E> {
    fn or_log(self, doing: &str) -> Option<T> {
        match self {
            Ok(value) => Some(value),
            Err(why) => {
                log::warn!("{doing}: {why}");
                None
            }
        }
    }

    fn or_log_default(self, doing: &str) -> T
    where
        T: Default,
    {
        self.or_log(doing).unwrap_or_default()
    }
}
