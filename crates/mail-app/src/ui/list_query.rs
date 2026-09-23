//! What the list pane shows, fetched off the thread that draws, a pause after the last keystroke.
//!
//! Split from [`super::app`] (`CONVENTIONS.md` §8). The place's list follows the shell at once:
//! choosing a place, a tile or another page is one query. A search follows the box through
//! [`super::debounce`]: it runs once the box has been still for its quiet period, and a result
//! for text a newer keystroke has replaced is dropped rather than drawn.

use super::data::PAGE;
use super::debounce::use_debounced;
use super::list_search::{Listed, Request, listed};
use crate::view::Shell;
use dioxus::prelude::*;
use mail_domain::ThreadSummary;
use mail_store::SqliteStore;
use std::sync::Arc;

/// The list's answer, split into what each part of the pane reads.
#[derive(Clone, Copy)]
pub(super) struct ListView {
    /// The rows: the place, or a search's matches newest first.
    pub threads: Memo<Vec<ThreadSummary>>,
    /// A search's "Top results" strip.
    pub top: Memo<Vec<ThreadSummary>>,
    pub marking: Memo<super::list_search::Marking>,
}

/// Run the list for `shell`, `pages` pages deep, again whenever `revision` moves.
///
/// Computed here the first time, off the thread afterwards. The first attempt at phase 8c was a
/// bare `use_resource`, and a bare resource is empty until it resolves — so the window opened on
/// an empty mailbox, and under F140 stayed that way because nothing ever polled the task. This
/// keeps the synchronous answer for the frame that has no other one, and afterwards the pane
/// shows what it last drew until a newer answer lands, never a blank and never a query on this
/// thread.
pub(super) fn use_list(
    shell: Signal<Shell>,
    pages: Signal<u32>,
    revision: Signal<u64>,
) -> ListView {
    let debounced = use_debounced(shell, |shell| shell.search.clone());
    let settled = debounced.settled;
    // A memo, so a shell change the list does not depend on — a letter typed into Ctrl F, a
    // peek mode — does not run the search again. The generation is part of it, so typing back
    // to the text last searched for is still a query of its own and cannot be dropped as stale.
    let request = use_memo(move || {
        let settled = settled.read();
        let request = Request::of(&shell.read(), &settled.text, PAGE * pages());
        (settled.generation, request)
    });
    let mut drawn = use_signal(|| {
        let store = consume_context::<Arc<SqliteStore>>();
        listed(&store, request.peek().1.clone(), chrono::Utc::now())
    });
    let _fetch = use_resource(move || {
        let _ = revision();
        let (generation, request) = request();
        let store = consume_context::<Arc<SqliteStore>>();
        async move {
            let done: Listed =
                tokio::task::spawn_blocking(move || listed(&store, request, chrono::Utc::now()))
                    .await
                    .unwrap_or_default();
            // A keystroke since this was asked for means the box no longer says this. Drawing it
            // would flash an answer to a question nobody is asking.
            if debounced.is_latest(generation) {
                drawn.set(done);
            }
        }
    });
    ListView {
        threads: use_memo(move || drawn.read().threads.clone()),
        top: use_memo(move || drawn.read().top.clone()),
        marking: use_memo(move || drawn.read().marking.clone()),
    }
}
