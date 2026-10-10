//! What the scheduler has in flight, and which of it still counts.
//!
//! Everything the scheduler starts (a pass, a timer, a live watch) is a future it polls itself,
//! on the task that runs it, so none of it needs to be `Send` or `'static`: a front end may lend
//! it a borrowed notifier, and the window lends it signals. The set is a plain list because
//! there are a few futures per account, not thousands.

use crate::fetch::Link;
use porter_core::AccountId;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::Poll;

/// A future that is not required to be `Send`: what a [`Host`](super::Host) hands back.
pub type Local<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;

/// One thing in flight. It reports through the scheduler's channel, not by returning.
pub(super) type Task<'a> = Local<'a, ()>;

/// The futures in flight.
#[derive(Default)]
pub(super) struct Tasks<'a>(Vec<Task<'a>>);

impl<'a> Tasks<'a> {
    /// Take on everything `fresh` holds.
    pub fn absorb(&mut self, fresh: &mut Vec<Task<'a>>) {
        self.0.append(fresh);
    }

    /// Wait until one of them has finished, and drop it. Pending for ever when there are none,
    /// which is for the caller's other branch to end.
    pub async fn finished(&mut self) {
        std::future::poll_fn(|cx| {
            let mut index = 0;
            while index < self.0.len() {
                if self.0[index].as_mut().poll(cx).is_ready() {
                    self.0.swap_remove(index);
                    return Poll::Ready(());
                }
                index += 1;
            }
            Poll::Pending
        })
        .await
    }
}

/// Which pass, or wait, of an account is the current one. A pass that was cancelled, or a wait
/// that a later one replaced, finds its number out of date and says nothing.
#[derive(Clone, Default)]
pub(super) struct Generations(Rc<RefCell<BTreeMap<AccountId, u64>>>);

impl Generations {
    /// A new one begins: every earlier one is out of date.
    pub fn next(&self, account: &AccountId) -> u64 {
        let mut all = self.0.borrow_mut();
        let slot = all.entry(account.clone()).or_insert(0);
        *slot += 1;
        *slot
    }

    /// Whether `generation` is still the account's latest.
    pub fn current(&self, account: &AccountId, generation: u64) -> bool {
        self.0.borrow().get(account) == Some(&generation)
    }

    /// The account is gone.
    pub fn forget(&self, account: &AccountId) {
        self.0.borrow_mut().remove(account);
    }
}

/// Whether `link` is at a stop that only a person can clear: a refused credential, a grant gone,
/// or a failure no retry changes. Not while a pass is running over it.
pub(super) fn stuck(link: &Link) -> bool {
    !link.is_busy()
        && matches!(
            link,
            Link::NeedsSignIn { .. } | Link::NeedsAllow { .. } | Link::Broken { .. }
        )
}
