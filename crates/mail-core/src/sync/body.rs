//! Fetching one message's body on demand, for a reader that opened a message no sync had
//! reached yet.
//!
//! Modelled on [`super::fetch_part`]: blocking, with a runtime of its own, because its callers
//! are a click handler and a command. It differs in what it returns. A part download reports
//! prose; this reports a [`Retry`] beside the prose, so the reader can offer "try again" for a
//! dropped connection and "sign in again" for a rejected credential, which are different
//! buttons.
//!
//! IMAP and Graph address one message by its own reference, so those work. POP3 does not: a
//! message is `RETR`ed by a number that only the session which listed it can resolve, and a
//! single-message fetch has no listing, so the account's sync is the only way a POP3 body
//! arrives. JMAP's engine fetches bodies only as a mailbox pass and offers no call for one
//! email. Both answer [`Retry::Fatal`] with that reason rather than running a whole pass the
//! caller did not ask for.

use super::*;
use mail_domain::{Retry, Retryable as _};

/// Download one message's body and store it, so the reader's next read shows it.
pub fn fetch_body(
    store: Arc<SqliteStore>,
    message: mail_domain::MessageId,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<(), (Retry, String)> {
    let registry =
        OAuthRegistry::load_default().map_err(|e| (Retry::Fatal(e.to_string()), e.to_string()))?;
    fetch_body_with(store, Arc::new(KeyringSecrets), &registry, message, now)
}

/// The same, with the secret store named, so a test can run it.
pub fn fetch_body_with(
    store: Arc<SqliteStore>,
    secrets: Arc<dyn Secrets>,
    registry: &OAuthRegistry,
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
        .map_err(fatal)?
        .into_iter()
        .find(|a| a.id == owner)
        .ok_or_else(|| {
            fatal("the account this message belongs to is no longer configured".to_owned())
        })?;
    match account.plan.incoming {
        Incoming::Imap { .. } | Incoming::Graph => {}
        Incoming::Pop3 { .. } => {
            return Err(fatal(
                "POP3 cannot fetch one message on its own; its bodies arrive with a sync"
                    .to_owned(),
            ));
        }
        Incoming::Jmap { .. } => {
            return Err(fatal(
                "JMAP bodies arrive with a sync; there is no fetch for a single email yet"
                    .to_owned(),
            ));
        }
        Incoming::Local => {
            return Err(fatal(
                "a local account has no server to fetch from".to_owned(),
            ));
        }
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| fatal(format!("cannot start the async runtime: {e}")))?;
    runtime.block_on(async {
        let (_tx, mut cancel) = watch::channel(false);
        let stored = secrets
            .get(&SecretKey {
                account: account.id.clone(),
                purpose: SecretPurpose::IncomingPassword,
            })
            // A credential that is not there is one to be asked for again.
            .map_err(|_| {
                (
                    Retry::NeedsReauth,
                    crate::account::no_credential(&account.address, &account.plan.auth),
                )
            })?;
        let credential = signed_in_typed(&account, stored, secrets.as_ref(), registry, now)
            .await
            .map_err(|f| (f.retry, f.why))?;
        let held = Held::new(credential);
        let renewal = renewal_for(
            &account,
            &held,
            secrets.clone(),
            registry,
            clock_for(Mode::Once, now),
        );
        let failed = |e: mail_runtime::RuntimeError| {
            (e.retry(), format!("cannot download the message: {e}"))
        };
        if let Incoming::Graph = account.plan.incoming {
            sending_token_typed(&account, secrets.as_ref(), registry, now)
                .await
                .map_err(|f| (f.retry, f.why))?;
            let mut engine = graph_engine(&store, &account, secrets).map_err(fatal)?;
            if let Some(renewal) = renewal {
                engine = engine.with_renewal(renewal);
            }
            engine
                .fetch_body(message, &mut cancel, now)
                .await
                .map_err(failed)
        } else {
            let mut engine = imap_engine(&store, &account, held, secrets);
            if let Some(renewal) = renewal {
                engine = engine.with_renewal(renewal);
            }
            engine
                .fetch_body(message, &mut cancel, now)
                .await
                .map_err(failed)
        }
    })
}
