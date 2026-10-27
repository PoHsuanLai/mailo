//! Waiting for the server to say something, as data.
//!
//! The scheduler (`mail_runtime::schedule`) runs the passes, and wants only the waiting from
//! here: one connection kept open (and, for an IMAP server with no `NOTIFY`, a few more for the
//! folders besides the inbox), and a typed [`Heard`] each time the server, the outbox or the
//! interval has something to say. [`SyncOps::listen`](crate::SyncOps) is that, built from the
//! engine calls a pass makes (`AccountEngine::wait`, `JmapEngine::wait`) and the floor under
//! them ([`super::poll_floor`]), so the window and `mailo watch` cannot disagree about how a
//! server is waited on.
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
//! | Graph    | a push relay is set ([`super::graph_push`]) |
//! | Local    | never: nothing to fetch                     |
//!
//! A Graph account with no relay is polled, as it always was; with one, it is waited on through
//! the relay's WebSocket (`mail_runtime::graph::push`) at the floor of its poll interval, so the
//! relay only brings new mail sooner, and a relay that cannot be reached leaves it polling.
//!
//! And only an account checked automatically ([`crate::offline::Every::Automatic`]): one the
//! person set to an interval is polled at it, and one set to manually holds no connection at all.

use super::report::Failure;
use super::{
    Configured, Mode, clock_for, configured, imap_engine, poll_floor, pushed_folders, renewal_for,
};
use super::{signed_in_typed, to_sync};
use crate::error::Logged;
use mail_domain::*;
use mail_runtime::graph::push::{Listener, SUBSCRIPTIONS};
use mail_runtime::{
    AccountEngine, AccountSecrets, Cancel, ClientRegistry, Held, JmapEngine, Notify, RuntimeError,
    Woke,
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

/// The most connections of their own a watch opens to hear of folders besides the inbox, for a
/// server with no `NOTIFY`. Fixed and small: every one is a connection the server counts
/// against the account ([`ConnectionBudget`]).
pub const EXTRA_IDLE: usize = 2;

/// How many folders besides the inbox this account may be watched on connections of their own.
///
/// What the server's budget leaves after the two a client always needs — one to watch, one to
/// work — and never more than [`EXTRA_IDLE`]. A server that allows no more than that watches
/// the inbox alone and the rest wait for the interval.
pub fn extra_idle(caps: &AccountCaps) -> usize {
    usize::from(caps.connections.max)
        .saturating_sub(2)
        .min(EXTRA_IDLE)
}

/// How long extra connections have to end their IDLE in protocol once the wait is over.
const WIND_DOWN: Duration = Duration::from_secs(3);

pub use mail_runtime::schedule::{Heard, Lost};

impl From<Failure> for Lost {
    fn from(failure: Failure) -> Self {
        Lost {
            retry: failure.retry,
            why: failure.why,
        }
    }
}

/// Why waiting on the server failed, as the scheduler hears it.
fn lost(e: &RuntimeError) -> Lost {
    Failure::of("", e).into()
}

/// Why this account cannot be waited on at all.
fn unsupported(why: &str) -> Lost {
    Failure::fatal(why).into()
}

pub use mail_runtime::schedule::Hold;

/// Whether the account's stored plan and capabilities can be waited on, and the person left it to
/// be: see the table above.
///
/// A Graph account is not here: whether it is pushed to is a setting the store holds, which
/// [`pushing_with`] reads ([`super::graph_push::relayed`]).
pub fn pushes(account: &Configured) -> bool {
    matches!(
        account.plan.incoming,
        Incoming::Imap { .. } | Incoming::Jmap { .. }
    ) && matches!(account.caps.watch, WatchMode::Idle)
        && account.every == crate::offline::Every::Automatic
}

/// Every account that can be waited on, as the store describes them now and the setting in the
/// user's config directory chooses ([`pushing_with`]).
pub fn pushing(store: &SqliteStore) -> Vec<AccountId> {
    pushing_with(store, &crate::offline::load_default())
}

/// Every account that can be waited on, as the store describes them now and `kept` chooses.
pub fn pushing_with(store: &SqliteStore, kept: &crate::offline::Kept) -> Vec<AccountId> {
    configured(store)
        .or_log_default("the accounts could not be read")
        .into_iter()
        .map(|account| super::kept_as(account, kept))
        .filter(|account| {
            pushes(account)
                || (account.every == crate::offline::Every::Automatic
                    && super::graph_push::relayed(store, account))
        })
        .filter(|account| !super::paused(store, account.id.clone()))
        .map(|account| account.id)
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
        let registry = mail.clients().map_err(|e| unsupported(&e.to_string()))?;
        listen_with(
            mail.store().clone(),
            mail.secrets(),
            &registry,
            account,
            Watch {
                cancel,
                hold,
                grace,
            },
            heard,
        )
        .await
    }
}

