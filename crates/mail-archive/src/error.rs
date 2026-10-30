//! Why an archive could not be read.

use mail_domain::{Retry, Retryable};
use std::io;

/// Why an mbox could not be read.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum MboxError {
    #[error("not an mbox: the first line is not a `From ` envelope line")]
    NotMbox,
    #[error("reading the mbox: {0}")]
    Io(#[from] io::Error),
}

impl Retryable for MboxError {
    fn retry(&self) -> Retry {
        // A file already on this computer: the same read gives the same answer.
        Retry::Fatal(self.to_string())
    }
}
