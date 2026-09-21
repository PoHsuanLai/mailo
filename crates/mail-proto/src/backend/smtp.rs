//! Submission as a [`Backend`].
//!
//! Separate from the incoming backends because it is a separate connection to a separate host.
//! Both `Pop3Backend` and a future `ImapBackend` refuse `ProtoOp::Submit`; the runtime routes it
//! here.

use crate::machine::{Backend, IoReady, Machine, Progress, ProtoError, ProtoOutcome};
use crate::smtp::{SmtpSession, Submission};
use mail_domain::{AccountCaps, AccountId, DraftId, ProtoOp};

/// Builds a session for one submission.
///
/// Takes the raw message because [`Submission`] carries it, and the credential because SMTP
/// authenticates per connection. A closure for the same reason as POP3's: the backend never
/// holds a password and so cannot leak one.
pub type SubmissionFactory = Box<dyn FnMut(Vec<u8>) -> Result<Submission, ProtoError> + Send>;

/// Sends one message per operation.
pub struct SmtpBackend {
    account: AccountId,
    caps: AccountCaps,
    build: SubmissionFactory,
    session: Option<SmtpSession>,
    /// The draft in flight, so the outcome can name what was sent.
    draft: Option<DraftId>,
}

impl SmtpBackend {
    /// A backend that submits through `build`.
    ///
    /// `raw` is supplied by the caller rather than fetched here: the bytes live in the blob
    /// store, which this crate deliberately knows nothing about.
    pub fn new(account: AccountId, caps: AccountCaps, build: SubmissionFactory) -> Self {
        Self {
            account,
            caps,
            build,
            session: None,
            draft: None,
        }
    }

    /// The account this backend submits for.
    pub fn account(&self) -> AccountId {
        self.account
    }

    /// Hand over the bytes for the next [`ProtoOp::Submit`].
    ///
    /// `ProtoOp::Submit` names a `BlobId`, and resolving one means reading the blob store, which
    /// is above this crate. The runtime reads the blob and calls this first.
    pub fn stage(&mut self, raw: Vec<u8>) -> Result<(), ProtoError> {
        let submission = (self.build)(raw)?;
        self.session = Some(SmtpSession::new(submission));
        Ok(())
    }
}

impl Backend for SmtpBackend {
    fn begin(&mut self, op: ProtoOp) -> Progress<ProtoOutcome> {
        let ProtoOp::Submit { draft, .. } = op else {
            return Progress::Failed(ProtoError::Unsupported(
                "this backend only submits".to_owned(),
            ));
        };
        let Some(session) = self.session.as_mut() else {
            // Staging is a separate step because the bytes come from the blob store. Failing
            // loudly beats sending an empty message.
            return Progress::Failed(ProtoError::Malformed(
                "submission was not staged: call stage() with the message bytes".to_owned(),
            ));
        };
        self.draft = Some(draft);
        match session.start() {
            Progress::Need(needs) => Progress::Need(needs),
            Progress::Done(_) => Progress::Failed(ProtoError::Malformed(
                "session finished before sending anything".to_owned(),
            )),
            Progress::Failed(e) => Progress::Failed(e),
        }
    }

    fn feed(&mut self, ready: IoReady) -> Progress<ProtoOutcome> {
        let Some(session) = self.session.as_mut() else {
            return Progress::Failed(ProtoError::Malformed(
                "bytes arrived with no submission in flight".to_owned(),
            ));
        };
        match session.feed(ready) {
            Progress::Need(needs) => Progress::Need(needs),
            Progress::Failed(e) => {
                // Drop the session: a failed submission must not be resumed mid-DATA, and the
                // outbox decides whether to retry from the beginning.
                self.session = None;
                Progress::Failed(e)
            }
            Progress::Done(_reply) => {
                self.session = None;
                self.draft = None;
                // `remote: None`: SMTP tells us the message was accepted, not where a copy was
                // filed. On Gmail the server files it in Sent itself and a client APPEND would
                // duplicate it; on POP3 accounts there is no Sent folder at all.
                Progress::Done(ProtoOutcome::Submitted { remote: None })
            }
        }
    }

    fn caps(&self) -> &AccountCaps {
        &self.caps
    }
}

// Written by hand: the factory closes over the account's credential, so a derived Debug would
// not compile and making the closure Debug would put a password one `{:?}` from a log.
impl std::fmt::Debug for SmtpBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SmtpBackend")
            .field("account", &self.account)
            .field("staged", &self.session.is_some())
            .field("draft", &self.draft)
            .finish()
    }
}
