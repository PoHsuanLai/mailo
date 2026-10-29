//! Fetching one message's body on demand, for a reader that opened a message no sync had
//! reached yet.
//!
//! Modelled on [`super::fetch_part_with`]. It differs in what it returns. A part download reports
//! prose; this reports a [`Retry`] beside the prose, so the reader can offer "try again" for a
//! dropped connection and "sign in again" for a rejected credential, which are different
//! buttons.
//!
//! IMAP and Graph address one message by its own reference, so those work. POP3 does not: a
//! message is `RETR`ed by a number that only the session which listed it can resolve, and a
//! single-message fetch has no listing, so the account's sync is the only way a POP3 body
//! arrives. JMAP's engine fetches bodies only as a mailbox pass and offers no call for one
//! email. Those, and local mail, answer [`Retry::Fatal`] ([`super::access`]) rather than running
//! a whole pass the caller did not ask for.

use super::access::access;
use super::*;
use mail_domain::Retry;

impl crate::mail::SyncOps<'_> {
    /// Download one message's body and store it, so the reader's next read shows it.
    pub async fn fetch_body(&self, message: mail_domain::MessageId) -> Result<(), (Retry, String)> {
        let mail = self.0;
        let registry = mail
            .clients()
            .map_err(|e| (Retry::Fatal(e.to_string()), e.to_string()))?;
        fetch_body_with(
            mail.store().clone(),
            mail.secrets(),
            &registry,
            message,
            mail.now(),
        )
        .await
    }
}

/// [`SyncOps::fetch_body`](crate::SyncOps), with the secret store named, so a test can run it.
pub async fn fetch_body_with(
    store: Arc<SqliteStore>,
    secrets: Arc<dyn AccountSecrets>,
    registry: &ClientRegistry,
    message: mail_domain::MessageId,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<(), (Retry, String)> {
    use mail_store::Store as _;
    let fatal = |why: String| (Retry::Fatal(why.clone()), why);
    let owner = store
        .message(message)
        .map_err(|e| fatal(e.to_string()))?
        .account;
    let account = configured(&store)
        .map_err(|e| fatal(e.to_string()))?
        .into_iter()
        .find(|a| a.id == owner)
        .ok_or_else(|| {
            fatal("the account this message belongs to is no longer configured".to_owned())
        })?;
    let run = Run {
        store: &store,
        secrets,
        registry,
        now,
        account: &account,
    };
    access(&account.plan.incoming)
        .body(&run, message)
        .await
        .map_err(|f| (f.retry, f.why))
}
