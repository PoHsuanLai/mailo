//! What the reader does with the rendered bodies of `mail_core::message`: the cache it owns, the
//! renderings sent off the thread that draws, and the warming that keeps the top of the list
//! rendered before anyone opens it.
//!
//! The cache is one [`Frames`] for the whole app, a root context (`ui::launch::contexts`); a
//! reader drawn with none around it (a test's) gets one of its own. Opened OpenPGP and S/MIME
//! bodies are `ui/pgp`'s, handed to it as the one thing it asks of them.

use std::sync::Arc;

use dioxus::prelude::*;
use mail_core::SqliteStore;
use mail_core::message::{FIRST_SCREEN, Frames, Key, Sent, ahead};
use mail_domain::{Message, ThreadId, ThreadSummary};
use mail_mime::SanitizePolicy;

use crate::ui::view::Shell;

/// A new cache of rendered bodies, asking `ui/pgp` which messages it has opened.
pub(in crate::ui) fn frames() -> Arc<Frames> {
    Arc::new(Frames::new(Arc::new(|message| {
        crate::ui::pgp::parsed(message)
    })))
}

/// The app's cache of rendered bodies: the root context, or a cache of this component's own where
/// there is none.
pub(in crate::ui) fn use_frames() -> Arc<Frames> {
    use_hook(|| try_consume_context::<Arc<Frames>>().unwrap_or_else(frames))
}

/// Render `later` on a blocking thread, then move `landed` so the reader draws them.
pub(super) fn render_later(
    frames: Arc<Frames>,
    store: Arc<SqliteStore>,
    later: Vec<(Message, SanitizePolicy)>,
    sent: std::rc::Rc<std::cell::RefCell<Sent>>,
    mut landed: Signal<u64>,
) {
    let keys: Vec<Key> = later
        .iter()
        .filter_map(|(message, policy)| frames.keyed(message, *policy))
        .collect();
    sent.borrow_mut().start(&keys);
    spawn(async move {
        let _ = tokio::task::spawn_blocking(move || {
            for (message, policy) in &later {
                let _ = frames.rendered(&store, message, *policy);
            }
        })
        .await;
        sent.borrow_mut().finish(keys);
        landed += 1;
    });
}

/// Keep what the person is about to open rendered: the top of the list once it is drawn, and
/// the neighbours of the open conversation whenever it changes. Their bodies, where they are not
/// here yet, are what the next body pass fetches first (`mail_runtime::wanted`).
///
/// Warmed under the frame's policy, which is the one a conversation opens with: showing remote
/// images is asked for per conversation, and opening another one clears it.
pub(in crate::ui) fn use_warming(shell: Signal<Shell>, threads: Memo<Vec<ThreadSummary>>) {
    let store = use_context::<Arc<SqliteStore>>();
    let frames = use_frames();
    // A memo, so that only a change of the open conversation counts, not every keystroke the
    // shell sees.
    let open = use_memo(move || shell.read().open);
    use_effect(move || {
        let list: Vec<ThreadId> = threads.read().iter().map(|thread| thread.id).collect();
        let order = ahead(open(), &list, FIRST_SCREEN);
        // The same order is what a body pass fetches first, with the open conversation ahead of
        // it: a message without its body yet cannot be rendered ahead, only fetched ahead.
        let bodies: Vec<ThreadId> = open().into_iter().chain(order.iter().copied()).collect();
        mail_core::wanted::ask_first(&bodies);
        frames.warm(store.clone(), order, SanitizePolicy::FRAME);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use mail_core::Store;
    use mail_mime::RemoteImages;

    #[test]
    fn warming_a_conversation_makes_opening_it_a_lookup() {
        let (store, _dir) = crate::ui::fixtures::realistic();
        let thread = crate::ui::fixtures::thread_like(&store, "rust-lang/rust");
        let messages: Vec<Message> = store
            .thread(thread)
            .unwrap()
            .messages
            .iter()
            .map(|id| store.message(*id).unwrap())
            .collect();
        let policy = SanitizePolicy::FRAME;
        assert!(
            messages.iter().any(|m| m.body.raw().is_some()),
            "the fixture has bodies"
        );
        let frames = Frames::plain();
        frames.warm_now(&store, &[thread], policy, || true);
        for message in messages.iter().filter(|m| m.body.raw().is_some()) {
            let had = frames.peek(message, policy).expect("warmed");
            assert_eq!(had, frames.uncached(&store, message, policy));
        }
    }

    #[test]
    fn warming_that_is_no_longer_wanted_stops() {
        let (store, _dir) = crate::ui::fixtures::realistic();
        let thread = crate::ui::fixtures::thread_like(&store, "rust-lang/rust");
        let message = store
            .message(store.thread(thread).unwrap().messages[0])
            .unwrap();
        let policy = SanitizePolicy {
            remote_images: RemoteImages::Allowed,
            version: u32::MAX,
            ..SanitizePolicy::FRAME
        };
        let frames = Frames::plain();
        frames.warm_now(&store, &[thread], policy, || false);
        assert_eq!(frames.peek(&message, policy), None);
    }
}
