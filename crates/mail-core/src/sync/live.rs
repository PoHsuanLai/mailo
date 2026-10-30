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

use super::access::{Access, Wait, access};
use super::report::Failure;
use super::run::Run;
use super::{Configured, configured};
use crate::error::Logged;
use mail_domain::*;
use mail_runtime::graph::push::SUBSCRIPTIONS;
use mail_runtime::{Cancel, RuntimeError, Woke};
use mail_secrets::{AccountSecrets, ClientRegistry};
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
pub(super) fn lost(e: &RuntimeError) -> Lost {
    Failure::of("", e).into()
}

/// Why this account cannot be waited on at all.
pub(super) fn unsupported(why: &str) -> Lost {
    Failure::fatal(why).into()
}

pub use mail_runtime::schedule::Hold;

/// Whether the account's stored plan and capabilities can be waited on, and the person left it to
/// be: see the table above.
///
/// A Graph account is not here: whether it is pushed to is a setting the store holds, which
/// [`pushing_with`] reads ([`super::graph_push::relayed`]).
pub fn pushes(account: &Configured) -> bool {
    account.plan.incoming.caps().pushes
        && matches!(account.caps.watch, WatchMode::Idle)
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
    watch: Watch,
    heard: &dyn Fn(Heard),
) -> Result<(), Lost> {
    listen_at(
        store,
        secrets,
        registry,
        SUBSCRIPTIONS,
        account,
        watch,
        heard,
    )
    .await
}

/// [`listen_with`], with Graph's subscriptions at `subscriptions` rather than Microsoft's: a
/// test's own listener.
pub async fn listen_at(
    store: Arc<SqliteStore>,
    secrets: Arc<dyn AccountSecrets>,
    registry: &ClientRegistry,
    subscriptions: &str,
    account: AccountId,
    watch: Watch,
    heard: &dyn Fn(Heard),
) -> Result<(), Lost> {
    let account = configured(&store)
        .map_err(|why| unsupported(&why.to_string()))?
        .into_iter()
        .find(|one| one.id == account)
        .ok_or_else(|| unsupported("that account is no longer configured"))?;
    let run = Run {
        store: &store,
        secrets,
        registry,
        // The wall clock, because a watch outlives any `now` it could be handed.
        now: chrono::Utc::now(),
        account: &account,
    };
    listen_on(
        access(&account.plan.incoming),
        &run,
        subscriptions,
        watch,
        heard,
    )
    .await
}

/// [`listen_at`], on the protocol it is given: wait on `run`'s account the way `access` does,
/// and say what is heard. Split out so a test can wait with something that is not a server.
pub(super) async fn listen_on(
    access: &dyn Access,
    run: &Run<'_>,
    subscriptions: &str,
    Watch {
        mut cancel,
        hold,
        grace,
    }: Watch,
    heard: &dyn Fn(Heard),
) -> Result<(), Lost> {
    let mut waiting = access.wait(run, subscriptions).await?;
    hearing(&mut *waiting, &mut cancel, hold, grace, heard).await
}

/// Wait, say what was heard, and wait again, until cancelled or lost.
async fn hearing(
    waiter: &mut dyn Wait,
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
    use super::super::access::fake::{Fake, Step};
    use super::*;
    use crate::support_tests::{account, caps};
    use std::sync::Mutex;

    #[test]
    fn a_wake_is_told_as_what_it_was() {
        assert_eq!(said(Woke::Mail), Heard::Mail);
        assert_eq!(said(Woke::Due), Heard::Due);
        assert_eq!(said(Woke::Interval), Heard::Interval);
    }

    /// An account whose protocol is whatever the test hands in, so nothing is connected to.
    fn configured_account() -> (Arc<SqliteStore>, Configured) {
        let store = Arc::new(mail_store::testing::in_memory());
        let plan = AccountPlan {
            address: "ada@example.test".to_owned(),
            incoming: Incoming::Local,
            outgoing: Outgoing::Nowhere,
            auth: AuthPlan::Password {
                username: Username::SameAsAddress,
                sasl: vec![SaslMech::Plain],
            },
            identities: Vec::new(),
        };
        let caps = caps();
        mail_store::testing::seed_account_plan(&store, account(), "ada@example.test", &plan, None);
        mail_store::testing::seed_caps(&store, account(), &caps, caps.observed_at).unwrap();
        let one = configured(&store).unwrap().remove(0);
        (store, one)
    }

    /// Listen on `access` for `lasting`, then cancel; returns how it ended and what it heard.
    async fn listening(
        access: &dyn Access,
        grace: Duration,
        lasting: Duration,
    ) -> (Result<(), Lost>, Vec<Heard>) {
        let (store, account) = configured_account();
        let run = Run {
            store: &store,
            secrets: Arc::new(porter_secrets::MemorySecrets::default()),
            registry: &ClientRegistry::default(),
            now: chrono::Utc::now(),
            account: &account,
        };
        let (stop, cancel) = watch::channel(false);
        let seen = Mutex::new(Vec::new());
        let heard = |h| seen.lock().unwrap().push(h);
        let hearing = listen_on(
            access,
            &run,
            "",
            Watch {
                cancel,
                hold: None,
                grace,
            },
            &heard,
        );
        let cancelling = async {
            tokio::time::sleep(lasting).await;
            let _ = stop.send(true);
        };
        let (ended, ()) = tokio::join!(hearing, cancelling);
        (ended, seen.into_inner().unwrap())
    }

    #[tokio::test]
    async fn a_server_announcing_mail_is_heard_once_the_wait_is_held() {
        let fake = Fake::waiting([Step::After(Duration::from_millis(60), Woke::Mail)]);
        let (ended, heard) =
            listening(&fake, Duration::from_millis(20), Duration::from_millis(300)).await;
        assert_eq!(heard, [Heard::Established, Heard::Mail], "held, then told");
        assert_eq!(ended, Ok(()), "cancelling is not a loss");
    }

    #[tokio::test]
    async fn a_quiet_wait_is_held_and_released_on_cancel() {
        let fake = Fake::waiting([]);
        let (ended, heard) =
            listening(&fake, Duration::from_millis(20), Duration::from_millis(200)).await;
        assert_eq!(heard, [Heard::Established], "nothing said");
        assert_eq!(ended, Ok(()));
    }

    /// A protocol that overrides nothing it cannot do is refused before anything is waited on.
    #[tokio::test]
    async fn a_protocol_that_cannot_be_waited_on_is_a_loss_that_is_never_established() {
        struct Deaf;
        impl Access for Deaf {
            fn pass<'a>(
                &'a self,
                _: &'a Run<'a>,
                _: super::super::access::Pass<'a>,
            ) -> super::super::access::Answer<'a, super::super::report::Done> {
                Box::pin(std::future::ready(Ok(Default::default())))
            }
        }
        let (ended, heard) =
            listening(&Deaf, Duration::from_millis(20), Duration::from_millis(100)).await;
        let lost = ended.expect_err("nothing to wait on");
        assert!(matches!(lost.retry, Retry::Fatal(_)), "{lost:?}");
        assert!(lost.why.contains("cannot be waited on"), "{}", lost.why);
        assert!(heard.is_empty());
    }
}
