//! `mailo watch`: the scheduler the window runs, with its passes announced as they end.
//!
//! The loop is `mail_engine::schedule`'s, so a watch and a window fetch the same way: a link per
//! account, a pass at each account's interval, a connection held to hear the server push where it
//! can, a backoff after a failure, and no pass at all for an account whose credential the server
//! refused. What this adds is what a watch is for. Each pass that ends is handed to the caller
//! ([`Watched`]), what it fetched is announced ([`told`]), and the run ends once every account
//! has stopped for a reason only a person can clear.

use super::report::{Hooks, PassEnd, Progress, Watched, outcome};
use super::{Announce, configured, live};
use crate::error::CoreError;
use crate::fetch::{Event, Trigger};
use crate::junk::Policy;
use crate::mail::{Mail, SyncOps};
use crate::notify::{Notifier, Whom};
use chrono::{DateTime, Utc};
use mail_engine::schedule::{
    Change, Daemon, End, Host, ListenCall, Local, Lost, Note, PassCall, Passed, Schedule,
    Scheduler, Stored,
};
use mail_store::SqliteStore;
use porter_core::AccountId;
use std::cell::RefCell;
use std::collections::BTreeMap;
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::watch;

/// What a watch's scheduler listens on for a pass someone asked for at the watch's door
/// (`mail_agent::ipc::watching::Watching::asked`): it moves once for each request.
pub type Asked = watch::Receiver<u64>;

/// What a watch says of a pass that has ended, and what it announces of it.
///
/// The pass is told first, so a notification never announces mail the caller has not yet heard
/// was fetched. A failure to announce is told and does not stop the watch: mail that cannot be
/// announced is still mail worth fetching. Only a pass that ran announces; the follow-up
/// reminders are swept after it, because what it fetched may be the reply one was waiting for or
/// the Sent copy a composer's reminder was waiting to join, and one may simply have come due.
/// Only a watch that announces sweeps, so that a reminder coming back is said by whoever brings
/// it back: a quiet watch leaves it to the window.
pub fn told(end: &PassEnd, announce: Announce<'_>, at: DateTime<Utc>, tell: &dyn Fn(Watched)) {
    tell(Watched::Pass(end.clone()));
    let PassEnd::Finished(report) = end else {
        return;
    };
    let Announce::To {
        store,
        notifier,
        whom,
    } = announce
    else {
        return;
    };
    if let Err(why) = crate::notify::announce(
        store,
        report.account.clone(),
        &report.counts.arrived,
        &report.counts.noted,
        notifier,
        whom(),
        at,
    ) {
        tell(Watched::AnnounceFailed {
            address: report.address.clone(),
            why: why.to_string(),
        });
    }
    if let Err(why) = crate::follow_up::sweep_and_announce(store, Some(notifier), at) {
        tell(Watched::RemindersFailed {
            why: why.to_string(),
        });
    }
}

/// What a watch asks again as each pass goes by, so that a setting changed while it runs is
/// followed from the next pass: `whom` is whose new mail it says, asked as each pass ends, and
/// `junk` is how the junk filter behaves, asked as each pass starts.
#[derive(Clone, Copy)]
pub struct Live<'a> {
    pub whom: &'a dyn Fn() -> Whom,
    pub junk: &'a dyn Fn() -> Policy,
}

impl std::fmt::Debug for Live<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Live(..)")
    }
}

