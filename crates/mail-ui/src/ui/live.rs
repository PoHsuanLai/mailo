//! The client as a window holds it: one for the app, and its values as signals.
//!
//! A value the client keeps read ([`mail_client::Live`]) becomes a signal here, set from a task
//! that follows it. Rendering reads the signal and nothing else, so no frame waits on the store,
//! and a component re-renders when its value changes, not when some counter says something
//! somewhere might have.

use crate::ui::clock::WallClock;
use dioxus::dioxus_core::{Task, provide_root_context};
use dioxus::prelude::*;
use mail_client::mail::Clock;
use mail_client::{Client, Live, Query, Snapshot};
use std::sync::Arc;

/// The app's client: the one the program gave every window (a test's, one that reads on the
/// thread that draws).
///
/// A window with a [`WallClock`] of its own (a test's, which follows quire's virtual clock and
/// jumps ahead) gives the client its time, so what the client reads by the clock — a snooze
/// ending, a reminder coming due — is read at the window's time.
pub(in crate::ui) fn use_client() -> Client {
    use_hook(client)
}

/// [`use_client`], from a handler or a task.
pub(in crate::ui) fn client() -> Client {
    let client = consume_context::<Client>();
    // Once for the app, not on every component that asks.
    if try_consume_context::<ClockGiven>().is_none() {
        provide_root_context(ClockGiven);
        if let Some(wall) = try_consume_context::<WallClock>() {
            // A client that reads here reads the window's clock itself; one with threads of its
            // own is told how far ahead it is.
            let clock: Arc<dyn Clock> = if client.keeps_time() {
                Arc::new(Ahead::of(&wall))
            } else {
                Arc::new(Wall(wall))
            };
            client.set_clock(clock);
        }
    }
    client
}

/// How many times the store has moved, here or in another process, as the client tells it
/// ([`Client::moved`]): for what still reads the store itself rather than a [`Live`], to read
/// again. Read inside a memo or a component, it subscribes them, so they run again when the
/// store moves; nobody bumps it by hand.
pub(in crate::ui) fn moved() -> u64 {
    moved_signal()()
}

/// [`moved`], without subscribing what reads it.
pub(in crate::ui) fn moved_peek() -> u64 {
    *moved_signal().peek()
}

/// The window's one signal following [`Client::moved`], made at the root the first time it is
/// asked for and set from a task that lives as long as the window.
#[derive(Clone, Copy)]
struct Moved(Signal<u64>);

fn moved_signal() -> Signal<u64> {
    if let Some(Moved(moved)) = try_consume_context::<Moved>() {
        return moved;
    }
    let client = client();
    let mut heard = client.moved();
    let mut moved = Signal::new_in_scope(*heard.borrow_and_update(), ScopeId::ROOT);
    provide_root_context(Moved(moved));
    // A client that reads on the thread that draws (a test's) acts on a commit when it is told
    // to, here, which is what moves it.
    if let Some(mut committed) = client.committed() {
        dioxus::dioxus_core::spawn_forever(async move {
            while committed.changed().await.is_ok() {
                client.settle();
            }
        });
    }
    dioxus::dioxus_core::spawn_forever(async move {
        while heard.changed().await.is_ok() {
            let now = *heard.borrow_and_update();
            moved.set(now);
        }
    });
    moved
}

/// [`client`], for a window that may have been handed no client: `None` there.
pub(in crate::ui) fn try_client() -> Option<Client> {
    try_consume_context::<Client>().is_some().then(client)
}

/// Tell `client` the clock has passed something it may count — a snooze ending, a reminder
/// coming due — so what it reads by the clock is read again, at the window's time.
pub(in crate::ui) fn clock_moved(client: &Client) {
    // Given only to a client with threads of its own; one that reads here reads the window's
    // clock itself.
    match try_consume_context::<WallClock>() {
        Some(wall) if try_consume_context::<ClockGiven>().is_some() && client.keeps_time() => {
            client.set_clock(Arc::new(Ahead::of(&wall)));
        }
        _ => client.invalidate(mail_client::Topics::CLOCK),
    }
}

/// That the window's clock was given to the program's client.
#[derive(Clone, Copy)]
struct ClockGiven;

/// The window's clock, read where the window reads it: for a client that reads on the thread
/// that draws.
struct Wall(WallClock);

impl Clock for Wall {
    fn now(&self) -> chrono::DateTime<chrono::Utc> {
        self.0.now()
    }
}

/// The window's clock for a client that reads on threads of its own: how far the window's time
/// is from the system's, taken on the thread that draws. A test's window clock follows quire's
/// virtual clock, which only that thread has, so the client's threads keep the system's time
/// moved on by how far the window's was ahead when it was last told.
struct Ahead(chrono::TimeDelta);

impl Ahead {
    fn of(wall: &WallClock) -> Ahead {
        Ahead(wall.now() - chrono::Utc::now())
    }
}

impl Clock for Ahead {
    fn now(&self) -> chrono::DateTime<chrono::Utc> {
        chrono::Utc::now() + self.0
    }
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
pub(in crate::ui) fn read_now<Q: Query>(client: &Client, query: Q) -> Q::Output {
    client.read(query).expect("read")
}
