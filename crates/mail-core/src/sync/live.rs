//! Waiting for the server to say something, as data.
//!
//! `mailo watch` waits inside [`super::drive`], between passes it runs itself. The window runs
//! its passes elsewhere and wants only the waiting: one connection kept open, and a typed
//! [`Heard`] each time the server, the outbox or the interval has something to say.
//! [`SyncOps::listen`](crate::SyncOps) is that, built from the same engine calls `drive` makes (`AccountEngine::wait`,
//! `JmapEngine::wait`) and the same floor under them ([`super::poll_floor`]), so the two cannot
//! disagree about how a server is waited on.
//!
//! It runs no pass and holds no lock on the store: [`mail_runtime::AccountEngine::wait`] looks
//! at the outbox with one short query every few seconds and otherwise only sleeps on a socket.
//!
//! # What can be waited on
//!
//! | Incoming | Waits when                                  |
//! |----------|---------------------------------------------|
//! | IMAP     | the stored capabilities say `IDLE`          |
//! | JMAP     | the session names an event source           |
//! | POP3     | never: no server push exists                |
//! | Graph    | never: polled                               |
//! | Local    | never: nothing to fetch                     |

use super::report::Failure;
use super::{Configured, Mode, clock_for, configured, imap_engine, poll_floor, renewal_for};
use super::{signed_in_typed, to_sync};
use crate::error::Logged;
use mail_domain::*;
use mail_runtime::{
    AccountEngine, AccountSecrets, Cancel, ClientRegistry, Held, JmapEngine, RuntimeError, Woke,
};
use mail_store::SqliteStore;
use porter_core::AccountId;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;

/// How long the connection must have stayed up before it is called established.
///
/// A server that accepts the connection and drops it again would otherwise read as "live" for a
/// moment, once per attempt.
pub const GRACE: Duration = Duration::from_secs(2);

/// The longest a wake that was acted on is waited for before listening again.
///
/// See [`Hold`]. A caller that never answers must not stop the watch for ever.
pub const ACT_CEILING: Duration = Duration::from_secs(10 * 60);

/// What the server, the outbox or the clock said.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Heard {
    /// The connection is held and the server is being waited on.
    Established,
    /// The server said something changed.
    Mail,
    /// Something in the outbox came due.
    Due,
    /// The server ended its wait without news, or the interval ran out. Mail may have arrived
    /// in the gap between one wait and the next, so this is when a quiet account is looked at.
    Interval,
}

/// Why the watch is over, when nobody asked it to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lost {
    /// What to do about it: wait, ask for a sign-in, or give up.
    pub retry: Retry,
    /// What to tell a person.
    pub why: String,
}

impl From<Failure> for Lost {
    fn from(failure: Failure) -> Self {
        Lost {
            retry: failure.retry,
            why: failure.why,
        }
    }
}

impl Lost {
    fn of(e: &RuntimeError) -> Self {
        Failure::of("", e).into()
    }

    fn unsupported(why: &str) -> Self {
        Failure::fatal(why).into()
    }
}

/// How the caller lets the watch know it has finished reacting to a wake.
///
/// After [`Heard::Mail`] the stored cursor is out of date until the caller's pass has run, so
/// waiting again at once would be told about the same mail again, in a loop. The caller bumps
/// this channel when its reaction is over; the watch waits for that, for at most
/// [`ACT_CEILING`], before it listens again.
pub type Hold = watch::Receiver<u64>;

/// Whether the account's stored plan and capabilities can be waited on: see the table above.
pub fn pushes(account: &Configured) -> bool {
    matches!(
        account.plan.incoming,
        Incoming::Imap { .. } | Incoming::Jmap { .. }
    ) && matches!(account.caps.watch, WatchMode::Idle)
}

/// Every account that can be waited on, as the store describes them now.
pub fn pushing(store: &SqliteStore) -> Vec<AccountId> {
    configured(store)
        .or_log_default("the accounts could not be read")
        .iter()
        .filter(|account| pushes(account))
        .map(|account| account.id.clone())
        .collect()
}

impl crate::mail::SyncOps<'_> {
    /// Wait on one account until `cancel` fires or the connection is lost, saying what is heard.
    ///
    /// `Ok(())` means `cancel` fired (or its sender was dropped, which the engines read the same
    /// way). `hold` is described at [`Hold`].
    pub async fn listen(
        &self,
        account: AccountId,
        cancel: watch::Receiver<bool>,
        hold: Option<Hold>,
        grace: Duration,
        heard: &dyn Fn(Heard),
    ) -> Result<(), Lost> {
        let mail = self.0;
        let registry = mail
            .clients()
            .map_err(|e| Lost::unsupported(&e.to_string()))?;
        listen_with(
            mail.store().clone(),
            mail.secrets(),
            &registry,
            account,
            cancel,
            hold,
            grace,
            heard,
        )
        .await
    }
}

