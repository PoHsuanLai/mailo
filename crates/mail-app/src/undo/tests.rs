use super::{DEPTH, Undo, UndoStack, reverse_intent, said};
use chrono::{TimeZone, Utc};
use mail_domain::*;

fn patch(changes: Vec<Change>) -> Patch {
    Patch {
        id: ChangeId::generate(),
        changes,
    }
}

#[test]
fn the_server_is_told_the_reverse_of_what_it_was_told() {
    let a = MessageId::generate();
    let b = MessageId::generate();
    let label = LabelId::generate();
    let archive = RemoteIntent::SetMailbox {
        messages: vec![a, b],
        role: MailboxRole::Archive,
    };
    let back_to_inbox = patch(vec![
        Change::MessageMailbox(a, MailboxRole::Inbox),
        Change::MessageMailbox(b, MailboxRole::Inbox),
    ]);
    assert_eq!(
        reverse_intent(&archive, &back_to_inbox),
        Some(RemoteIntent::SetMailbox {
            messages: vec![a, b],
            role: MailboxRole::Inbox,
        })
    );

    let read = RemoteIntent::SetFlags {
        messages: vec![a],
        read: Some(ReadState::Read),
        star: None,
    };
    assert_eq!(
        reverse_intent(
            &read,
            &patch(vec![Change::MessageRead(a, ReadState::Unread)])
        ),
        Some(RemoteIntent::SetFlags {
            messages: vec![a],
            read: Some(ReadState::Unread),
            star: None,
        })
    );

    let labelled = RemoteIntent::SetLabels {
        messages: vec![a],
        add: vec![label],
        remove: vec![],
    };
    assert_eq!(
        reverse_intent(
            &labelled,
            &patch(vec![Change::MessageLabel(a, label, Membership::Out)])
        ),
        Some(RemoteIntent::SetLabels {
            messages: vec![a],
            add: vec![],
            remove: vec![label],
        })
    );

    // An inverse with nothing of the forward's kind reverses nothing: a snooze has no server half.
    let thread = ThreadId::generate();
    assert_eq!(
        reverse_intent(
            &archive,
            &patch(vec![Change::ThreadSnooze(thread, Snooze::Inactive)])
        ),
        None
    );
}

#[test]
fn the_stack_forgets_the_oldest_past_its_depth() {
    let mut stack = UndoStack::default();
    let account = AccountId::generate();
    let threads: Vec<ThreadId> = (0..DEPTH + 3).map(|_| ThreadId::generate()).collect();
    for thread in &threads {
        stack.push(Undo {
            said: "Archived".to_owned(),
            thread: Some(*thread),
            account,
            forward: patch(vec![]),
            inverse: patch(vec![]),
            remote: None,
        });
    }
    assert_eq!(stack.len(), DEPTH);
    assert_eq!(stack.last().and_then(|u| u.thread), threads.last().copied());
    assert_eq!(stack.pop().and_then(|u| u.thread), threads.last().copied());
    assert_eq!(stack.len(), DEPTH - 1);
}

#[test]
fn a_handle_takes_back_its_own_entry_and_only_once() {
    // The toast holds the handle of the op it named. A later op pushed on top must not be what
    // its tab takes back.
    let mut stack = UndoStack::default();
    let account = AccountId::generate();
    let entry = |thread| Undo {
        said: "Archived".to_owned(),
        thread: Some(thread),
        account,
        forward: patch(vec![]),
        inverse: patch(vec![]),
        remote: None,
    };
    let (first, second) = (ThreadId::generate(), ThreadId::generate());
    let named = stack.push(entry(first));
    let later = stack.push(entry(second));
    assert_ne!(named, later);
    assert_eq!(stack.take(named).and_then(|u| u.thread), Some(first));
    assert_eq!(stack.take(named), None, "an entry is taken back once");
    assert_eq!(stack.last().and_then(|u| u.thread), Some(second));
}

#[test]
fn the_toast_says_what_happened() {
    let at = Utc.with_ymd_and_hms(2026, 9, 24, 9, 0, 0).unwrap();
    type Make = fn(chrono::DateTime<Utc>) -> Op;
    const CASES: &[(Make, &str)] = &[
        (|_| Op::Archive, "Archived"),
        (|_| Op::Trash, "Moved to Trash"),
        (|_| Op::SetStar(Star::Starred), "Starred"),
        (
            |at| Op::SetSnooze(Snooze::Until(at)),
            "Snoozed until Thu 09:00",
        ),
    ];
    for (make, want) in CASES {
        assert_eq!(said(&make(at), &Utc), *want);
    }
}
