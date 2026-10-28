//! Why a value could not be read.

use mail_store::StoreError;

/// Why a [`crate::Live`] value could not be read, kept beside the last value that could.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ClientError {
    /// The store refused, or could not be read: its words, for a front end to put into its own.
    #[error("the mail could not be read: {0}")]
    Store(String),
    /// The reading itself failed (a bug in what it ran): said, and the next reading goes on.
    #[error("reading the mail went wrong: {0}")]
    Panicked(String),
}

impl From<StoreError> for ClientError {
    fn from(error: StoreError) -> Self {
        ClientError::Store(error.to_string())
    }
}

impl From<mail_core::CoreError> for ClientError {
    fn from(error: mail_core::CoreError) -> Self {
        ClientError::Store(error.to_string())
    }
}
