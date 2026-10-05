//! What the person is about to read, for a body pass to fetch first.
//!
//! A body pass fetches what the store has headers for and no body, newest first, a budget at a
//! time. That is the right order for a mailbox nobody is looking at and the wrong one for the
//! conversation on the screen: scrolled to from last month, it waits behind every newer message
//! in the backlog. So the window says what it is showing ([`ask_first`]) and the engine puts
//! those conversations' messages at the front of the pass.
//!
//! The window also says which message it is fetching for itself as it is opened ([`claim`]), so
//! a pass that runs meanwhile does not fetch the same body a second time.
//!
//! # One per process
//!
//! This is the state of the one window in this process, and it is a process global rather than
//! a value handed down through every pass because nothing else could use it: a `mailo watch` is
//! another process, and its passes fetch in the ordinary order. The lists are small (a screenful
//! of conversations, the one or two messages being opened) and replaced or released whole, so a
//! lock held for the length of a copy is all they need.

use mail_domain::{MessageId, ThreadId};
use std::sync::{Mutex, MutexGuard, PoisonError};

/// How many conversations the window may ask for. More than a screen and its neighbours is a
/// second backlog, which is what the ordinary order is for.
pub const ASKED: usize = 64;

struct Wanted {
    first: Vec<ThreadId>,
    /// One entry per claim, so two claims of one message are released one at a time.
    claimed: Vec<MessageId>,
}

static WANTED: Mutex<Wanted> = Mutex::new(Wanted {
    first: Vec::new(),
    claimed: Vec::new(),
});

fn held() -> MutexGuard<'static, Wanted> {
    // Nothing done under the lock can leave it half-changed: a panic elsewhere while it was held
    // is no reason for every later pass to fail.
    WANTED.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The conversations to fetch bodies for first, most wanted first. Replaces what was asked
/// before; past [`ASKED`] is dropped.
pub fn ask_first(threads: &[ThreadId]) {
    let mut wanted = held();
    wanted.first.clear();
    wanted.first.extend(threads.iter().take(ASKED).copied());
}

/// What [`ask_first`] last asked for.
pub fn first() -> Vec<ThreadId> {
    held().first.clone()
}

/// Being fetched now by the window itself, until the returned [`Claim`] is dropped.
pub fn claim(message: MessageId) -> Claim {
    held().claimed.push(message);
    Claim(message)
}

/// The messages claimed now.
pub fn claimed() -> Vec<MessageId> {
    held().claimed.clone()
}

/// A message being fetched by the window. Dropping it gives the message back to the passes.
#[derive(Debug)]
#[must_use = "the claim ends when this is dropped"]
pub struct Claim(MessageId);

impl Drop for Claim {
    fn drop(&mut self) {
        let mut wanted = held();
        if let Some(at) = wanted.claimed.iter().position(|id| *id == self.0) {
            wanted.claimed.remove(at);
        }
    }
}

/// The order a body pass fetches in: the wanted conversations' messages first, in the order the
/// conversations were asked for, then the rest of the backlog as the store gave it, each address
/// once, and no more than `limit` in all.
///
/// `hinted` is each wanted message beside its conversation, newest first within one, as
/// `Store::unfetched_in_threads` answers; `rest` is `Store::unfetched_in`'s answer. Returns the
/// order and how many at its front came from `hinted`, which the caller keeps in front.
pub fn ahead<R: PartialEq + Clone>(
    asked: &[ThreadId],
    mut hinted: Vec<(ThreadId, R)>,
    rest: Vec<R>,
    limit: usize,
) -> (Vec<R>, usize) {
    // Stable, so the store's newest-first order within a conversation survives.
    hinted.sort_by_key(|(thread, _)| asked.iter().position(|it| it == thread));
    let mut order: Vec<R> = Vec::new();
    for (_, remote) in hinted {
        if order.len() == limit {
            break;
        }
        if !order.contains(&remote) {
            order.push(remote);
        }
    }
    let first = order.len();
    for remote in rest {
        if order.len() == limit {
            break;
        }
        if !order.contains(&remote) {
            order.push(remote);
        }
    }
    (order, first)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn thread(n: u128) -> ThreadId {
        ThreadId::from_uuid(uuid::Uuid::from_u128(n))
    }

    #[test]
    fn the_wanted_come_first_in_the_order_they_were_asked_for() {
        let (a, b) = (thread(1), thread(2));
        let hinted = vec![(a, "a-new"), (b, "b-new"), (a, "a-old")];
        let (order, first) = ahead(&[b, a], hinted, vec!["newest", "a-new", "older"], 10);
        assert_eq!(order, ["b-new", "a-new", "a-old", "newest", "older"]);
        assert_eq!(first, 3);
    }

    #[test]
    fn nothing_asked_is_the_backlog_as_it_was() {
        let (order, first) = ahead::<&str>(&[], vec![], vec!["one", "two"], 10);
        assert_eq!(order, ["one", "two"]);
        assert_eq!(first, 0);
    }

    #[test]
    fn the_limit_counts_the_wanted_and_the_rest_together() {
        let a = thread(1);
        let hinted = vec![(a, "a1"), (a, "a2")];
        assert_eq!(
            ahead(&[a], hinted.clone(), vec!["r1", "r2"], 3).0,
            ["a1", "a2", "r1"]
        );
        assert_eq!(ahead(&[a], hinted, vec!["r1"], 1), (vec!["a1"], 1));
    }

    #[test]
    fn a_claim_lasts_until_it_is_dropped() {
        let message = MessageId::from_uuid(uuid::Uuid::from_u128(0xc1a1));
        let held = claim(message);
        let again = claim(message);
        assert!(claimed().contains(&message));
        drop(held);
        assert!(
            claimed().contains(&message),
            "the second claim still holds it"
        );
        drop(again);
        assert!(!claimed().contains(&message));
    }

    #[test]
    fn asking_again_replaces_and_is_bounded() {
        let many: Vec<ThreadId> = (1..=(ASKED as u128 + 10)).map(thread).collect();
        ask_first(&many);
        assert_eq!(first().len(), ASKED);
        ask_first(&many[..2]);
        assert_eq!(first(), many[..2]);
        ask_first(&[]);
    }
}