impl SyncOps<'_> {
    /// Keep syncing until every account has stopped for a reason worth stopping for: `mailo
    /// watch`.
    ///
    /// IDLE is a connection held open for as long as the server allows, so it needs a process
    /// whose job is to stay open; the window is not that when it is closed, so this is.
    ///
    /// `notifier` is where what each pass fetches is announced: the desktop's
    /// service in the binary, `None` for a watch that says nothing aloud. `whom` is whose new
    /// mail it says, everyone's or the VIPs' alone, asked as each pass ends so that a change of
    /// the setting is followed from the next pass.
    ///
    /// A watch has no end to return a report at, so each thing it would say is handed to `tell`
    /// as it happens ([`Watched`]) and the caller words it. It returns only once every account
    /// has stopped, with how each last ended.
    ///
    /// `junk` is how the junk filter behaves, asked as each pass starts, as `whom` is asked as
    /// each ends: a handle's own setting is the one it was built with, which a watch that runs
    /// for days would keep for as long.
    ///
    /// `asked` is the watch's door (`mail_agent::ipc::watching::Watching::asked`): each time it
    /// moves, every account is asked for a pass now, as the window's Sync asks its own.
    pub async fn watch(
        &self,
        notifier: Option<&dyn Notifier>,
        live: Live<'_>,
        tell: &dyn Fn(Watched),
        asked: Option<Asked>,
    ) -> Result<Vec<PassEnd>, CoreError> {
        self.watch_until(notifier, live, tell, asked, None, End::WhenStuck)
            .await
    }

    /// [`watch`](Self::watch), for a watch that is not a command: the one mailo.app runs in its
    /// own process while it is open (`mail_app::background`).
    ///
    /// `accounts` moves when the accounts may have changed: each time, they are read again, so an
    /// account added in a window of the same process is fetched on the watch's schedule from then
    /// on, and one removed stops being. `mailo watch` reads them once and leaves a new one to the
    /// next watch. `end` is when the run returns: [`End::WhenStuck`] for a command, which has
    /// nothing left to do once every account is refused; [`End::Never`] for the app's, which a
    /// sign-in in its window may bring back to life.
    pub async fn watch_until(
        &self,
        notifier: Option<&dyn Notifier>,
        live: Live<'_>,
        tell: &dyn Fn(Watched),
        asked: Option<Asked>,
        accounts: Option<Asked>,
        end: End,
    ) -> Result<Vec<PassEnd>, CoreError> {
        let Live { whom, junk } = live;
        let mail = self.0;
        // Read once up front: an unreadable registry is a reason not to start, where it would
        // otherwise fail every pass.
        mail.clients()?;
        let announce = match notifier {
            Some(notifier) => Announce::To {
                store: mail.store(),
                notifier,
                whom,
            },
            None => Announce::Quietly,
        };
        let host = Watching {
            mail,
            announce,
            junk,
            tell,
            ends: RefCell::new(BTreeMap::new()),
        };
        let (tx, inbox) = tokio::sync::mpsc::unbounded_channel();
        let scheduler = Scheduler::new(
            &host,
            mail.store().clone(),
            BTreeMap::new(),
            BTreeMap::new(),
            tx.clone(),
        );
        // The accounts are read once, as they always were: one made since is the window's to
        // tell the next watch about.
        let _ = tx.send(Note::Accounts(super::due::intervals(mail.store())));
        tokio::select! {
            _ = scheduler.run(inbox, end) => {}
            () = answer(asked, mail.store(), tx.clone()) => {}
            () = follow(accounts, mail.store(), tx) => {}
        }
        let mut ends = host.ends.into_inner();
        // In the order the accounts are configured.
        Ok(configured(mail.store())?
            .into_iter()
            .filter_map(|account| ends.remove(&account.id))
            .collect())
    }
}

/// Ask every account for a pass each time `asked` moves, as a person's Sync does: a pass starts
/// at once on a link at rest (a parked IDLE holds a connection of its own and is not in the
/// way), and a link already in a pass is in the middle of what was asked for. Never returns: the
/// scheduler's end is the watch's.
pub async fn answer(asked: Option<Asked>, store: &SqliteStore, tx: UnboundedSender<Note>) {
    if let Some(mut asked) = asked {
        while asked.changed().await.is_ok() {
            for (account, _) in super::due::intervals(store) {
                // The send fails only once the scheduler has stopped, and then nobody is left to ask.
                let _ = tx.send(Note::Event(account, Event::Start(Trigger::Manual)));
            }
        }
    }
    std::future::pending::<()>().await;
}

