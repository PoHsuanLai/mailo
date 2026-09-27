//! One revision for every window of the app.
//!
//! A window re-runs its queries when its own `revision` signal moves (`App`, `list_query`). A
//! `Signal` belongs to one VirtualDom, so a message opened in a window of its own (`ui/window`)
//! cannot read the main window's, and an archive in either would leave the other drawing what
//! the store no longer says.
//!
//! [`Revisions`] is the counter every window gets as a root context (quire hands each window
//! the same `AppConfig` contexts). Each window publishes its own moves to it and follows
//! everyone else's: a move it hears from another window moves its own `revision` by one, which
//! re-runs its queries exactly as a move of its own does. [`Bridge`] is that bookkeeping, pure,
//! so a window never re-publishes what it only followed (two windows would otherwise wake each
//! other forever).

use dioxus::prelude::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::watch;

/// The app's shared revision, as a root context of every window. Cloning shares it.
#[derive(Clone)]
pub struct Revisions(Arc<Shared>);

struct Shared {
    stamp: watch::Sender<Stamp>,
    /// Hands each window that joins a name of its own.
    windows: AtomicU64,
}

impl std::fmt::Debug for Revisions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Revisions")
            .field(&*self.0.stamp.borrow())
            .finish()
    }
}

impl Default for Revisions {
    fn default() -> Self {
        Revisions::new()
    }
}

/// Which window a move came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct Origin(u64);

/// The newest move: how many there have been, and whose it was.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Stamp {
    pub generation: u64,
    pub by: Option<Origin>,
}

impl Revisions {
    /// No move yet.
    pub fn new() -> Self {
        let (stamp, _) = watch::channel(Stamp {
            generation: 0,
            by: None,
        });
        Revisions(Arc::new(Shared {
            stamp,
            windows: AtomicU64::new(0),
        }))
    }

    /// A window joins, its own `revision` at `local`: its bridge, and what it hears moves on.
    pub(crate) fn join(&self, local: u64) -> (Bridge, watch::Receiver<Stamp>) {
        let me = Origin(self.0.windows.fetch_add(1, Ordering::Relaxed));
        let heard = self.0.stamp.subscribe();
        let seen = heard.borrow().generation;
        (Bridge::new(me, seen, local), heard)
    }

    /// Tell every other window that `bridge`'s window moved.
    pub(crate) fn publish(&self, bridge: &mut Bridge) {
        self.0.stamp.send_modify(|stamp| bridge.publish(stamp));
    }

    /// How many moves there have been, from any window.
    pub fn generation(&self) -> u64 {
        self.0.stamp.borrow().generation
    }
}

/// One window's side of the shared revision: what it has published, heard and followed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Bridge {
    me: Origin,
    /// The newest generation this window has accounted for.
    seen: u64,
    /// The last value of the window's own `revision` that is already accounted for: published,
    /// or moved by following another window.
    settled: u64,
}

impl Bridge {
    fn new(me: Origin, seen: u64, local: u64) -> Self {
        Bridge {
            me,
            seen,
            settled: local,
        }
    }

    /// The window's own `revision` is now `local`: whether that is a move of its own, to
    /// publish. A value it reached by following is not.
    pub(crate) fn moved(&mut self, local: u64) -> bool {
        let moved = local != self.settled;
        self.settled = local;
        moved
    }

    /// `stamp` is the newest move: whether the window must follow it. Its own move comes back
    /// too and is not followed; a move already accounted for is not followed twice.
    pub(crate) fn heard(&mut self, stamp: Stamp) -> bool {
        if stamp.generation == self.seen {
            return false;
        }
        self.seen = stamp.generation;
        stamp.by != Some(self.me)
    }

    /// The window followed a move by setting its own `revision` to `local`.
    pub(crate) fn followed(&mut self, local: u64) {
        self.settled = local;
    }

    /// Write this window's move into `stamp`. A window that moves has re-run its queries
    /// against the store as it is now, which includes every move published before this one, so
    /// those count as heard.
    fn publish(&mut self, stamp: &mut Stamp) {
        stamp.generation = stamp.generation.wrapping_add(1);
        stamp.by = Some(self.me);
        self.seen = stamp.generation;
    }
}

