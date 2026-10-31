use super::{DEPTH, Undo, UndoStack, reverse_intent};
use mail_domain::id::new_account_id;
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

/// A colour undone goes back to what each message wore: unflagged, or flagged in the colour it
/// had. An unflagging undone puts the colour back with the flag, which the server lost with it.
#[test]
fn a_flag_colour_is_taken_back_to_what_it_was() {
    let a = MessageId::generate();
    let orange = RemoteIntent::SetFlag {
        messages: vec![a],
        hue: Hue::Orange,
    };
    let was_unflagged = patch(vec![
        Change::MessageStar(a, Star::Unstarred),
        Change::MessageFlag(a, Hue::Red),
    ]);
    assert_eq!(
        reverse_intent(&orange, &was_unflagged),
        Some(RemoteIntent::SetFlags {
            messages: vec![a],
            read: None,
            star: Some(Star::Unstarred),
        })
    );
    let was_blue = patch(vec![Change::MessageFlag(a, Hue::Blue)]);
    assert_eq!(
        reverse_intent(&orange, &was_blue),
        Some(RemoteIntent::SetFlag {
            messages: vec![a],
            hue: Hue::Blue,
        })
    );

    let unflag = RemoteIntent::SetFlags {
        messages: vec![a],
        read: None,
        star: Some(Star::Unstarred),
    };
    let was_green = patch(vec![
        Change::MessageStar(a, Star::Starred),
        Change::MessageFlag(a, Hue::Green),
    ]);
    assert_eq!(
        reverse_intent(&unflag, &was_green),
        Some(RemoteIntent::SetFlag {
            messages: vec![a],
            hue: Hue::Green,
        })
    );
    // A red flag is plain `\Flagged`, back as a star.
    assert_eq!(
        reverse_intent(&unflag, &patch(vec![Change::MessageStar(a, Star::Starred)])),
        Some(RemoteIntent::SetFlags {
            messages: vec![a],
            read: None,
            star: Some(Star::Starred),
        })
    );
}

/// Unflagging five red and one orange and taking it back restores the five as they were; the
/// colour of the one is the most the undo gives up, never the five.
#[test]
fn the_most_common_colour_wins_an_undone_unflagging() {
    let reds: Vec<MessageId> = (0..5).map(|_| MessageId::generate()).collect();
    let orange = MessageId::generate();
    let everyone: Vec<MessageId> = reds.iter().copied().chain([orange]).collect();
    let unflag = RemoteIntent::SetFlags {
        messages: everyone.clone(),
        read: None,
        star: Some(Star::Unstarred),
    };
    let mut changes: Vec<Change> = everyone
        .iter()
        .map(|id| Change::MessageStar(*id, Star::Starred))
        .collect();
    changes.push(Change::MessageFlag(orange, Hue::Orange));
    let back = reverse_intent(&unflag, &patch(changes)).unwrap();
    let RemoteIntent::SetFlags { messages, star, .. } = back else {
        panic!("red is plain: {back:?}");
    };
    assert_eq!(star, Some(Star::Starred));
    for red in &reds {
        assert!(messages.contains(red), "a red one was dropped");
    }
}

#[test]
fn the_stack_forgets_the_oldest_past_its_depth() {
    let mut stack = UndoStack::default();
    let account = new_account_id();
    let threads: Vec<ThreadId> = (0..DEPTH + 3).map(|_| ThreadId::generate()).collect();
    for thread in &threads {
        stack.push(Undo {
            thread: Some(*thread),
            account: account.clone(),
            forward: patch(vec![]),
            inverse: patch(vec![]),
            remote: None,
            resend: None,
        });
    }
    assert_eq!(stack.len(), DEPTH);
    assert_eq!(
        stack.last().and_then(|u| u[0].thread),
        threads.last().copied()
    );
    assert_eq!(
        stack.pop().and_then(|u| u[0].thread),
        threads.last().copied()
    );
    assert_eq!(stack.len(), DEPTH - 1);
}

#[test]
fn a_handle_takes_back_its_own_entry_and_only_once() {
    // A caller holds the handle of the op it named. A later op pushed on top must not be what
    // that handle takes back.
    let mut stack = UndoStack::default();
    let account = new_account_id();
    let entry = |thread| Undo {
        thread: Some(thread),
        account: account.clone(),
        forward: patch(vec![]),
        inverse: patch(vec![]),
        remote: None,
        resend: None,
    };
    let (first, second) = (ThreadId::generate(), ThreadId::generate());
    let named = stack.push(entry(first));
    let later = stack.push(entry(second));
    assert_ne!(named, later);
    assert_eq!(stack.take(named).and_then(|u| u[0].thread), Some(first));
    assert_eq!(stack.take(named), None, "an entry is taken back once");
    assert_eq!(stack.last().and_then(|u| u[0].thread), Some(second));
}

#[test]
fn one_gesture_on_several_conversations_is_one_entry() {
    // Archiving three picked conversations is one thing done, and one undo takes all three back.
    let mut stack = UndoStack::default();
    let account = new_account_id();
    let entry = |thread| Undo {
        thread: Some(thread),
        account: account.clone(),
        forward: patch(vec![]),
        inverse: patch(vec![]),
        remote: None,
        resend: None,
    };
    let before = stack.push(entry(ThreadId::generate()));
    let threads: Vec<ThreadId> = (0..3).map(|_| ThreadId::generate()).collect();
    let batch = stack
        .push_all(threads.iter().copied().map(entry).collect())
        .expect("three undos are an entry");
    assert_eq!(stack.len(), 2, "one entry for the gesture, not three");
    let taken: Vec<Option<ThreadId>> = stack
        .take(batch)
        .expect("the batch is held")
        .iter()
        .map(|undo| undo.thread)
        .collect();
    assert_eq!(taken, threads.iter().copied().map(Some).collect::<Vec<_>>());
    assert!(stack.take(before).is_some());
    // A gesture that did nothing is not remembered.
    assert_eq!(stack.push_all(Vec::new()), None);
    assert!(stack.is_empty());
}

#[test]
fn mail_deleted_forever_is_forgotten_by_the_stack() {
    // A move to Trash of a message later deleted forever cannot be taken back: the message is
    // gone. Its part goes, an entry left with no part goes, and the rest stays as it was.
    let mut stack = UndoStack::default();
    let account = new_account_id();
    let (gone, kept) = (MessageId::generate(), MessageId::generate());
    let trashed = |m: MessageId| Undo {
        thread: Some(ThreadId::generate()),
        account: account.clone(),
        forward: patch(vec![Change::MessageMailbox(m, MailboxRole::Trash)]),
        inverse: patch(vec![Change::MessageMailbox(m, MailboxRole::Inbox)]),
        remote: None,
        resend: None,
    };
    let alone = stack.push(trashed(gone));
    let both = stack
        .push_all(vec![trashed(gone), trashed(kept)])
        .expect("an entry");
    let other = stack.push(trashed(kept));
    assert_eq!(stack.len(), 3);

    stack.forget_messages(&[gone]);
    assert_eq!(stack.len(), 2, "the entry that was only about it is gone");
    assert!(stack.take(alone).is_none());
    let left = stack.take(both).expect("the shared entry stays");
    assert_eq!(left.len(), 1);
    assert_eq!(
        left[0].forward.changes,
        vec![Change::MessageMailbox(kept, MailboxRole::Trash)]
    );
    assert!(stack.take(other).is_some());
}
