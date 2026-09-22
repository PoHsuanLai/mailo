//! What a client and the daemon say to each other.
//!
//! Newline-delimited JSON, one message per line. Deliberately boring: the interesting problems
//! in this project are in the mail protocols, and inventing a framing here would mean debugging
//! two wire formats instead of one.
//!
//! # Versioned from the first line
//!
//! Every message carries [`VERSION`]. A daemon left running across an upgrade is the normal
//! case, not the exception — `mailo watch` holds IDLE connections for hours and nobody restarts
//! it to install a new binary — so the first thing a new client meets is an old daemon. It has
//! to be told, not left to misparse a field.

use serde::{Deserialize, Serialize};

/// The version of everything below. Bump it when a variant changes meaning; adding a variant
/// with `#[serde(other)]` handling on the far side does not need one.
pub const VERSION: u32 = 1;

/// One message, with the version it was written by.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Framed<T> {
    pub version: u32,
    pub body: T,
}

impl<T> Framed<T> {
    pub fn now(body: T) -> Self {
        Self {
            version: VERSION,
            body,
        }
    }
}

/// What a client asks for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Request {
    /// Is anything there, and what is it. The cheapest possible call, which is what
    /// `connect_or_start` uses to tell a live daemon from a stale socket file.
    Ping,
    /// Run a pass now, rather than waiting for the interval or for IDLE to say something.
    SyncNow,
    /// Stop. Used by tests and by an upgrade that wants the old daemon gone.
    Shutdown,
}

/// What the daemon answers.
///
/// Every request can be answered by `Refused`, which is why it carries a sentence rather than a
/// code: the thing reading it is a person, through a CLI that will print it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Response {
    /// Alive, and what it is.
    Pong {
        pid: u32,
        version: String,
    },
    /// A pass has been asked for. Not "a pass has finished" — a first sync takes minutes, and a
    /// request that blocked for that long would be a request nothing could cancel.
    Started,
    Stopping,
    Refused(String),
    /// The daemon speaks a different version. Both are named so the client can say which way
    /// round the mismatch is, which decides whether the answer is "restart the daemon" or
    /// "upgrade this client".
    WrongVersion {
        daemon: u32,
        client: u32,
    },
}

/// Serialise one message as a line.
pub fn line<T: Serialize>(body: T) -> Result<String, String> {
    let mut out = serde_json::to_string(&Framed::now(body)).map_err(|e| e.to_string())?;
    // The framing *is* the newline, so a value that contained one would end the message early.
    // `serde_json::to_string` never emits a raw newline — it escapes them — and this asserts the
    // property the framing depends on rather than trusting it.
    debug_assert!(
        !out.contains('\n'),
        "a message contained its own terminator"
    );
    out.push('\n');
    Ok(out)
}

/// Read one message back, refusing a version this build does not speak.
///
/// The version is checked before the body is interpreted, so a field that changed meaning is
/// never read under the old meaning.
pub fn parse<T: for<'de> Deserialize<'de>>(text: &str) -> Result<T, Mismatch> {
    let framed: Framed<serde_json::Value> =
        serde_json::from_str(text.trim_end()).map_err(|e| Mismatch::Unreadable(e.to_string()))?;
    if framed.version != VERSION {
        return Err(Mismatch::Version {
            theirs: framed.version,
            ours: VERSION,
        });
    }
    serde_json::from_value(framed.body).map_err(|e| Mismatch::Unreadable(e.to_string()))
}

/// Why a message could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mismatch {
    Version { theirs: u32, ours: u32 },
    Unreadable(String),
}

impl std::fmt::Display for Mismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Mismatch::Version { theirs, ours } => write!(
                f,
                "the daemon speaks version {theirs} and this build speaks {ours}; \
                 restart it with `mailo daemon --replace`"
            ),
            Mismatch::Unreadable(why) => write!(f, "unreadable message: {why}"),
        }
    }
}
