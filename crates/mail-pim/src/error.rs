//! What can be wrong with a card or a calendar.

use mail_domain::{Retry, Retryable};

/// The bytes did not hold together.
///
/// Never a panic: a vCard file or a calendar came from another
/// program, and may be anything.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum PimError {
    /// Well-formed text that is not the document asked for, such as an invitation with no event.
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
