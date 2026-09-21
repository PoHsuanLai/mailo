//! The one shape every protocol machine has.
//!
//! This module is part of the frozen interface (see `CONVENTIONS.md` §1). Sessions and
//! backends share it, which is why a backend test is a byte transcript exactly like a session
//! test, and why there is no separate adapter crate.

use mail_domain::{AccountCaps, Ingest, ProtoOp, RemoteRef, Retry, Retryable, Tls};
use std::time::Duration;

/// What a machine wants done before it can continue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IoNeed {
    Write(Vec<u8>),
    /// More bytes. A machine asks for this repeatedly while a response is still incomplete.
    Read,
    Flush,
    OpenTls {
        host: String,
        port: u16,
        mode: Tls,
    },
    /// Wait. Used for poll intervals and for IDLE's re-issue deadline.
    Sleep(Duration),
    Close,
}

/// What happened, fed back into the machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IoReady {
    Bytes(Vec<u8>),
    Eof,
    TlsOpen,
    /// A [`IoNeed::Sleep`] elapsed.
    Woke,
    /// The runtime is asking the machine to wind down gracefully and yield control — the user
    /// acted while an IDLE was outstanding, and the connection is wanted for a command.
    ///
    /// This is the whole cancellation story. A machine that can be interrupted must respond by
    /// emitting whatever the protocol requires (`DONE` for IMAP IDLE) and then finishing.
    Interrupt,
}

/// Where a machine is after being fed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Progress<T> {
    /// Satisfy these, in order, then feed the results back.
    Need(Vec<IoNeed>),
    Done(T),
    Failed(ProtoError),
}

/// A protocol state machine.
///
/// The runtime owns the loop. A machine never asks for bytes itself, which is what keeps this
/// crate free of `async`, makes every transition replayable from a transcript, and leaves the
/// runtime free to `select!` a machine against a cancellation channel.
pub trait Machine {
    /// What this machine produces when it finishes.
    type Out;

    /// Begin. Called once, before any [`Machine::feed`].
    fn start(&mut self) -> Progress<Self::Out>;

    /// Advance on the result of a previously-returned [`IoNeed`].
    fn feed(&mut self, ready: IoReady) -> Progress<Self::Out>;
}

/// What a [`Backend`] produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtoOutcome {
    Ingested(Box<Ingest>),
    Caps(Box<AccountCaps>),
    /// Flags, labels or mailbox membership are now confirmed on the server.
    Applied,
    Submitted {
        /// Where the sent copy landed, when the server filed one. `None` on POP3.
        remote: Option<RemoteRef>,
    },
    /// One message's raw bytes, as the server gave them.
    ///
    /// Not folded into [`ProtoOutcome::Ingested`] because an `Ingest` carries fully-built
    /// `Message` values, and a protocol machine cannot build one: a `Message` needs a
    /// `MessageId`, a `ThreadId` and a stored `BlobId`, none of which the wire supplies. The
    /// runtime stores the bytes, assigns the ids, and parses.
    Fetched {
        /// One entry per message the batch retrieved, in arrival order.
        items: Vec<(RemoteRef, Vec<u8>)>,
    },
    /// A watch saw activity. The runtime schedules a fetch; the machine does not do it itself.
    Woken,
}

/// Turns a [`ProtoOp`] into a walk of the relevant session, then into domain values.
///
/// Same shape as [`Machine`], deliberately.
pub trait Backend {
    /// Begin work on `op`. Called once per operation.
    fn begin(&mut self, op: ProtoOp) -> Progress<ProtoOutcome>;

    /// Advance on the result of a previously-returned [`IoNeed`].
    fn feed(&mut self, ready: IoReady) -> Progress<ProtoOutcome>;

    /// What this backend believes the server supports. Refreshed by [`ProtoOp::FetchCaps`].
    fn caps(&self) -> &AccountCaps;

    /// What the last survey found: every address the server holds, with a size where the
    /// protocol reports one.
    ///
    /// This exists because on a **first** sync the store knows nothing, so asking it what is
    /// missing returns an empty list and the client fetches nothing at all — silently, with no
    /// error, which is exactly how it behaved until an end-to-end test caught it. The server's
    /// own listing is the only source of truth before anything has been stored.
    ///
    /// Empty by default: a protocol that cannot enumerate a mailbox up front has nothing to
    /// report here, and the caller falls back to what the store knows.
    fn surveyed(&self) -> Vec<(RemoteRef, u64)> {
        Vec::new()
    }
}

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