/// How long a watch lasts and how it is paced: the signal that ends it, the hold on its
/// reconnects ([`Hold`]), and the grace after which a wait that has not failed counts as
/// established.
#[derive(Debug)]
pub struct Watch {
    pub cancel: watch::Receiver<bool>,
    pub hold: Option<Hold>,
    pub grace: Duration,
}

/// [`SyncOps::listen`](crate::SyncOps), with the secret store named, so a test can run it.
pub async fn listen_with(
    store: Arc<SqliteStore>,
    secrets: Arc<dyn AccountSecrets>,
    registry: &ClientRegistry,
    account: AccountId,
    Watch {
        cancel,
        hold,
        grace,
    }: Watch,
    heard: &dyn Fn(Heard),
) -> Result<(), Lost> {
    listen_at(
        store,
        secrets,
        registry,
        SUBSCRIPTIONS,
        account,
        cancel,
        hold,
        grace,
        heard,
    )
    .await
}

/// [`listen_with`], with Graph's subscriptions at `subscriptions` rather than Microsoft's: a
/// test's own listener.
#[allow(clippy::too_many_arguments)]
pub async fn listen_at(
    store: Arc<SqliteStore>,
    secrets: Arc<dyn AccountSecrets>,
    registry: &ClientRegistry,
    subscriptions: &str,
    account: AccountId,
    cancel: watch::Receiver<bool>,
    hold: Option<Hold>,
    grace: Duration,
    heard: &dyn Fn(Heard),
) -> Result<(), Lost> {
    let account = configured(&store)
        .map_err(|why| unsupported(&why.to_string()))?
        .into_iter()
        .find(|one| one.id == account)
        .ok_or_else(|| unsupported("that account is no longer configured"))?;
    let mut waiting = Waiter::open(&store, &account, secrets, registry, subscriptions).await?;
    let mut cancel = cancel;
    hearing(&mut waiting, &mut cancel, hold, grace, heard).await
}

