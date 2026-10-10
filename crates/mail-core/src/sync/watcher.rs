//! `mailo watch`: the scheduler the window runs, with its passes announced as they end.
//!
//! The loop is `mail_runtime::schedule`'s, so a watch and a window fetch the same way: a link per
//! account, a pass at each account's interval, a connection held to hear the server push where it
//! can, a backoff after a failure, and no pass at all for an account whose credential the server
//! refused. What this adds is what a watch is for. Each pass that ends is handed to the caller
//! ([`Watched`]), what it fetched is announced ([`told`]), and the run ends once every account
//! has stopped for a reason only a person can clear.

use super::report::{Hooks, PassEnd, Progress, Watched, outcome};
use super::{Announce, configured, live};
use crate::error::CoreError;
use crate::mail::{Mail, SyncOps};
use crate::notify::Notifier;
use chrono::{DateTime, Utc};
use mail_runtime::schedule::{
    Change, Daemon, End, Host, ListenCall, Local, Lost, Note, PassCall, Passed, Schedule,
    Scheduler, Stored,
};
use porter_core::AccountId;
use std::cell::RefCell;
use std::collections::BTreeMap;

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
    let Announce::To { store, notifier } = announce else {
        return;
    };
    if let Err(why) = crate::notify::announce(
        store,
        report.account.clone(),
        &report.counts.arrived,
        notifier,
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

impl SyncOps<'_> {
    /// Keep syncing until every account has stopped for a reason worth stopping for — `mailo
    /// watch`, and `plan.md` phase 8g.
    ///
    /// IDLE is a connection held open for as long as the server allows, so it needs a process
    /// whose job is to stay open; the window is not that when it is closed, so this is.
    ///
    /// `notifier` is where what each pass fetches is announced (`plan.md` 10.6): the desktop's
    /// service in the binary, `None` for a watch that says nothing aloud.
    ///
    /// A watch has no end to return a report at, so each thing it would say is handed to `tell`
    /// as it happens ([`Watched`]) and the caller words it. It returns only once every account
    /// has stopped, with how each last ended.
    pub async fn watch(
        &self,
        notifier: Option<&dyn Notifier>,
        tell: &dyn Fn(Watched),
    ) -> Result<Vec<PassEnd>, CoreError> {
        let mail = self.0;
        // Read once up front: an unreadable registry is a reason not to start, where it would
        // otherwise fail every pass.
        mail.clients()?;
        let announce = match notifier {
            Some(notifier) => Announce::To {
                store: mail.store(),
                notifier,
            },
            None => Announce::Quietly,
        };
        let host = Watching {
            mail,
            announce,
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
        scheduler.run(inbox, End::WhenStuck).await;
        let mut ends = host.ends.into_inner();
        // In the order the accounts are configured.
        Ok(configured(mail.store())?
            .into_iter()
            .filter_map(|account| ends.remove(&account.id))
            .collect())
    }
}

/// What the scheduler is lent by a watch.
struct Watching<'a> {
    mail: &'a Mail,
    announce: Announce<'a>,
    tell: &'a dyn Fn(Watched),
    /// How each account's latest pass ended.
    ends: RefCell<BTreeMap<AccountId, PassEnd>>,
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
                let _ = progress.send(step.event());
            };
            let hooks = Hooks {
                progress: Some(&say),
                cancel: BTreeMap::from([(account.clone(), cancel)]),
            };
            let done = self
                .mail
                .sync()
                .run_due(std::slice::from_ref(&account), hooks)
                .await;
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
