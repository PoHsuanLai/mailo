//! Store failures.

use mail_domain::{
    CertFingerprint, DraftId, Fingerprint, MessageId, Retry, Retryable, RuleId, TemplateId,
    ThreadId, ViewId,
};
use porter_core::AccountId;
use std::time::Duration;

/// What a failure carries as its cause, kept as the error it was rather than as its text.
pub type Source = Box<dyn std::error::Error + Send + Sync + 'static>;

/// Something went wrong locally.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// The database refused or could not be reached. The cause is kept, so a caller that wants
    /// to look can `source()` its way to it; it is not the store's business to flatten it.
    #[error("database: {0}")]
    Db(#[source] Source),
    /// A write broke a rule the database holds (a name taken, a row another row depends on):
    /// told apart from [`StoreError::Db`] because the caller can answer it, by choosing another
    /// name or by reading what is there, where a refused disk it can only report.
    #[error("conflicts with what is stored: {0}")]
    Conflict(#[source] Source),
    #[error("no such account: {0}")]
    NoAccount(AccountId),
    #[error("no such thread: {0}")]
    NoThread(ThreadId),
    #[error("no such message: {0}")]
    NoMessage(MessageId),
    #[error("no such draft: {0}")]
    NoDraft(DraftId),
    #[error("no such template: {0}")]
    NoTemplate(TemplateId),
    #[error("no such view: {0}")]
    NoView(ViewId),
    #[error("no such rule: {0}")]
    NoRule(RuleId),
    /// Rule names are how rules are named on the command line, so one account has one of each.
    #[error("there is already a rule called {0:?}")]
    RuleNameTaken(String),
    #[error("no OpenPGP key {0}")]
    NoPgpKey(Fingerprint),
    #[error("no S/MIME certificate {0}")]
    NoSmimeCert(CertFingerprint),
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
    #[error("message {message} has no part {section} still on the server")]
    NoPart {
        message: MessageId,
        section: mail_domain::Section,
    },
    /// A contact was asked for under something that is not an address.
    #[error("{0:?} is not an email address")]
    BadAddress(String),
}

impl StoreError {
    /// A failure of the database, whatever it was: a `rusqlite` error, or words for one the
    /// in-memory store makes up.
    ///
    /// A refusal because a constraint would break is [`StoreError::Conflict`], told from the
    /// error itself so that every statement in the store says the same thing about it.
    pub(crate) fn db(cause: impl Into<Source>) -> Self {
        let cause = cause.into();
        let breaks_a_rule = cause
            .downcast_ref::<rusqlite::Error>()
            .and_then(rusqlite::Error::sqlite_error_code)
            == Some(rusqlite::ErrorCode::ConstraintViolation);
        if breaks_a_rule {
            StoreError::Conflict(cause)
        } else {
            StoreError::Db(cause)
        }
    }
}

impl StoreError {
    /// Whether the database said "not now" rather than "no": another connection holds the lock
    /// (`SQLITE_BUSY`, `SQLITE_LOCKED`). The one failure of the store that going away and trying
    /// again is the remedy for; a [`StoreError::Conflict`] or a missing row will answer the same
    /// way every time.
    pub fn is_busy(&self) -> bool {
        let StoreError::Db(cause) = self else {
            return false;
        };
        matches!(
            cause
                .downcast_ref::<rusqlite::Error>()
                .and_then(rusqlite::Error::sqlite_error_code),
            Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked)
        )
    }
}

/// Words about what was being done, kept in front of the error that stopped it.
#[derive(Debug, thiserror::Error)]
#[error("{doing}: {source}")]
pub(crate) struct Context {
    pub doing: String,
    #[source]
    pub source: Source,
}

impl Context {
    pub(crate) fn new(doing: impl Into<String>, source: impl Into<Source>) -> Self {
        Self {
            doing: doing.into(),
            source: source.into(),
        }
    }
}

impl Retryable for StoreError {
    fn retry(&self) -> Retry {
        match self {
            // Usually a lock contended by another connection ([`StoreError::is_busy`]); WAL makes
            // this brief. Any other refusal of the disk is given the same short wait.
            StoreError::Db(_) | StoreError::Blob(_, _) => Retry::After(Duration::from_millis(250)),
            // Our own bug or our own data. Retrying re-runs the same failure forever.
            StoreError::NoThread(_)
            | StoreError::NoMessage(_)
            | StoreError::NoDraft(_)
            | StoreError::NoTemplate(_)
            | StoreError::NoView(_)
            | StoreError::NoRule(_)
            | StoreError::RuleNameTaken(_)
            | StoreError::Conflict(_)
            | StoreError::NoAccount(_)
            | StoreError::NoPgpKey(_)
            | StoreError::NoSmimeCert(_)
            | StoreError::NoPart { .. }
            | StoreError::BadAddress(_)
            | StoreError::Decode { .. }
            | StoreError::BadCursor => Retry::Fatal(self.to_string()),
            // Downgrading into an upgraded database. Stop, do not migrate backwards.
            StoreError::SchemaTooNew { .. } => Retry::Fatal(self.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failure(code: i32) -> StoreError {
        StoreError::db(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(code),
            None,
        ))
    }

    #[test]
    fn a_contended_lock_is_busy_and_nothing_else_is() {
        assert!(failure(rusqlite::ffi::SQLITE_BUSY).is_busy());
        assert!(failure(rusqlite::ffi::SQLITE_LOCKED).is_busy());
        assert!(!failure(rusqlite::ffi::SQLITE_CORRUPT).is_busy());
        assert!(!failure(rusqlite::ffi::SQLITE_CONSTRAINT).is_busy());
        assert!(!StoreError::BadCursor.is_busy());
    }

    #[test]
    fn a_busy_database_is_tried_again_and_a_conflict_is_not() {
        assert!(matches!(
            failure(rusqlite::ffi::SQLITE_BUSY).retry(),
            Retry::After(_)
        ));
        assert!(matches!(
            failure(rusqlite::ffi::SQLITE_CONSTRAINT).retry(),
            Retry::Fatal(_)
        ));
    }
}
