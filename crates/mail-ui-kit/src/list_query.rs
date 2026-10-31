//! What the list pane shows, read by the client off the thread that draws, a pause after the last
//! keystroke.
//!
//! Split from [`super::app`]. The place's list follows the shell at once: choosing a place, a
//! Space or another page is one question. A search follows the box through [`crate::debounce`]:
//! it is asked once the box has been still for its quiet period, and an answer for a question no
//! longer asked is never drawn.

use crate::data::PAGE;
use crate::debounce::use_debounced;
use crate::list_search::{Listed, Request};
use crate::live::use_live;
use crate::view::Shell;
use dioxus::prelude::*;
use mail_client::{Generation, Snapshot};
use mail_domain::{Filter, MessageId, Op, ThreadId, ThreadSummary};

/// The list's answer, split into what each part of the pane reads.
#[derive(Debug, Clone, Copy)]
pub struct ListView {
    /// The rows: the place, or a search's matches newest first.
    pub threads: Memo<Vec<ThreadSummary>>,
    /// Which message each row is, row for row, while the place lists a message to a row; empty
    /// while it lists conversations.
    pub messages: Memo<Vec<MessageId>>,
    /// A search's "Top results" strip.
    pub top: Memo<Vec<ThreadSummary>>,
    pub marking: Memo<crate::list_search::Marking>,
    /// Whether a deeper page has been asked for and has not been drawn yet.
    pub paging: Memo<bool>,
    /// Which question the rows answer, counted: it moves when rows for another place or another
    /// search land and take rows out of the list, and not when the same one is asked again
    /// deeper or after the store moved. The list is keyed by it, so a new answer replaces the
    /// rows outright instead of playing an exit for every row the last answer had — a search
    /// that drops 240 rows would otherwise stay unreadable while they leave. An archive is the
    /// same question, and its row still leaves the way the design says.
    pub asked: ReadSignal<u64>,
    /// Whether the last reading of the rows failed (the rows shown, if any, are an earlier
    /// one's): the pane says so when it has none.
    pub failed: Memo<bool>,
}

