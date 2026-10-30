//! Labels the server reports, landing where a search can find them.
//!
//! `Ingest.labels` upserts label *definitions*; `Ingest.label_names` says which message carries
//! which: the server's complete list per message, by name, which the store resolves to ids and
//! applies as a difference.

use crate::support::*;
use mail_domain::*;
use mail_store::{SqliteStore, Store};

fn gmail_ref(uid: u32) -> RemoteRef {
    RemoteRef::Imap {
        mailbox: "INBOX".to_owned(),
        uidvalidity: 1,
        uid,
    }
}

/// One message, already stored and mapped, as a survey would find it.
fn message(store: &SqliteStore, uid: u32) -> MessageId {
    let raw = store.blobs().put(b"raw").unwrap();
    let message = Message {
        id: MessageId::generate(),
        thread: ThreadId::generate(),
        date: at(0),
        ..crate::support::message(u128::from(uid))
    };
    let id = message.id;
    let mut batch = ingest("INBOX");
    batch.messages.push(Fetched {
        remote: gmail_ref(uid),
        key: message.key.clone(),
        raw,
        message,
    });
    store.ingest(acct_account(), batch).unwrap();
    id
}

/// A survey of the inbox reporting each message's complete list of labels.
fn label_sweep(store: &SqliteStore, rows: Vec<(RemoteRef, Vec<String>)>) -> Patch {
    let batch = Ingest {
        label_names: rows,
        ..ingest("INBOX")
    };
    store.ingest(acct_account(), batch).unwrap()
}

fn names_on(store: &SqliteStore, message: MessageId) -> Vec<String> {
    let held = store.message(message).unwrap().labels;
    let mut names: Vec<String> = store
        .labels(acct_account())
        .unwrap()
        .into_iter()
        .filter(|l| held.contains(&l.id))
        .map(|l| l.name)
        .collect();
    names.sort();
    names
}

#[test]
fn a_label_the_server_reports_is_created_attached_and_found() {
    let store = sqlite();
    let id = message(&store, 42);

    label_sweep(
        &store,
        vec![(gmail_ref(42), vec!["travel".to_owned(), "家人".to_owned()])],
    );

    assert_eq!(
        names_on(&store, id),
        vec!["travel".to_owned(), "家人".to_owned()]
    );
    // Created by the server, so the user may not rename or delete it here — which is what the
    // origin is for.
    let travel = store
        .labels(acct_account())
        .unwrap()
        .into_iter()
        .find(|l| l.name == "travel")
        .expect("the label exists");
    assert_eq!(travel.origin, LabelOrigin::Provider);

    // And the point of all of it: the message is findable by that label.
    let found = store
        .threads(&query(Filter::HasLabel(travel.id), 10), at(0))
        .unwrap();
    assert_eq!(found.items.len(), 1);
    assert_eq!(found.items[0].id, store.message(id).unwrap().thread);
}

#[test]
fn the_same_name_twice_is_the_same_label() {
    // Two messages, one label. `UNIQUE (account, name)` is what makes that safe, and a client
    // that made a new row per message would show the same label several times in a list of labels.
    let store = sqlite();
    let first = message(&store, 42);
    let second = message(&store, 43);

    label_sweep(
        &store,
        vec![
            (gmail_ref(42), vec!["travel".to_owned()]),
            (gmail_ref(43), vec!["travel".to_owned()]),
        ],
    );

    assert_eq!(store.labels(acct_account()).unwrap().len(), 1);
    assert_eq!(names_on(&store, first), vec!["travel".to_owned()]);
    assert_eq!(names_on(&store, second), vec!["travel".to_owned()]);
}

#[test]
fn a_label_the_server_no_longer_lists_is_removed() {
    // The list is complete, not additive. A client that only ever adds accumulates labels the
    // user deleted years ago, and no later sync ever takes them off.
    let store = sqlite();
    let id = message(&store, 42);
    label_sweep(
        &store,
        vec![(gmail_ref(42), vec!["travel".to_owned(), "work".to_owned()])],
    );
    assert_eq!(names_on(&store, id).len(), 2);

    label_sweep(&store, vec![(gmail_ref(42), vec!["work".to_owned()])]);

    assert_eq!(names_on(&store, id), vec!["work".to_owned()]);
    // The label itself survives — another message may still carry it, and the user may still
    // want to search for it.
    assert!(
        store
            .labels(acct_account())
            .unwrap()
            .iter()
            .any(|l| l.name == "travel")
    );
}

#[test]
fn an_unchanged_list_writes_nothing() {
    // Only the difference is applied. Writing every membership on every pass would fill the
    // change log with nothing.
    let store = sqlite();
    let id = message(&store, 42);
    let first = label_sweep(&store, vec![(gmail_ref(42), vec!["travel".to_owned()])]);
    assert!(
        first
            .changes
            .iter()
            .any(|c| matches!(c, Change::MessageLabel(_, _, Membership::In))),
        "the first pass attached it"
    );

    let again = label_sweep(&store, vec![(gmail_ref(42), vec!["travel".to_owned()])]);
    assert!(
        !again
            .changes
            .iter()
            .any(|c| matches!(c, Change::MessageLabel(..))),
        "the second pass wrote it again: {:?}",
        again.changes
    );
    assert_eq!(names_on(&store, id), vec!["travel".to_owned()]);
}

#[test]
fn a_message_the_survey_does_not_know_is_skipped_rather_than_failing() {
    // A label row for a message this client has never fetched. Normal: the survey sees every
    // uid, and the bodies arrive later.
    let store = sqlite();
    label_sweep(&store, vec![(gmail_ref(999), vec!["travel".to_owned()])]);
    assert!(
        store.labels(acct_account()).unwrap().is_empty(),
        "no label invented"
    );
}

#[test]
fn a_labels_chosen_colour_survives_a_sync_that_reports_it_again() {
    let store = sqlite();
    message(&store, 42);
    label_sweep(&store, vec![(gmail_ref(42), vec!["travel".to_owned()])]);
    let travel = store
        .labels(acct_account())
        .unwrap()
        .into_iter()
        .find(|l| l.name == "travel")
        .unwrap();

    // The person picks a colour, as the label editor writes it.
    let coloured = Label {
        color: Some("purple".to_owned()),
        ..travel.clone()
    };
    let write = |label: &Label| {
        store
            .apply(
                acct_account(),
                &Patch {
                    id: ChangeId::generate(),
                    changes: vec![Change::LabelUpsert(label.clone())],
                },
            )
            .unwrap();
    };
    let colour_now = || {
        store
            .labels(acct_account())
            .unwrap()
            .into_iter()
            .find(|l| l.id == travel.id)
            .unwrap()
            .color
    };
    write(&coloured);
    assert_eq!(colour_now().as_deref(), Some("purple"));

    // A later sync names the label again: it is found by its name, not written over.
    label_sweep(&store, vec![(gmail_ref(42), vec!["travel".to_owned()])]);
    assert_eq!(colour_now().as_deref(), Some("purple"), "after a sync");

    // "No colour" in the editor is an upsert of the label with none: the one way it is cleared.
    write(&travel);
    assert_eq!(colour_now(), None);
}