/// Tell the scheduler the accounts as they are now each time `accounts` moves. Never returns, as
/// [`answer`] does not.
pub async fn follow(accounts: Option<Asked>, store: &SqliteStore, tx: UnboundedSender<Note>) {
    if let Some(mut accounts) = accounts {
        while accounts.changed().await.is_ok() {
            // The send fails only once the scheduler has stopped, and then nobody is left to tell.
            let _ = tx.send(Note::Accounts(super::due::intervals(store)));
        }
    }
    std::future::pending::<()>().await;
}

/// Why a pass did not end in an answer.
#[derive(Debug, thiserror::Error)]
enum Stopped {
    #[error("{0}")]
    Failed(CoreError),
    #[error("the sync pass stopped: {0}")]
    Panicked(String),
}

/// `pass`, with a panic in it turned into a failed pass. The scheduler runs on the watch's own
/// task, so a panic left to unwind would end the watch; a failed pass is one it backs off from
/// and tries again, as the window's does ([`outcome`]).
async fn survive<T>(
    pass: impl std::future::Future<Output = Result<T, CoreError>>,
) -> Result<T, Stopped> {
    use futures_util::FutureExt;
    match std::panic::AssertUnwindSafe(pass).catch_unwind().await {
        Ok(done) => done.map_err(Stopped::Failed),
        Err(panicked) => {
            let what = panicked
                .downcast_ref::<&str>()
                .map(|text| (*text).to_owned())
                .or_else(|| panicked.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "a pass panicked".to_owned());
            log::error!("a sync pass panicked: {what}");
            Err(Stopped::Panicked(what))
        }
    }
}

/// What the scheduler is lent by a watch.
struct Watching<'a> {
    mail: &'a Mail,
    announce: Announce<'a>,
    /// How the junk filter behaves, asked as each pass starts.
    junk: &'a dyn Fn() -> Policy,
    tell: &'a dyn Fn(Watched),
    /// How each account's latest pass ended.
    ends: RefCell<BTreeMap<AccountId, PassEnd>>,
}

impl Watching<'_> {
    /// The handle a pass runs over: the watch's, with the junk filter as it is set now.
    fn mail_now(&self) -> Mail {
        self.mail.clone().with_junk((self.junk)())
    }
}