/// An engine, and what it waits with.
enum Waiter {
    Imap {
        engine: Box<AccountEngine<mail_proto::backend::ImapBackend>>,
        inbox: MailboxRef,
        floor: Duration,
        /// Engines for folders besides the inbox, used only where the server has no `NOTIFY`
        /// for the main one to hear of them through. Opened lazily: building one connects to
        /// nothing.
        extras: Vec<(MailboxRef, AccountEngine<mail_proto::backend::ImapBackend>)>,
    },
    Jmap {
        engine: Box<JmapEngine>,
        floor: Duration,
    },
    /// A Graph account, through its relay.
    Graph {
        listener: Box<Listener>,
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
        subscriptions: &str,
    ) -> Result<Self, Lost> {
        if super::graph_push::relayed(store, account) {
            return Self::relayed(store, account, secrets, registry, subscriptions).await;
        }
        if !pushes(account) {
            return Err(unsupported("this account has no push to wait on"));
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
                let folders = {
                    use mail_store::Store as _;
                    store
                        .folders(account.id.clone())
                        .or_log_default("the folders could not be read to watch them")
                };
                let others = pushed_folders(account, &folders);
                let build = |held: Held| {
                    let engine = imap_engine(store, account, held, secrets.clone());
                    match &renewal {
                        Some(renewal) => engine.with_tokens(renewal.clone()),
                        None => engine,
                    }
                };
                let extras = others
                    .iter()
                    .take(extra_idle(&account.caps))
                    .map(|m| (m.clone(), build(held.clone())))
                    .collect();
                let engine = build(held).with_also(others);
                let inbox = to_sync(account, &[])
                    .into_iter()
                    .next()
                    .ok_or_else(|| unsupported("nothing to watch"))?;
                Ok(Waiter::Imap {
                    engine: Box::new(engine),
                    inbox,
                    floor,
                    extras,
                })
            }
            Incoming::Jmap { .. } => {
                let mut engine = JmapEngine::new(
                    account.id.clone(),
                    account.plan.clone(),
                    store.clone(),
                    secrets,
                )
                .map_err(|e| lost(&e))?;
                engine.connect().await.map_err(|e| lost(&e))?;
                // The stored capabilities said push; the server's own session is the authority.
                if !engine.pushes() {
                    return Err(unsupported("the server offers no event source"));
                }
                Ok(Waiter::Jmap {
                    engine: Box::new(engine),
                    floor,
                })
            }
            Incoming::Pop3 { .. } | Incoming::Graph | Incoming::Local => {
                Err(unsupported("this account has no push to wait on"))
            }
        }
    }

    /// A Graph account's relay, set up as far as it is not already: see `mail_runtime::graph::push`.
    async fn relayed(
        store: &Arc<SqliteStore>,
        account: &Configured,
        secrets: Arc<dyn AccountSecrets>,
        registry: &ClientRegistry,
        subscriptions: &str,
    ) -> Result<Self, Lost> {
        let AuthPlan::OAuth { issuer, .. } = &account.plan.auth else {
            return Err(unsupported("a Graph account signs in with Microsoft"));
        };
        let client = mail_runtime::clients::client(registry, *issuer)
            .ok_or_else(|| Lost::from(Failure::reauth(crate::account::no_client_id(*issuer))))?
            .clone();
        let listener = Listener::open(
            account.id.clone(),
            &account.plan,
            store.clone(),
            secrets,
            client,
            subscriptions,
        )
        .await
        .map_err(|e| lost(&e))?;
        Ok(Waiter::Graph {
            listener: Box::new(listener),
            floor: poll_floor(&account.caps.watch),
        })
    }

    async fn wait(&mut self, cancel: &mut Cancel) -> Result<Woke, RuntimeError> {
        match self {
            Waiter::Imap {
                engine,
                inbox,
                floor,
                extras,
            } => {
                // Through `NOTIFY` where the server has it, the one connection hears of every
                // folder. Without it, one connection each, up to the limit.
                if extras.is_empty() || engine.notifies() != Notify::Absent {
                    return engine.wait(inbox, cancel, *floor).await;
                }
                wait_on_each(engine, inbox, *floor, extras, cancel).await
            }
            Waiter::Jmap { engine, floor } => engine.wait(cancel, *floor).await,
            Waiter::Graph { listener, floor } => listener.wait(cancel, *floor).await,
        }
    }
}

/// [`AccountEngine::wait`] on the inbox, with every extra folder idled on a connection of its
/// own meanwhile: whichever has news ends the wait as mail.
///
/// An extra that fails, or whose server ends its IDLE without news, drops out of this wait and
/// is tried again on the next: a folder another client deleted must not end the account's push.
async fn wait_on_each(
    engine: &mut AccountEngine<mail_proto::backend::ImapBackend>,
    inbox: &MailboxRef,
    floor: Duration,
    extras: &mut [(MailboxRef, AccountEngine<mail_proto::backend::ImapBackend>)],
    cancel: &mut Cancel,
) -> Result<Woke, RuntimeError> {
    use futures_util::StreamExt;
    use futures_util::stream::FuturesUnordered;

    let (stop, stopped) = watch::channel(false);
    let mut others: FuturesUnordered<_> = extras
        .iter_mut()
        .map(|(mailbox, engine)| {
            let mut stopped = stopped.clone();
            async move { engine.watch(mailbox, &mut stopped).await }
        })
        .collect();
    let primary = engine.wait(inbox, cancel, floor);
    tokio::pin!(primary);

    let woke = loop {
        tokio::select! {
            woke = &mut primary => break woke,
            Some(heard) = others.next() => match heard {
                Ok(true) => break Ok(Woke::Mail),
                Ok(false) => {}
                Err(e) => log::warn!("a folder could not be watched: {e}"),
            },
        }
    };
    // Ended in protocol, so each connection stays usable; bounded, so a dead one cannot hold
    // the wait.
    let _ = stop.send(true);
    let drained = async { while others.next().await.is_some() {} };
    if tokio::time::timeout(WIND_DOWN, drained).await.is_err() {
        log::debug!("an extra connection did not end its IDLE in time");
    }
    woke
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
            Err(e) => return Err(lost(&e)),
        }
    }
}

/// What a wake is told as.
pub fn said(woke: Woke) -> Heard {
    Heard::from(woke)
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
