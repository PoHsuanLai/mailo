//! What the window lends the scheduler: its passes, its watch, its clock, and the signals a
//! change is written into.
//!
//! The scheduler (`mail_core::schedule`) owns every account's link and when it moves; this is
//! the window's side of that. A pass and a live watch are the seams the servers hide behind
//! ([`Passer`], [`Listener`]), so a test provides its own, and a change the scheduler reports is
//! written into the signals the surfaces read.

use super::live::Listener;
use super::pass::may_have_stored;
use super::{Delegate, Passer};
use crate::ui::clock::WallClock;
use chrono::{DateTime, Utc};
use dioxus::prelude::*;
use ds::motion::detail::operation::{Operation, PendingToken};
use mail_core::fetch::{FolderFetch, Link};
use mail_core::schedule::{
    Change, Daemon, Heard, Host, ListenCall, Local, Lost, PassCall, Passed, Schedule, Stored,
    Timing,
};
use mail_core::sync::report::{Hooks, Progress, outcome};
use mail_domain::Retry;
use mail_store::SqliteStore;
use porter_core::AccountId;
use std::collections::BTreeMap;
use std::sync::Arc;

/// The window, as the scheduler's host.
pub(super) struct WindowHost {
    pub store: Arc<SqliteStore>,
    pub passer: Passer,
    pub listener: Listener,
    pub delegate: Delegate,
    /// The window's own clock, when it has one (a test's follows quire's).
    pub clock: Option<WallClock>,
    pub links: Signal<BTreeMap<AccountId, Link>>,
    /// What each account's pass is doing, for a spinner: read by the surfaces of the next wave.
    pub ops: Signal<BTreeMap<AccountId, Operation>>,
    pub folders: Signal<BTreeMap<(AccountId, String), FolderFetch>>,
    pub revision: Signal<u64>,
}

/// The operation to show once `link` is what it is: running only while a pass is, and under the
/// token it started with.
pub(super) fn operation(before: Operation, link: &Link) -> Operation {
    match (link.is_busy(), before) {
        (false, _) => Operation::Idle,
        (true, running @ Operation::Running(_)) => running,
        (true, Operation::Idle) => Operation::Running(PendingToken::start()),
    }
}

impl Host for WindowHost {
    fn now(&self) -> DateTime<Utc> {
        self.clock.as_ref().map_or_else(Utc::now, WallClock::now)
    }

    fn schedule(&self) -> Schedule {
        self.delegate.schedule()
    }

    fn daemon(&self) -> Daemon {
        (self.listener.daemon)()
    }

    fn pushers(&self) -> Vec<AccountId> {
        (self.listener.pushers)(&self.store)
    }

    fn folder_in_flight(&self, account: &AccountId) -> bool {
        self.folders
            .peek()
            .iter()
            .any(|((one, _), fetch)| one == account && *fetch == FolderFetch::Fetching)
    }

    fn timing(&self) -> Timing {
        Timing {
            first: self.listener.first,
            retry: self.listener.retry,
            ..Timing::default()
        }
    }

    fn pass(&self, call: PassCall) -> Local<'_, Passed> {
        let (store, passer) = (self.store.clone(), self.passer.clone());
        Box::pin(async move {
            let PassCall {
                account,
                started,
                cancel,
                progress,
            } = call;
            let this_account = account.clone();
            // `spawn_blocking`, not this task: a pass waits on the application's runtime
            // (`edge::block_on`), and `Runtime::block_on` inside an async context panics.
            let done = tokio::task::spawn_blocking(move || {
                let said = |_: AccountId, step: Progress| {
                    let _ = progress.send(step.event());
                };
                let hooks = Hooks {
                    progress: Some(&said),
                    cancel: BTreeMap::from([(this_account.clone(), cancel)]),
                };
                (passer.0)(store, started, this_account, hooks)
            })
            .await
            .unwrap_or_else(|e| Err(format!("the sync pass stopped: {e}")));
            let stored = if may_have_stored(&done) {
                Stored::Maybe
            } else {
                Stored::Nothing
            };
            Passed {
                event: outcome(done, account),
                stored,
            }
        })
    }

    fn listen(&self, call: ListenCall) -> Local<'_, Result<(), Lost>> {
        let (listen, store) = (self.listener.listen.clone(), self.store.clone());
        Box::pin(async move {
            let ListenCall {
                account,
                stop,
                hold,
                heard,
            } = call;
            // A thread of its own: the watch builds a runtime, and parks in a socket.
            let ended = tokio::task::spawn_blocking(move || {
                let say = |said: Heard| heard(said);
                listen(store, account, stop, hold, &say)
            })
            .await;
            ended.unwrap_or_else(|e| {
                Err(Lost {
                    retry: Retry::Fatal(e.to_string()),
                    why: format!("the watch stopped: {e}"),
                })
            })
        })
    }

    fn changed(&self, change: Change) {
        match change {
            Change::Link(account, link) => {
                let before = self.ops.peek().get(&account).copied().unwrap_or_default();
                let op = operation(before, &link);
                let mut links = self.links;
                links.write().insert(account.clone(), link);
                if op != before {
                    let mut ops = self.ops;
                    ops.write().insert(account, op);
                }
            }
            Change::Gone(account) => {
                let (mut links, mut ops, mut folders) = (self.links, self.ops, self.folders);
                links.write().remove(&account);
                ops.write().remove(&account);
                folders.write().retain(|(one, _), _| *one != account);
            }
            // What a pass stored is what the list reads, cancelled or not. A pass that stored
            // nothing leaves nothing to read again: a refused or unreachable account must not
            // redraw the window.
            Change::Stored => {
                let mut revision = self.revision;
                revision += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use mail_core::fetch::{First, Live, Pause, Step};

    fn at(second: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 1, 12, 0, 0).unwrap() + chrono::TimeDelta::seconds(second)
    }

    fn current() -> Link {
        Link::Current {
            at: at(0),
            trouble: vec![],
            live: Live::Polling,
        }
    }

    #[test]
    fn the_spinner_runs_only_while_a_pass_does() {
        let token = Operation::Running(PendingToken::start());
        let syncing = Link::Syncing {
            first: First::No,
            step: Step::Flags,
            count: None,
            after: Box::new(current()),
        };
        let resting = [
            current(),
            Link::Fresh,
            Link::Waiting {
                until: at(5),
                why: Pause::Unreachable,
                failures: 1,
                first: First::No,
            },
            Link::NeedsSignIn {
                why: "x".into(),
                first: First::No,
            },
            Link::Broken {
                why: "x".into(),
                first: First::No,
            },
        ];
        assert_eq!(operation(token, &syncing), token, "it keeps its own token");
        assert!(
            matches!(operation(Operation::Idle, &syncing), Operation::Running(_)),
            "a pass that begins shows as running"
        );
        for link in resting {
            assert_eq!(operation(token, &link), Operation::Idle, "{link:?}");
        }
        assert_eq!(operation(Operation::Idle, &current()), Operation::Idle);
    }

    #[test]
    fn mail_with_no_stamp_is_a_fetched_account_current_as_of_now() {
        let (store, _dir) = crate::ui::fixtures::seeded();
        let link = mail_core::schedule::probe(&store, crate::ui::fixtures::acct_account(), at(0));
        assert_eq!(
            link,
            Link::Current {
                at: at(0),
                trouble: vec![],
                live: Live::Polling
            }
        );
    }
}
