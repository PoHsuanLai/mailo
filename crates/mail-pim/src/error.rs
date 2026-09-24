//! What can be wrong with a card or a DAV reply.

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
}

impl Retryable for PimError {
    fn retry(&self) -> Retry {
        // Facts about bytes already received. Asking again gets the same bytes.
        Retry::Fatal(self.to_string())
    }
}