impl Host for Watching<'_> {
    fn now(&self) -> DateTime<Utc> {
        self.mail.now()
    }

    /// A watch is the one that schedules.
    fn schedule(&self) -> Schedule {
        Schedule::Window
    }

    /// A watch holds the account's connection itself: it is the one that stays open.
    fn daemon(&self) -> Daemon {
        Daemon::Absent
    }

    fn pushers(&self) -> Vec<AccountId> {
        live::pushing(self.mail.store())
    }

    fn folder_in_flight(&self, _: &AccountId) -> bool {
        false
    }

    fn pass(&self, call: PassCall) -> Local<'_, Passed> {
        Box::pin(async move {
            let PassCall {
                account,
                started,
                cancel,
                progress,
            } = call;
            let say = |_: AccountId, step: Progress| {
                // Nobody listening for progress any more is not the pass's concern.
                let _ = progress.send(step.event());
            };
            let hooks = Hooks {
                progress: Some(&say),
                cancel: BTreeMap::from([(account.clone(), cancel)]),
            };
            let mail = self.mail_now();
            let done = survive(mail.sync().run_due(std::slice::from_ref(&account), hooks)).await;
            let mut stored = Stored::Nothing;
            match &done {
                Ok(ends) => {
                    for end in ends {
                        told(end, self.announce, started, self.tell);
                        if end.may_have_stored() {
                            stored = Stored::Maybe;
                        }
                        self.ends.borrow_mut().insert(end.account(), end.clone());
                    }
                }
                Err(why) => (self.tell)(Watched::RunFailed {
                    why: why.to_string(),
                }),
            }
            Passed {
                event: outcome(done, account),
                stored,
            }
        })
    }

    fn listen(&self, call: ListenCall) -> Local<'_, Result<(), Lost>> {
        Box::pin(async move {
            let ListenCall {
                account,
                stop,
                hold,
                heard,
            } = call;
            self.mail
                .sync()
                .listen(account, stop, Some(hold), live::GRACE, &*heard)
                .await
        })
    }

    fn changed(&self, _: Change) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::support_tests::{account, caps};

    use mail_domain::*;

    /// A store with one IMAP account, which is what a pass can be asked of.
    fn store() -> SqliteStore {
        let store = mail_store::testing::in_memory();
        let plan = AccountPlan {
            address: "ada@example.test".to_owned(),
            incoming: Incoming::Imap {
                host: "imap.example.test".to_owned(),
                port: 993,
                tls: Tls::Implicit,
            },
            outgoing: Outgoing::Nowhere,
            auth: AuthPlan::Password {
                username: Username::SameAsAddress,
                sasl: vec![SaslMech::Plain],
            },
            identities: Vec::new(),
        };
        let caps = AccountCaps {
            watch: WatchMode::Idle,
            ..caps()
        };
        mail_store::testing::seed_account_plan(&store, account(), "ada@example.test", &plan, None);
        mail_store::testing::seed_caps(&store, account(), &caps, caps.observed_at).unwrap();
        store
    }

    /// A pass that panics is a failed pass the scheduler backs off from, not the end of the watch.
    #[tokio::test]
    async fn a_pass_that_panics_is_a_failed_pass() {
        let failed = survive::<Vec<PassEnd>>(async { panic!("a bad message") }).await;
        assert!(matches!(&failed, Err(Stopped::Panicked(what)) if what == "a bad message"));
        assert!(matches!(outcome(failed, account()), Event::Failed { .. }));
        // The next one runs as any other.
        assert!(matches!(
            survive(async { Ok::<_, CoreError>(7) }).await,
            Ok(7)
        ));
    }

    /// A watch that runs for days follows the junk filter's setting from its next pass.
    #[test]
    fn each_pass_runs_over_the_junk_policy_as_it_is_set_then() {
        use crate::junk::Filtering;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let mail = Mail::new(
            std::sync::Arc::new(mail_store::testing::in_memory()),
            runtime.handle().clone(),
        );
        let now = std::cell::Cell::new(Policy::default());
        let junk = || now.get();
        let host = Watching {
            mail: &mail,
            announce: Announce::Quietly,
            junk: &junk,
            tell: &|_| {},
            ends: RefCell::new(BTreeMap::new()),
        };
        assert_eq!(host.mail_now().junk().filtering, Filtering::Off);
        now.set(Policy {
            filtering: Filtering::On,
            ..Policy::default()
        });
        assert_eq!(host.mail_now().junk().filtering, Filtering::On);
        assert_eq!(
            mail.junk().filtering,
            Filtering::Off,
            "the handle itself is unchanged"
        );
    }

    /// A Sync asked for at the watch's door was refused, so a window that left fetching to the
    /// watch had a Sync that did nothing. Now each ask is a manual pass of every account, on
    /// the watch's own scheduler.
    #[tokio::test]
    async fn an_ask_at_the_door_starts_a_pass_of_every_account() {
        let store = store();
        let (asks, asked) = tokio::sync::watch::channel(0u64);
        let (tx, mut notes) = tokio::sync::mpsc::unbounded_channel();
        let answering = answer(Some(asked), &store, tx);
        tokio::pin!(answering);

        // Nothing is started before anyone asks.
        tokio::select! {
            biased;
            () = &mut answering => panic!("the answer ended"),
            () = tokio::task::yield_now() => {}
        }
        assert!(notes.try_recv().is_err(), "a pass nobody asked for");

        asks.send_modify(|n| *n += 1);
        let note = tokio::select! {
            () = &mut answering => panic!("the answer ended"),
            note = notes.recv() => note,
        };
        match note {
            Some(Note::Event(started, Event::Start(Trigger::Manual))) => {
                assert_eq!(started, account());
            }
            other => panic!("not a manual pass: {other:?}"),
        }
    }
}