/// The list for `shell`, `pages` pages deep, as the client keeps it read: again whenever a commit
/// touches what it lists, off the thread that draws.
///
/// The pane shows what it last drew until the answer for a new question lands, never a blank.
pub fn use_list(shell: Signal<Shell>, pages: Signal<u32>) -> ListView {
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
    // The generation the rows were read at, which tells the window's own writes in them from
    // those still on their way.
    let mut read_at = use_signal(Generation::default);
    let mut hides = use_signal(Vec::<Hide>::new);
    let listed = use_memo(move || request().0.filter());
    use_context_provider(|| Hidden {
        hides,
        filter: listed,
        drawn,
        read_at,
    });
    let mut answered = use_signal(|| request.peek().1.clone());
    let mut asked = use_signal(|| 0u64);
    // An answer for the question asked now — not the last one's, shown while this one is read —
    // is drawn. The question is followed as well as the answer: a new question can have the
    // same answer, and it is still the one the rows answer from then on.
    use_effect(move || {
        let (_, question, depth) = request();
        let Snapshot::Ready { value, as_of } = &*found.read() else {
            return;
        };
        if *read_at.peek() != *as_of {
            read_at.set(*as_of);
            // Those the rows have been read past are no longer anything to keep.
            let past = |hide: &Hide| hide.until.is_some_and(|until| until <= *as_of);
            if hides.peek().iter().any(past) {
                hides.write().retain(|hide| !past(hide));
            }
        }
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
    // What a write of the window's own took out of the list and the rows do not show yet is not
    // drawn: the row leaves on the press, not when the store has it.
    let hidden = move |id: ThreadId| {
        let at = read_at();
        hides
            .read()
            .iter()
            .filter(|hide| hide.until.is_none_or(|until| at < until))
            .any(|hide| hide.threads.contains(&id))
    };
    let shown = move |rows: &[ThreadSummary]| {
        rows.iter()
            .filter(|row| !hidden(row.id))
            .cloned()
            .collect::<Vec<_>>()
    };
    ListView {
        threads: use_memo(move || shown(&drawn.read().threads)),
        messages: use_memo(move || {
            let drawn = drawn.read();
            drawn
                .threads
                .iter()
                .zip(&drawn.messages)
                .filter(|(row, _)| !hidden(row.id))
                .map(|(_, message)| *message)
                .collect()
        }),
        top: use_memo(move || shown(&drawn.read().top)),
        marking: use_memo(move || drawn.read().marking.clone()),
        paging: use_memo(move || pages() > reached()),
        asked: asked.into(),
        failed: use_memo(move || matches!(*found.read(), Snapshot::Failed { .. })),
    }
}

/// What a write of the window's own takes out of the list, not drawn from the press until the
/// rows are read at or after the generation it committed at.
#[derive(Debug, Clone, PartialEq)]
struct Hide {
    serial: u64,
    threads: Vec<ThreadId>,
    /// `None` while the write is on its way.
    until: Option<Generation>,
}

/// The list's hidden rows, as a write reaches them: provided by [`use_list`].
#[derive(Debug, Clone, Copy)]
pub struct Hidden {
    hides: Signal<Vec<Hide>>,
    /// What the place lists, when the summary alone can say whether a conversation is in it;
    /// `None` for a search, whose rows are only known once it is asked again.
    filter: Memo<Option<Filter>>,
    drawn: Signal<Listed>,
    read_at: Signal<Generation>,
}

/// The rows a write takes out of the list, once it is under way: [`Hidden::settle`] it when
/// the write is done.
#[derive(Debug, Clone, Copy)]
pub struct Hiding(Option<(Hidden, u64)>);

/// Stop drawing what `ops` take out of the window's list, now, before they are written. Nothing
/// for a window with no list, or a list whose rows only a reading can tell.
pub fn hide(ops: &[(ThreadId, Op)]) -> Hiding {
    let Some(hidden) = try_consume_context::<Hidden>() else {
        return Hiding(None);
    };
    let Some(filter) = hidden.filter.peek().clone() else {
        return Hiding(None);
    };
    let now = crate::clock::now();
    let threads: Vec<ThreadId> = {
        let drawn = hidden.drawn.peek();
        ops.iter()
            .filter(|(thread, op)| {
                drawn
                    .threads
                    .iter()
                    .find(|row| row.id == *thread)
                    .is_some_and(|row| mail_client::act::leaves(&filter, row, op, now))
            })
            .map(|(thread, _)| *thread)
            .collect()
    };
    if threads.is_empty() {
        return Hiding(None);
    }
    let mut hides = hidden.hides;
    let serial = hides
        .peek()
        .iter()
        .map(|hide| hide.serial)
        .max()
        .unwrap_or(0)
        + 1;
    hides.write().push(Hide {
        serial,
        threads,
        until: None,
    });
    Hiding(Some((hidden, serial)))
}

impl Hiding {
    /// The write is done, at `generation`: what it took out stays hidden until the rows are read
    /// at it, and `kept` (what it did not do after all) is shown again now. `None` when it was
    /// not done at all, and everything is shown again.
    pub fn settle(self, generation: Option<Generation>, kept: &[ThreadId]) {
        let Some((hidden, serial)) = self.0 else {
            return;
        };
        let at = *hidden.read_at.peek();
        let mut hides = hidden.hides;
        let mut hides = hides.write();
        // Those the rows have been read past are no longer anything to keep.
        hides.retain(|hide| hide.until.is_none_or(|until| at < until));
        match generation {
            Some(generation) => {
                for hide in hides.iter_mut().filter(|hide| hide.serial == serial) {
                    hide.until = Some(generation);
                    hide.threads.retain(|thread| !kept.contains(thread));
                }
            }
            None => hides.retain(|hide| hide.serial != serial),
        }
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

    /// The file that reads the list asks the store nothing: what a
    /// frame shows is a query the client keeps read, and what a press changes is a command. A
    /// call to `stored()` on the thread that draws waits behind whatever an ingest holds.
    #[test]
    fn the_files_that_draw_ask_the_store_nothing() {
        let files = [("list_query.rs", include_str!("list_query.rs"))];
        for (name, source) in files {
            let drawn = source.split("#[cfg(test)]").next().unwrap_or(source);
            let calls: Vec<&str> = drawn
                .lines()
                .filter(|line| !line.trim_start().starts_with("//"))
                .filter(|line| line.contains(".stored()"))
                .collect();
            assert!(calls.is_empty(), "{name} asks the store: {calls:?}");
        }
    }

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