/// Tie this window's `revision` to the app's [`Revisions`], when the window was given one: its
/// own moves are published, and every other window's move moves it by one. A window with none (a
/// test's single window) is left as it was.
pub(in crate::ui) fn use_shared_revision(mut revision: Signal<u64>) {
    let joined = use_hook(|| {
        try_consume_context::<Revisions>().map(|shared| {
            let (bridge, heard) = shared.join(*revision.peek());
            (shared, CopyValue::new(bridge), heard)
        })
    });
    let publishing = joined.clone();
    use_effect(move || {
        let local = revision();
        if let Some((shared, mut bridge, _)) = publishing.clone()
            && bridge.write().moved(local)
        {
            shared.publish(&mut bridge.write());
        }
    });
    use_future(move || {
        let joined = joined.clone();
        async move {
            let Some((shared, mut bridge, mut heard)) = joined else {
                return;
            };
            while heard.changed().await.is_ok() {
                let stamp = *heard.borrow_and_update();
                if !bridge.write().heard(stamp) {
                    continue;
                }
                // A move of this window's own that its effect has not published yet goes first,
                // so following cannot swallow it.
                let local = *revision.peek();
                if bridge.write().moved(local) {
                    shared.publish(&mut bridge.write());
                }
                let next = local.wrapping_add(1);
                bridge.write().followed(next);
                revision.set(next);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_move_in_one_window_is_followed_by_the_other_and_not_sent_back() {
        let shared = Revisions::new();
        let (mut main, mut main_hears) = shared.join(0);
        let (mut message, mut message_hears) = shared.join(0);

        // The main window archives: its revision moves to 1, and it publishes.
        assert!(main.moved(1), "a move of the window's own is published");
        shared.publish(&mut main);

        // The message window hears it and follows, moving its own revision to 1.
        assert!(message_hears.has_changed().unwrap());
        let stamp = *message_hears.borrow_and_update();
        assert!(
            message.heard(stamp),
            "the other window's move was not followed"
        );
        message.followed(1);
        // Its effect then sees revision 1, which it reached by following: nothing to publish.
        assert!(
            !message.moved(1),
            "a followed move was published back, and the windows would wake each other forever"
        );

        // The main window hears its own move come back, and does not follow it.
        let stamp = *main_hears.borrow_and_update();
        assert!(!main.heard(stamp), "a window followed its own move");
        assert_eq!(shared.generation(), 1, "one gesture, one move");
    }

    #[test]
    fn each_window_s_moves_reach_every_other_window() {
        let shared = Revisions::new();
        let (mut a, mut a_hears) = shared.join(0);
        let (mut b, mut b_hears) = shared.join(5);
        let (mut c, mut c_hears) = shared.join(0);

        assert!(b.moved(6));
        shared.publish(&mut b);
        let stamp = *a_hears.borrow_and_update();
        assert!(a.heard(stamp));
        let stamp = *c_hears.borrow_and_update();
        assert!(c.heard(stamp));
        let stamp = *b_hears.borrow_and_update();
        assert!(!b.heard(stamp));
    }

    #[test]
    fn a_stamp_already_accounted_for_is_not_followed_twice() {
        let shared = Revisions::new();
        let (mut a, _) = shared.join(0);
        let (mut b, mut b_hears) = shared.join(0);
        assert!(a.moved(1));
        shared.publish(&mut a);
        let stamp = *b_hears.borrow_and_update();
        assert!(b.heard(stamp));
        assert!(!b.heard(stamp), "the same move was followed twice");
    }

    #[test]
    fn a_window_that_moves_has_heard_everything_before_its_move() {
        // B moves, then A moves before A's follower has run: A's queries re-ran after B's write,
        // so A does not follow B's move as well, and B follows A's.
        let shared = Revisions::new();
        let (mut a, mut a_hears) = shared.join(0);
        let (mut b, mut b_hears) = shared.join(0);
        assert!(b.moved(1));
        shared.publish(&mut b);
        assert!(a.moved(1));
        shared.publish(&mut a);

        let stamp = *a_hears.borrow_and_update();
        assert!(!a.heard(stamp));
        let stamp = *b_hears.borrow_and_update();
        assert!(b.heard(stamp), "B missed A's move");
        assert_eq!(shared.generation(), 2);
    }

    #[test]
    fn a_window_that_joins_late_does_not_follow_what_came_before_it() {
        let shared = Revisions::new();
        let (mut a, _) = shared.join(0);
        assert!(a.moved(1));
        shared.publish(&mut a);
        let (mut late, late_hears) = shared.join(0);
        let stamp = *late_hears.borrow();
        assert!(
            !late.heard(stamp),
            "a new window refreshed for a move it never missed"
        );
    }
}
