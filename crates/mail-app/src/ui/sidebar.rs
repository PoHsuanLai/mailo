//! The places down the side of the window.
//!
//! Where a conversation can be, plus the two actions that are not about one conversation:
//! writing a new message, and fetching mail. Split from [`super::app`] (`CONVENTIONS.md` §8).
//! The hooks that feed the badges stay in `App`; this only reads the memo it is handed, so a
//! keystroke in the search box does not recount them.

use super::ops::start_new;
use crate::view::{Shell, SyncState, synced};
use dioxus::prelude::*;
use mail_store::SqliteStore;
use std::sync::Arc;

/// The places a conversation can be, and the way to write or fetch.
#[component]
pub(super) fn Places(
    shell: Signal<Shell>,
    pages: Signal<u32>,
    badges: Memo<Vec<Option<u64>>>,
    revision: Signal<u64>,
    sync_state: Signal<SyncState>,
) -> Element {
    rsx! {
        nav { class: "places",
            for (index, place) in shell.read().places.iter().enumerate() {
                button {
                    key: "{place.name}",
                    class: if index == shell.read().selected { "place on" } else { "place" },
                    onclick: move |_| {
                        shell.write().select(index);
                        pages.set(1);
                    },
                    "{place.name}"
                    if let Some(Some(count)) = badges().get(index).copied() {
                        span { class: "badge", "{count}" }
                    }
                }
            }
            button {
                class: "place compose",
                onclick: move |_| {
                    let store = consume_context::<Arc<SqliteStore>>();
                    let known = shell.peek().accounts.clone();
                    match start_new(&store, &known) {
                        Ok(draft) => {
                            shell.write().compose(&draft);
                            revision += 1;
                        }
                        Err(why) => eprintln!("compose: {why}"),
                    }
                },
                title: "Write a new message (c)",
                "New"
            }
            div { class: "spacer" }
            button {
                class: "place sync",
                disabled: !sync_state.read().may_start(),
                onclick: move |_| {
                    if !sync_state.read().may_start() {
                        return;
                    }
                    sync_state.set(SyncState::Running);
                    let store = consume_context::<Arc<SqliteStore>>();
                    spawn(async move {
                        // `spawn_blocking`, not this task: sync::run opens sockets and
                        // builds its own runtime, and `Runtime::block_on` inside an async
                        // context panics. Off the UI thread either way — a pass takes
                        // minutes on a first sync and would freeze the window.
                        let done = tokio::task::spawn_blocking(move || {
                            crate::sync::run(store, chrono::Utc::now())
                        })
                        .await;
                        sync_state.set(match done {
                            Ok(result) => synced(result.map(|ran| ran.text)),
                            // The blocking task panicked. Saying so beats a window that
                            // sits on "Syncing…" for ever.
                            Err(e) => synced(Err(format!("the sync pass stopped: {e}"))),
                        });
                        revision += 1;
                    });
                },
                if sync_state.read().may_start() { "Sync" } else { "Syncing…" }
            }
            if let Some(note) = sync_state.read().message() {
                p {
                    class: if sync_state.read().is_failure() { "sync-note bad" } else { "sync-note" },
                    "{note}"
                }
            }
        }
    }
}
