//! Runtime failures.

use mail_domain::{Retry, Retryable};
use mail_net::{Failure, NetError};
use mail_secrets::{CredentialError, SecretTrouble};
use std::time::Duration;

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
    #[error("cryptography: {0}")]
    Crypto(#[from] mail_crypto::CryptoError),
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
            RuntimeError::Crypto(e) => e.retry(),
            // Delegate: the machines already classify their own failures, including the
            // transient/permanent split and rate limiting.
            RuntimeError::Proto(e) => e.retry(),
            RuntimeError::Store(e) => e.retry(),
            // Missing is a sign-in to ask for; a locked keyring is a wait (`SecretTrouble`).
            RuntimeError::Secrets(failure) => SecretTrouble::of_failure(failure).retry(),
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

/// A failure on the wire is the runtime's failure of the same kind.
impl From<NetError> for RuntimeError {
    fn from(error: NetError) -> Self {
        match error {
            NetError::Connect(why) => RuntimeError::Connect(why),
            NetError::Tls(why) => RuntimeError::Tls(why),
            NetError::Io(why) => RuntimeError::Io(why),
            NetError::Stalled { doing, after } => RuntimeError::Stalled { doing, after },
            NetError::Unsupported(why) => RuntimeError::UnsupportedIo(why),
        }
    }
}

/// A failure over a credential is the runtime's failure of the same kind.
impl From<CredentialError> for RuntimeError {
    fn from(error: CredentialError) -> Self {
        match error {
            CredentialError::Secrets(why) => RuntimeError::Secrets(why),
            CredentialError::Io(why) => RuntimeError::Io(why),
            CredentialError::Net(error) => error.into(),
        }
    }
}

impl RuntimeError {
    /// What this failure, from reading a secret, asks of the person: [`SecretTrouble::of`] for
    /// the failures that are the store's, a sign-in for anything else.
    pub fn secret_trouble(&self) -> SecretTrouble {
        match self {
            RuntimeError::Secrets(failure) => SecretTrouble::of_failure(failure),
            _ => SecretTrouble::Absent,
        }
    }
}

/// A server's refusal of a token the issuer has just issued, said as what it is.
///
/// The rule every engine follows after `TokenSource::after_refusal`: a refusal of the token it
/// minted (`AfterRefusal::TryAgain`, then refused), or of one it minted earlier
/// (`AfterRefusal::StillRefused`), is not [`Retry::NeedsReauth`]. The issuer is the authority
/// on the sign-in and has just renewed it, so asking the user to sign in again sends them
/// round a loop that mints the same token: a Microsoft 365 mailbox with IMAP or SMTP switched
/// off answers every token with `NO AUTHENTICATE failed.`, and read as a refused credential it
/// asked for a sign-in straight after one had succeeded. Anything that is not a refused
/// credential passes through as it was.
pub fn refused_fresh(error: RuntimeError) -> RuntimeError {
    if matches!(error.retry(), Retry::NeedsReauth) {
        RuntimeError::RefusedFresh(Box::new(error))
    } else {
        error
    }
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
