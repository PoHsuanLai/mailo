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
    NoPart { message: MessageId, section: String },
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
            // Usually a lock contended by another connection; WAL makes this brief.
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
