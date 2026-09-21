//! Store failures.

use mail_domain::{MessageId, Retry, Retryable, ThreadId};
use std::time::Duration;

/// Something went wrong locally.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database: {0}")]
    Db(String),
    #[error("no such thread: {0}")]
    NoThread(ThreadId),
    #[error("no such message: {0}")]
    NoMessage(MessageId),
    #[error("blob {0}: {1}")]
    Blob(String, String),
    /// A stored value no longer matches its type. Almost always a missing migration or a
    /// `#[serde(default)]` that was not added when a field was.
    #[error("cannot decode stored {what}: {why}")]
    Decode { what: String, why: String },
    #[error("database is newer than this build: schema v{found}, expected v{expected}")]
    SchemaTooNew { found: u32, expected: u32 },
    #[error("malformed pagination cursor")]
    BadCursor,
}

impl Retryable for StoreError {
    fn retry(&self) -> Retry {
        match self {
            // Usually a lock contended by another connection; WAL makes this brief.
            StoreError::Db(_) | StoreError::Blob(_, _) => Retry::After(Duration::from_millis(250)),
            // Our own bug or our own data. Retrying re-runs the same failure forever.
            StoreError::NoThread(_)
            | StoreError::NoMessage(_)
            | StoreError::Decode { .. }
            | StoreError::BadCursor => Retry::Fatal(self.to_string()),
            // Downgrading into an upgraded database. Stop, do not migrate backwards.
            StoreError::SchemaTooNew { .. } => Retry::Fatal(self.to_string()),
        }
    }
}
