//! The window hearing about what it did not write.
//!
//! The client hears every commit to the store, whoever made it: this window, another window, the
//! background watch writing through the same store, a `mailo watch` or `mailo` in a terminal in
//! another process (`mail_client`, its `external`). What the window still reads from the store
//! itself, rather than as a value the client keeps read, is read again when its revision moves;
//! so the revision moves each time the client says the store changed.
//!
//! (Until 2026-10 the window watched the store's file itself and asked SQLite whether another
//! connection had committed, which missed what the background watch committed through the
//! window's own store.)

use dioxus::prelude::*;

/// Move `revision` whenever the client says the store changed. Once, from
/// [`super::use_fetching`].
pub(super) fn use_external_changes(mut revision: Signal<u64>) {
    let client = crate::ui::live::use_client();
    let _following = use_future(move || {
        let mut moved = client.moved();
        async move {
            while moved.changed().await.is_ok() {
                revision += 1;
            }
        }
    });
}

#[cfg(test)]
mod tests;
