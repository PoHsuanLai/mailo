//! Why a domain value was refused: a folder change, or text that is not the value it names.
//!
//! Every type here answers [`Retryable`] with a fatal verdict. Each is a fact about the request
//! or the text, so asking again gets the same answer.

use crate::folder::SpecialUse;
use crate::retry::{Retry, Retryable};

/// Why a folder change was refused before anything was sent.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FolderError {
    /// POP3 has one mailbox, and nothing to name.
    #[error(
        "a POP3 account has exactly one mailbox; there are no folders to create, rename or delete"
    )]
    SingleMailbox,
    /// Mail kept only on this computer has no server to hold folders; its places are labels.
    #[error("this account's mail is kept on this computer; it has no server folders, only labels")]
    KeptLocally,
    /// A special-use mailbox, or the inbox, or a folder holding one.
    #[error("{path} is the account's {} folder; the server and other clients depend on it", special.name())]
    Special { path: String, special: SpecialUse },
    #[error("{path} holds {messages} message(s); say so explicitly to delete it with them")]
    NotEmpty { path: String, messages: u64 },
    #[error("there is already a folder called {0}")]
    Exists(String),
    #[error("there is no folder called {0}; `mailo sync` refreshes the list")]
    Unknown(String),
    #[error("{0} has folders inside it; rename or delete those first")]
    HasChildren(String),
    #[error("{name:?} cannot be a folder name: {why}")]
    BadName { name: String, why: String },
}

impl Retryable for FolderError {
    fn retry(&self) -> Retry {
        // Every one of these is a fact about the request, not the moment.
        Retry::Fatal(self.to_string())
    }
}

/// Why text is not a key id, a key fingerprint or a certificate fingerprint.
///
/// The text is carried with every variant because the usual place this is shown is beside a
/// command-line argument or a stored row, and the reader needs to see which one.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ParseFingerprintError {
    /// Hex comes in pairs of digits.
    #[error("{0:?} has an odd number of hex digits")]
    OddDigits(String),
    /// A character that is not a hex digit.
    #[error("{0:?} is not hex")]
    NotHex(String),
    /// Hex, but not the length of the thing asked for. `what` completes "is not a ...".
    #[error("{text:?} is not a {what}")]
    WrongLength { text: String, what: &'static str },
}

impl Retryable for ParseFingerprintError {
    fn retry(&self) -> Retry {
        Retry::Fatal(self.to_string())
    }
}

/// Why text is not a mailbox address.
///
/// Not a full RFC 5322 validation: that accepts things no mail server does, and rejecting what a
/// person's own server accepts is worse than letting the server answer. These are the mistakes
/// people actually make.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ParseAddressError {
    /// Nothing between the angle brackets, or nothing at all.
    #[error("{0:?} has no address")]
    Empty(String),
    /// Not exactly one `@` with something either side of it.
    #[error("{0:?} is not an email address")]
    NotAnAddress(String),
    /// A space or other whitespace inside the address.
    #[error("{0:?} contains a space")]
    Whitespace(String),
}

impl Retryable for ParseAddressError {
    fn retry(&self) -> Retry {
        Retry::Fatal(self.to_string())
    }
}

/// Why text is not an IMAP body section (RFC 3501 section 6.4.5).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0:?} is not an IMAP body section")]
pub struct ParseSectionError(pub String);

impl Retryable for ParseSectionError {
    fn retry(&self) -> Retry {
        Retry::Fatal(self.to_string())
    }
}
