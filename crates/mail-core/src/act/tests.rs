use super::*;
use crate::support_tests::{account, at, caps, message};
use mail_store::testing::{clear_outbox, in_memory, seed_account, seed_caps};

/// A store whose server files into named folders, holding one message in the inbox, and a label
/// the folder `Projects` is.
fn store_with_a_copy_target() -> (SqliteStore, ThreadId, LabelId) {
    let store = in_memory();
    seed_account(&store, account(), "me@example.test");
    let caps = AccountCaps {
        archive: ArchiveMeans::MoveToFolder("Archive".to_owned()),
        ..caps()
    };
    seed_caps(&store, account(), &caps, at(0)).unwrap();
    let mut held = message(1, "Hello", "Body", 0, ReadState::Read, Star::Unstarred);
    // The message's bytes are a row of their own that it refers to.
    let raw = store.blobs().put(b"Body").unwrap();
    if let mail_domain::Body::Present { raw: slot, .. } = &mut held.body {
        *slot = raw;
    }
    let thread = held.thread;
    let message_id = held.id;
    store
        .apply(
            account(),
            &Patch {
                id: ChangeId::generate(),
                changes: vec![Change::MessageUpsert(Box::new(held))],
            },
        )
        .unwrap();
    // The server holds it, so work on it is queued for the server: a message with no address
    // there has nothing to tell the server about.
    mail_store::testing::seed_remote_uid(&store, account(), "INBOX", 1, message_id);
    let projects = store.create_label(account(), "Projects").unwrap();
    (store, thread, projects)
}

fn copies_queued(store: &SqliteStore) -> usize {
    mail_store::testing::outbox_ops(store)
        .iter()
        .filter(|op| matches!(op, ProtoOp::Copy { .. }))
        .count()
}

/// A copy still waiting for the server is withdrawn with its undo; one the server has made stays
/// made, and an undo that would only put it back at the next sync is refused, with nothing
/// changed here.
#[test]
fn a_copy_is_taken_back_only_while_it_is_still_waiting_for_the_server() {
    let (store, thread, projects) = store_with_a_copy_target();
    let in_projects = |store: &SqliteStore| {
        let held = store.thread(thread).unwrap();
        messages_of(store, &held)
            .iter()
            .all(|m| m.labels.contains(&projects))
    };

    let waiting = perform(&store, thread, Op::Copy(projects)).expect("copied");
    assert_eq!(copies_queued(&store), 1);
    assert!(in_projects(&store));
    assert!(take_back(&store, &waiting), "refused while it was waiting");
    assert!(!in_projects(&store));
    assert_eq!(copies_queued(&store), 0, "still queued after its undo");

    let sent = perform(&store, thread, Op::Copy(projects)).expect("copied again");
    // The server took it: nothing waits.
    clear_outbox(&store);
    assert!(
        !take_back(&store, &sent),
        "taken back after the server made it"
    );
    assert!(in_projects(&store), "the copy was taken out of its folder");
}
