//! What can be wrong with a card, a calendar or a DAV reply.

use mail_domain::{Retry, Retryable};

/// The bytes did not hold together.
///
/// Never a panic: a vCard file came from another program and a DAV reply from a server, and
/// either may be anything.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PimError {
    /// A DAV reply that is not well-formed XML.
    #[error("the server's reply is not XML: {0}")]
    Xml(String),
    /// Well-formed XML that is not the document asked for, such as a reply with no
    /// `multistatus` at its root.
    #[error("the server's reply is not a {expected}")]
    Unexpected { expected: &'static str },
    /// More text than a calendar object is allowed to be here.
    #[error("the calendar is larger than {limit} bytes")]
    TooLarge { limit: usize },
    /// An invitation that cannot be answered as asked: it names no organiser, or does not list
    /// the one answering.
    #[error("{0}")]
    Unanswerable(&'static str),
}

impl Retryable for PimError {
    fn retry(&self) -> Retry {
        // Facts about bytes already received. Asking again gets the same bytes.
        Retry::Fatal(self.to_string())
    }
}
