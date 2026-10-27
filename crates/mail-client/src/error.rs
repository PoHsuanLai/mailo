//! Why a value could not be read.

use mail_store::StoreError;

/// Why a [`crate::Live`] value could not be read, kept beside the last value that could.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ClientError {
    /// The store refused, or could not be read: its words, for a front end to put into its own.
    #[error("the mail could not be read: {0}")]
    Store(String),
}

impl From<StoreError> for ClientError {
    fn from(error: StoreError) -> Self {
        ClientError::Store(error.to_string())
    }
}
