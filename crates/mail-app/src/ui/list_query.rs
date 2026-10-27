//! What the list pane shows, read by the client off the thread that draws, a pause after the last
//! keystroke.
//!
//! Split from [`super::app`]. The place's list follows the shell at once: choosing a place, a
//! Space or another page is one question. A search follows the box through [`super::debounce`]:
//! it is asked once the box has been still for its quiet period, and an answer for a question no
//! longer asked is never drawn.

use super::data::PAGE;
use super::debounce::use_debounced;
use super::list_search::{Listed, Request};
use super::live::use_live;
use crate::ui::view::Shell;
use dioxus::prelude::*;
use mail_client::Snapshot;
use mail_domain::{MessageId, ThreadId, ThreadSummary};

/// The list's answer, split into what each part of the pane reads.
#[derive(Clone, Copy)]
pub(super) struct ListView {
    /// The rows: the place, or a search's matches newest first.
    pub threads: Memo<Vec<ThreadSummary>>,
    /// Which message each row is, row for row, while the place lists a message to a row; empty
    /// while it lists conversations.
    pub messages: Memo<Vec<MessageId>>,
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

/// The list for `shell`, `pages` pages deep, as the client keeps it read: again whenever a commit
/// touches what it lists, off the thread that draws.
///
/// The pane shows what it last drew until the answer for a new question lands, never a blank.
pub(super) fn use_list(shell: Signal<Shell>, pages: Signal<u32>) -> ListView {
    let debounced = use_debounced(shell, |shell| shell.search.clone());
    let settled = debounced.settled;
    // A memo, so a shell change the list does not depend on — a letter typed into ⌘F, a
    // peek mode — does not ask again.
    let request = use_memo(move || {
        let settled = settled.read();
        let shell = shell.read();
        let request = Request::of(&shell, &settled.text, PAGE * pages());
        // The same question at no depth, to tell another question from a deeper page.
        let question = Request::of(&shell, &settled.text, 0);
        (request, question, pages())
    });
    let found = use_live(move || request().0.asked());
    let mut drawn = use_signal(|| {
        found
            .peek()
            .value()
            .map(|found| Listed::from(found.as_ref()))
            .unwrap_or_default()
    });
    let mut reached = use_signal(|| *pages.peek());
    let mut answered = use_signal(|| request.peek().1.clone());
    let mut asked = use_signal(|| 0u64);
    // An answer for the question asked now — not the last one's, shown while this one is read —
    // is drawn. The question is followed as well as the answer: a new question can have the
    // same answer, and it is still the one the rows answer from then on.
    use_effect(move || {
        let (_, question, depth) = request();
        let Snapshot::Ready { value, .. } = &*found.read() else {
            return;
        };
        let done = Listed::from(value.as_ref());
        if *answered.peek() != question {
            // Another question. A new list only when it would lose rows: the same rows under a
            // reworded filter are the same list, and redrawing it whole would cost a frame for
            // nothing.
            if drops_rows(&drawn.peek(), &done) {
                asked += 1;
            }
            answered.set(question);
        }
        if *drawn.peek() != done {
            drawn.set(done);
        }
        reached.set(depth);
    });
    ListView {
        threads: use_memo(move || drawn.read().threads.clone()),
        messages: use_memo(move || drawn.read().messages.clone()),
        top: use_memo(move || drawn.read().top.clone()),
        marking: use_memo(move || drawn.read().marking.clone()),
        paging: use_memo(move || pages() > reached()),
        asked: asked.into(),
    }
}

/// Whether drawing `next` over `was` would take a row out of the list: a conversation in either
/// part of `was` that `next` does not hold, or, a message to a row, a message. Turning
/// Organise by Conversation on or off makes every row another row.
fn drops_rows(was: &Listed, next: &Listed) -> bool {
    if !was.messages.is_empty() || !next.messages.is_empty() {
        return was
            .messages
            .iter()
            .any(|message| !next.messages.contains(message))
            || (was.messages.is_empty() && !was.threads.is_empty());
    }
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

    use mail_domain::*;

    fn row() -> ThreadSummary {
        ThreadSummary {
            from: Address {
                name: None,
                email: "a@b.test".into(),
            },
            ..mail_domain::testing::summary()
        }
    }

    fn listing(threads: &[ThreadSummary], top: &[ThreadSummary]) -> Listed {
        Listed {
            threads: threads.to_vec(),
            messages: Vec::new(),
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
        assert!(drops_rows(&was, &listing(&[a.clone(), c], &[])));

        // A message to a row: a message that leaves is a row that leaves, whatever its
        // conversation, and turning conversations on or off replaces every row.
        let single = |messages: &[u128]| Listed {
            messages: messages
                .iter()
                .map(|n| MessageId::from_uuid(uuid::Uuid::from_u128(*n)))
                .collect(),
            ..listing(&vec![a.clone(); messages.len()], &[])
        };
        assert!(!drops_rows(&single(&[1, 2]), &single(&[1, 2, 3])));
        assert!(drops_rows(&single(&[1, 2]), &single(&[1])));
        assert!(drops_rows(&was, &single(&[1])));
        assert!(drops_rows(&single(&[1]), &was));
    }
}
