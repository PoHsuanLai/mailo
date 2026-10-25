//! The client as a window holds it: one for the app, and its values as signals.
//!
//! A value the client keeps read ([`mail_client::Live`]) becomes a signal here, set from a task
//! that follows it. Rendering reads the signal and nothing else, so no frame waits on the store,
//! and a component re-renders when its value changes, not when some counter says something
//! somewhere might have.

use dioxus::dioxus_core::{Task, provide_root_context};
use dioxus::prelude::*;
use mail_client::{Client, Live, Query, Snapshot};
use mail_core::SqliteStore;
use std::sync::Arc;

/// The app's client: the one the program gave every window, or for a window built with only a
/// store (a unit test's), one that reads on the thread that draws, made once and shared from the
/// root.
pub(in crate::ui) fn use_client() -> Client {
    use_hook(|| {
        try_consume_context::<Client>().unwrap_or_else(|| {
            let store = consume_context::<Arc<SqliteStore>>();
            let client = Client::inline(store, Arc::new(mail_core::mail::SystemClock));
            provide_root_context(client.clone())
        })
    })
}

/// The value `key` asks for, as a signal. `key` is a memo: it follows the signals it reads, and
/// an equal query is not asked again. A new query is asked for as the frame is drawn — the
/// client's registry, never the store — so a frame never shows one query's answer under
/// another's; while it is being read the value shows the last one's, so a change of Space does
/// not flash empty.
pub(in crate::ui) fn use_live<Q: Query>(
    key: impl FnMut() -> Q + 'static,
) -> Memo<Snapshot<Q::Output>> {
    let client = use_client();
    let query = use_memo(key);
    let held = use_memo(move || Held(client.live(query())));
    // Moved by the task that follows the value, so the frame reads it again.
    let mut told = use_signal(|| 0u64);
    let mut last = use_hook(|| CopyValue::new(None::<Arc<Q::Output>>));
    let out = use_memo(move || {
        let _ = told();
        let now = held.read().0.snapshot().showing(last.peek().clone());
        if let Some(value) = now.value() {
            last.set(Some(value.clone()));
        }
        now
    });
    let mut following = use_hook(|| CopyValue::new(None::<Task>));
    use_effect(move || {
        let mut live = held.read().0.clone();
        if let Some(task) = following.take() {
            task.cancel();
        }
        following.set(Some(spawn(async move {
            while live.changed().await {
                told += 1;
            }
        })));
    });
    out
}

/// A held value, as a memo keeps it: a new one only when the query changed, and never equal to
/// the last, so what follows it moves on to it.
struct Held<T>(Live<T>);

impl<T> PartialEq for Held<T> {
    fn eq(&self, _: &Self) -> bool {
        false
    }
}

/// What `query` answers now, read here: for a test that checks what a component makes of it.
#[cfg(test)]
pub(in crate::ui) fn read_now<Q: Query>(store: &SqliteStore, query: Q) -> Q::Output {
    query.run(store, chrono::Utc::now()).expect("read")
}
