//! Why an edit was refused.
//!
//! A bad position came from the browser or from a caller, so it is an error value. The
//! document is left as it was.

use mail_domain::{Retry, Retryable};
use std::fmt;

/// An edit that does not apply to this document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpError {
    /// `Pos` or `Range` does not fall inside the document.
    OutOfRange,
    /// The edit needs a paragraph and the position is an object.
    NotText,
    /// `Merge` was asked to join an object, or the first node, with a neighbour.
    CannotMerge,
}

impl fmt::Display for OpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutOfRange => write!(f, "position is outside the document"),
            Self::NotText => write!(f, "that edit does not apply to an object"),
            Self::CannotMerge => write!(f, "those two nodes do not join"),
        }
    }
}

impl std::error::Error for OpError {}

impl Retryable for OpError {
    fn retry(&self) -> Retry {
        // The same edit on the same document fails the same way. Nothing to retry.
        Retry::Fatal(self.to_string())
    }
}
