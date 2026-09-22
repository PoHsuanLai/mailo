//! Labels the server reports, landing where a search can find them.
//!
//! `Ingest.labels` has upserted label *definitions* since phase 1 and nothing ever said which
//! message carried which. The half added here is `Ingest.label_names`: the server's complete
//! list per message, by name, which the store resolves to ids and applies as a difference.

use chrono::{DateTime, TimeZone, Utc};
use mail_domain::*;
use mail_store::{SqliteStore, Store};

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));

fn now() -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000, 0).unwrap()
}

fn gmail_ref(uid: u32) -> RemoteRef {
    RemoteRef::Imap {
        mailbox: "INBOX".to_owned(),
        uidvalidity: 1,
        uid,
    }
}

fn store() -> (SqliteStore, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::in_memory(dir.path()).unwrap();
    store
        .connection()
        .execute(
            "INSERT INTO accounts (id, address, plan, created_at)
             VALUES (?1, 'me@example.test', '{}', datetime('now'))",
            [ACCOUNT.to_string()],
        )
        .unwrap();
    (store, dir)
}

/// One message, already stored and mapped, as a survey would find it.
fn message(store: &SqliteStore, uid: u32) -> MessageId {
    let raw = store.blobs().put(&store.connection(), b"raw").unwrap();
    let id = MessageId::generate();
    let message = Message {
        id,
        thread: ThreadId::generate(),
        account: ACCOUNT,
        key: MessageKey::Rfc(format!("m{uid}@example.test")),
        date: now(),
        from: Address {
            name: None,
            email: "ada@example.test".to_owned(),
        },
        reply_to: vec![],
        to: vec![],
        cc: vec![],
        bcc: vec![],
        subject: format!("message {uid}"),
        in_reply_to: None,
        references: vec![],
        rfc_message_id: None,
        read: ReadState::Unread,
        star: Star::Unstarred,
        mailbox: MailboxRole::Inbox,
        labels: vec![],
        body: Body::Absent,
        attachments: vec![],
    };
    store
        .ingest(
            ACCOUNT,
            Ingest {
                mailbox: MailboxRef {
                    account: ACCOUNT,
                    path: "INBOX".to_owned(),
                },
                validity: UidValidity::Same,
                cursor: None,
                messages: vec![Fetched {
                    remote: gmail_ref(uid),
                    key: message.key.clone(),
                    raw,
                    message,
                }],
                flags: vec![],
                labels: vec![],
                label_names: vec![],
                gone: vec![],
            },
        )
        .unwrap();
    id
}

fn label_sweep(store: &SqliteStore, rows: Vec<(RemoteRef, Vec<String>)>) {
    store
        .ingest(
            ACCOUNT,
            Ingest {
                mailbox: MailboxRef {
                    account: ACCOUNT,
                    path: "INBOX".to_owned(),
                },
                validity: UidValidity::Same,
                cursor: None,
                messages: vec![],
                flags: vec![],
                labels: vec![],
                label_names: rows,
                gone: vec![],
            },
        )
        .unwrap();
}

fn names_on(store: &SqliteStore, message: MessageId) -> Vec<String> {
    let held = store.message(message).unwrap().labels;
    let mut names: Vec<String> = store
        .labels(ACCOUNT)
        .unwrap()
        .into_iter()
        .filter(|l| held.contains(&l.id))
        .map(|l| l.name)
        .collect();
    names.sort();
    names
}

#[test]
fn a_label_the_server_reports_is_created_and_attached() {
    let (store, _dir) = store();
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
        .labels(ACCOUNT)
        .unwrap()
        .into_iter()
        .find(|l| l.name == "travel")
        .expect("the label exists");
    assert_eq!(travel.origin, LabelOrigin::Provider);
}

#[test]
fn the_same_name_twice_is_the_same_label() {
    // Two messages, one label. `UNIQUE (account, name)` is what makes that safe, and a client
    // that made a new row per message would show the same label several times in a sidebar.
    let (store, _dir) = store();
    let first = message(&store, 42);
    let second = message(&store, 43);

    label_sweep(
        &store,
        vec![
            (gmail_ref(42), vec!["travel".to_owned()]),
            (gmail_ref(43), vec!["travel".to_owned()]),
        ],
    );

    assert_eq!(store.labels(ACCOUNT).unwrap().len(), 1);
    assert_eq!(names_on(&store, first), vec!["travel".to_owned()]);
    assert_eq!(names_on(&store, second), vec!["travel".to_owned()]);
}

#[test]
fn a_label_the_server_no_longer_lists_is_removed() {
    // The list is complete, not additive. A client that only ever adds accumulates labels the
    // user deleted years ago, and no later sync ever takes them off.
    let (store, _dir) = store();
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
            .labels(ACCOUNT)
            .unwrap()
            .iter()
            .any(|l| l.name == "travel")
    );
}

#[test]
fn an_unchanged_list_writes_nothing() {
    // Only the difference is applied. Writing every membership on every pass would fill the
    // change log with nothing, which is the same waste F95 and F113 were about.
    let (store, _dir) = store();
    let id = message(&store, 42);
    let first = label_patch(&store, vec![(gmail_ref(42), vec!["travel".to_owned()])]);
    assert!(
        first
            .changes
            .iter()
            .any(|c| matches!(c, Change::MessageLabel(_, _, Membership::In))),
        "the first pass attached it"
    );

    let again = label_patch(&store, vec![(gmail_ref(42), vec!["travel".to_owned()])]);
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
    let (store, _dir) = store();
    label_sweep(&store, vec![(gmail_ref(999), vec!["travel".to_owned()])]);
    assert!(
        store.labels(ACCOUNT).unwrap().is_empty(),
        "no label invented"
    );
}

#[test]
fn a_labelled_message_is_findable_by_that_label() {
    // The point of all of it.
    let (store, _dir) = store();
    let id = message(&store, 42);
    label_sweep(&store, vec![(gmail_ref(42), vec!["travel".to_owned()])]);

    let travel = store
        .labels(ACCOUNT)
        .unwrap()
        .into_iter()
        .find(|l| l.name == "travel")
        .unwrap();
    let found = store
        .threads(
            &Query {
                filter: Filter::HasLabel(travel.id),
                sort: Sort {
                    property: Property::Date,
                    dir: SortDir::Desc,
                },
                page: PageReq {
                    after: None,
                    limit: 10,
                },
            },
            now(),
        )
        .unwrap();
    assert_eq!(found.items.len(), 1);
    assert_eq!(found.items[0].id, store.message(id).unwrap().thread);
}

fn label_patch(store: &SqliteStore, rows: Vec<(RemoteRef, Vec<String>)>) -> Patch {
    store
        .ingest(
            ACCOUNT,
            Ingest {
                mailbox: MailboxRef {
                    account: ACCOUNT,
                    path: "INBOX".to_owned(),
                },
                validity: UidValidity::Same,
                cursor: None,
                messages: vec![],
                flags: vec![],
                labels: vec![],
                label_names: rows,
                gone: vec![],
            },
        )
        .unwrap()
}