/// [`SyncOps::listen`](crate::SyncOps), with the secret store named, so a test can run it.
#[allow(clippy::too_many_arguments)]
pub async fn listen_with(
    store: Arc<SqliteStore>,
    secrets: Arc<dyn AccountSecrets>,
    registry: &ClientRegistry,
    account: AccountId,
    cancel: watch::Receiver<bool>,
    hold: Option<Hold>,
    grace: Duration,
    heard: &dyn Fn(Heard),
) -> Result<(), Lost> {
    let account = configured(&store)
        .map_err(|why| Lost::unsupported(&why.to_string()))?
        .into_iter()
        .find(|one| one.id == account)
        .ok_or_else(|| Lost::unsupported("that account is no longer configured"))?;
    let mut waiting = Waiter::open(&store, &account, secrets, registry).await?;
    let mut cancel = cancel;
    hearing(&mut waiting, &mut cancel, hold, grace, heard).await
}

/// An engine, and what it waits with.
enum Waiter {
    Imap {
        engine: Box<AccountEngine<mail_proto::backend::ImapBackend>>,
        inbox: MailboxRef,
        floor: Duration,
    },
    Jmap {
        engine: Box<JmapEngine>,
        floor: Duration,
    },
}

impl Waiter {
    /// Sign in and build the engine for `account`, or say why it cannot be waited on.
    async fn open(
        store: &Arc<SqliteStore>,
        account: &Configured,
        secrets: Arc<dyn AccountSecrets>,
        registry: &ClientRegistry,
    ) -> Result<Self, Lost> {
        if !pushes(account) {
            return Err(Lost::unsupported("this account has no push to wait on"));
        }
        let stored = super::stored_credential(account, secrets.as_ref()).await?;
        // The wall clock, because a watch outlives any `now` it could be handed.
        let now = chrono::Utc::now();
        let credential = signed_in_typed(account, stored, secrets.as_ref(), registry, now).await?;
        let floor = poll_floor(&account.caps.watch);
        match &account.plan.incoming {
            Incoming::Imap { .. } => {
                let held = Held::new(credential);
                let renewal = renewal_for(
                    account,
                    &held,
                    secrets.clone(),
                    registry,
                    clock_for(Mode::Watch, now),
                );
                let mut engine = imap_engine(store, account, held, secrets);
                if let Some(renewal) = renewal {
                    engine = engine.with_tokens(renewal);
                }
                let inbox = to_sync(account, &[])
                    .into_iter()
                    .next()
                    .ok_or_else(|| Lost::unsupported("nothing to watch"))?;
                Ok(Waiter::Imap {
                    engine: Box::new(engine),
                    inbox,
                    floor,
                })
            }
            Incoming::Jmap { .. } => {
                let mut engine = JmapEngine::new(
                    account.id.clone(),
                    account.plan.clone(),
                    store.clone(),
                    secrets,
                )
                .map_err(|e| Lost::of(&e))?;
                engine.connect().await.map_err(|e| Lost::of(&e))?;
                // The stored capabilities said push; the server's own session is the authority.
                if !engine.pushes() {
                    return Err(Lost::unsupported("the server offers no event source"));
                }
                Ok(Waiter::Jmap {
                    engine: Box::new(engine),
                    floor,
                })
            }
            Incoming::Pop3 { .. } | Incoming::Graph | Incoming::Local => {
                Err(Lost::unsupported("this account has no push to wait on"))
            }
        }
    }

    async fn wait(&mut self, cancel: &mut Cancel) -> Result<Woke, RuntimeError> {
        match self {
            Waiter::Imap {
                engine,
                inbox,
                floor,
            } => engine.wait(inbox, cancel, *floor).await,
            Waiter::Jmap { engine, floor } => engine.wait(cancel, *floor).await,
        }
    }
}

/// Wait, say what was heard, and wait again, until cancelled or lost.
async fn hearing(
    waiter: &mut Waiter,
    cancel: &mut Cancel,
    mut hold: Option<Hold>,
    grace: Duration,
    heard: &dyn Fn(Heard),
) -> Result<(), Lost> {
    let mut established = false;
    loop {
        let result = {
            let waited = waiter.wait(cancel);
            tokio::pin!(waited);
            if established {
                waited.await
            } else {
                tokio::select! {
                    result = &mut waited => result,
                    () = tokio::time::sleep(grace) => {
                        established = true;
                        heard(Heard::Established);
                        waited.await
                    }
                }
            }
        };
        match result {
            Ok(woke) => {
                let said = said(woke);
                heard(said);
                if said == Heard::Mail {
                    rest(hold.as_mut(), cancel).await;
                }
            }
            Err(RuntimeError::Cancelled) => return Ok(()),
            Err(e) => return Err(Lost::of(&e)),
        }
    }
}

/// What a wake is told as.
pub fn said(woke: Woke) -> Heard {
    match woke {
        Woke::Mail => Heard::Mail,
        Woke::Due => Heard::Due,
        Woke::Interval => Heard::Interval,
    }
}

/// After a wake for mail: wait until the caller has reacted, the ceiling, or cancellation.
async fn rest(hold: Option<&mut Hold>, cancel: &mut Cancel) {
    let Some(hold) = hold else { return };
    tokio::select! {
        _ = hold.changed() => {}
        () = tokio::time::sleep(ACT_CEILING) => {}
        _ = cancel.changed() => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wake_is_told_as_what_it_was() {
        assert_eq!(said(Woke::Mail), Heard::Mail);
        assert_eq!(said(Woke::Due), Heard::Due);
        assert_eq!(said(Woke::Interval), Heard::Interval);
    }
}
