//! What the list pane shows, fetched off the thread that draws, a pause after the last keystroke.
//!
//! Split from [`super::app`] (`CONVENTIONS.md` §8). The place's list follows the shell at once:
//! choosing a place, a tile or another page is one query. A search follows the box through
//! [`super::debounce`]: it runs once the box has been still for its quiet period, and a result
//! for text a newer keystroke has replaced is dropped rather than drawn.

use super::data::PAGE;
use super::debounce::use_debounced;
use super::list_search::{Listed, Request, listed};
use crate::ui::view::Shell;
use dioxus::prelude::*;
use mail_domain::{ThreadId, ThreadSummary};
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
    /// Whether a deeper page has been asked for and has not been drawn yet.
    pub paging: Memo<bool>,
    /// Which question the rows answer, counted: it moves when rows for another place or another
    /// search land and take rows out of the list, and not when the same one is asked again
    /// deeper or after the store moved. The list is keyed by it, so a new answer replaces the
    /// rows outright instead of playing an exit for every row the last answer had — a search
    /// that drops 240 rows would otherwise stay unreadable while they leave. An archive is the
    /// same question, and its row still leaves the way the design says.
    pub asked: ReadSignal<u64>,
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
    // A memo, so a shell change the list does not depend on — a letter typed into ⌘F, a
    // peek mode — does not run the search again. The generation is part of it, so typing back
    // to the text last searched for is still a query of its own and cannot be dropped as stale.
    let request = use_memo(move || {
        let settled = settled.read();
        let shell = shell.read();
        let request = Request::of(&shell, &settled.text, PAGE * pages());
        // The same question at no depth, to tell another question from a deeper page.
        let question = Request::of(&shell, &settled.text, 0);
        (settled.generation, request, question)
    });
    let mut drawn = use_signal(|| {
        let store = consume_context::<Arc<SqliteStore>>();
        listed(&store, request.peek().1.clone(), chrono::Utc::now())
    });
    let mut reached = use_signal(|| *pages.peek());
    let mut answered = use_signal(|| request.peek().2.clone());
    let mut asked = use_signal(|| 0u64);
    let _fetch = use_resource(move || {
        let _ = revision();
        let (generation, request, question) = request();
        let depth = *pages.peek();
        let store = consume_context::<Arc<SqliteStore>>();
        async move {
            let done: Listed =
                tokio::task::spawn_blocking(move || listed(&store, request, chrono::Utc::now()))
                    .await
                    .unwrap_or_default();
            // A keystroke since this was asked for means the box no longer says this. Drawing it
            // would flash an answer to a question nobody is asking.
            if debounced.is_latest(generation) {
                if *answered.peek() != question {
                    // Another question. A new list only when it would lose rows: the same rows
                    // under a reworded filter are the same list, and redrawing it whole would
                    // cost a frame for nothing.
                    if drops_rows(&drawn.peek(), &done) {
                        asked += 1;
                    }
                    answered.set(question);
                }
                drawn.set(done);
                reached.set(depth);
            }
        }
    });
    ListView {
        threads: use_memo(move || drawn.read().threads.clone()),
        top: use_memo(move || drawn.read().top.clone()),
        marking: use_memo(move || drawn.read().marking.clone()),
        paging: use_memo(move || pages() > reached()),
        asked: asked.into(),
    }
}

/// Whether drawing `next` over `was` would take a row out of the list: a conversation in either
/// part of `was` that `next` does not hold.
fn drops_rows(was: &Listed, next: &Listed) -> bool {
    let kept = |id: &ThreadId| {
        next.threads
            .iter()
            .chain(&next.top)
            .any(|thread| thread.id == *id)
    };
    was.threads
        .iter()
        .chain(&was.top)
        .any(|thread| !kept(&thread.id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use mail_domain::id::new_account_id;
    use mail_domain::*;

    fn row() -> ThreadSummary {
        ThreadSummary {
            id: ThreadId::generate(),
            account: new_account_id(),
            subject: "s".into(),
            snippet: String::new(),
            from: Address {
                name: None,
                email: "a@b.test".into(),
            },
            participants: vec![],
            recipients: vec![],
            last_date: chrono::Utc.timestamp_opt(1_700_000_000, 0).unwrap(),
            message_count: 1,
            read: ReadState::Unread,
            star: Star::Unstarred,
            mailboxes: MailboxSet::only(MailboxRole::Inbox),
            labels: vec![],
            attachments: Attachments::None,
            snooze: Snooze::Inactive,
            pin: Pin::Unpinned,
            mute: Mute::Unmuted,
            follow_up: FollowUp::Inactive,
        }
    }

    fn listing(threads: &[ThreadSummary], top: &[ThreadSummary]) -> Listed {
        Listed {
            threads: threads.to_vec(),
            top: top.to_vec(),
            marking: Default::default(),
        }
    }

    #[test]
    fn a_new_list_only_when_rows_would_leave() {
        let (a, b, c) = (row(), row(), row());
        let was = listing(&[a.clone(), b.clone()], &[]);
        // The same rows, or more of them: nothing leaves.
        assert!(!drops_rows(&was, &listing(&[a.clone(), b.clone()], &[])));
        assert!(!drops_rows(
            &was,
            &listing(&[a.clone(), b.clone(), c.clone()], &[])
        ));
        // A row moved up into the top results has not left.
        assert!(!drops_rows(
            &was,
            &listing(std::slice::from_ref(&a), std::slice::from_ref(&b))
        ));
        // A search that keeps one of them drops the other.
        assert!(drops_rows(&was, &listing(&[a, c], &[])));
    }
}
