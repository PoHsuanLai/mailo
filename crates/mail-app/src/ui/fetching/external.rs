//! The window hearing about what it did not write.
//!
//! A `mailo watch` stores the mail it fetches. It is another process, and the window's lists are
//! read again only when its revision moves. So the window looks, every couple of seconds, at
//! whether the store has been committed to by another connection
//! ([`SqliteStore::data_version`]), and moves the revision when it has.

use dioxus::prelude::*;
use mail_store::SqliteStore;
use std::sync::Arc;
use std::time::Duration;

/// How often the window looks.
#[cfg(not(test))]
const LOOK: Duration = Duration::from_secs(2);
#[cfg(test)]
pub(super) const LOOK: Duration = Duration::from_millis(30);

/// Move `revision` whenever another connection has committed to `store`. Once, from
/// [`super::use_fetching`].
pub(super) fn use_external_changes(mut revision: Signal<u64>, store: Arc<SqliteStore>) {
    let _looking = use_future(move || {
        let store = store.clone();
        async move {
            let mut seen = store.data_version();
            loop {
                tokio::time::sleep(LOOK).await;
                let now = store.data_version();
                // A busy writer answers nothing this time; the next look will.
                if now.is_some() && now != seen {
                    seen = now;
                    revision += 1;
                }
            }
        }
    });
}

#[cfg(test)]
mod tests;
