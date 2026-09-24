//! `Store::import`: mail no server holds, kept once, with labels that only grow, and never a
//! `remote_map` row — on both stores, with the same answers.

use chrono::{DateTime, TimeZone, Utc};
use mail_domain::*;
use mail_store::{MemoryStore, SqliteStore, Store};

fn at(n: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000 + n, 0).unwrap()
}

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000b1"));

/// Stored bytes for message `n`, so the body's foreign key holds.
fn raw(store: &SqliteStore, n: i64) -> BlobId {
    store
        .blobs()
        .put(&store.connection(), format!("raw {n}").as_bytes())
        .unwrap()
}

fn sqlite() -> (SqliteStore, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::in_memory(dir.path()).unwrap();
    store
        .connection()
        .execute(
            "INSERT INTO accounts (id, address, plan, created_at)
             VALUES (?1, 'local folders', '{}', datetime('now'))",
            [ACCOUNT.to_string()],
        )
        .unwrap();
    (store, dir)
}

fn kept(key: &str, n: i64, raw: BlobId, labels: &[&str]) -> Kept {
    Kept {
        key: MessageKey::Rfc(key.to_owned()),
        raw,
        message: Message {
            id: MessageId::generate(),
            thread: ThreadId::generate(),
            account: ACCOUNT,
            key: MessageKey::Rfc(key.to_owned()),
            date: at(n),
            from: Address {
                name: None,
                email: "ada@example.test".into(),
            },
            reply_to: vec![],
            to: vec![],
            cc: vec![],
            bcc: vec![],
            subject: format!("Subject {n}"),
            in_reply_to: None,
            references: vec![],
            rfc_message_id: Some(key.to_owned()),
            read: ReadState::Read,
            star: Star::Unstarred,
            mailbox: MailboxRole::Archive,
            labels: vec![],
            body: Body::Present {
                text: Some(format!("body {n}")),
                raw,
            },
            attachments: vec![],
        },
        labels: labels.iter().map(|l| (*l).to_owned()).collect(),
    }
}

fn added(patch: &Patch) -> usize {
    patch
        .changes
        .iter()
        .filter(|c| matches!(c, Change::MessageUpsert(_)))
        .count()
}

/// What a store holds for the account, in a comparable form: subject, labels by name, and
/// whether the server knows of it.
fn held(store: &dyn Store) -> Vec<(String, usize, usize)> {
    let page = store
        .threads(
            &Query {
                filter: Filter::Account(ACCOUNT),
                sort: Sort {
                    property: Property::Date,
                    dir: SortDir::Asc,
                },
                page: PageReq {
                    after: None,
                    limit: 100,
                },
            },
            at(1000),
        )
        .unwrap();
    let mut out = Vec::new();
    for summary in page.items {
        for id in store.thread(summary.id).unwrap().messages {
            let message = store.message(id).unwrap();
            let remotes = store.remotes_of(id).unwrap().len();
            out.push((message.subject, message.labels.len(), remotes));
        }
    }
    out.sort();
    out
}

fn run(store: &dyn Store, raws: [BlobId; 2]) -> Vec<(String, usize, usize)> {
    let [ra, rb] = raws;
    let first = store
        .import(
            ACCOUNT,
            Import {
                messages: vec![kept("a@x", 1, ra, &["Receipts"]), kept("b@x", 2, rb, &[])],
            },
        )
        .unwrap();
    assert_eq!(added(&first), 2);

    // The same file again, where one copy of `a` now also sits in another folder.
    let second = store
        .import(
            ACCOUNT,
            Import {
                messages: vec![
                    kept("a@x", 1, ra, &["Receipts"]),
                    kept("a@x", 1, ra, &["Travel"]),
                    kept("b@x", 2, rb, &[]),
                ],
            },
        )
        .unwrap();
    assert_eq!(added(&second), 0, "nothing is kept twice");
    assert!(
        store
            .holds(ACCOUNT, &MessageKey::Rfc("a@x".into()))
            .unwrap()
    );
    assert!(
        !store
            .holds(ACCOUNT, &MessageKey::Rfc("c@x".into()))
            .unwrap()
    );
    held(store)
}

#[test]
fn importing_twice_keeps_each_message_once_and_labels_only_grow() {
    let (sqlite, _dir) = sqlite();
    let raws = [raw(&sqlite, 1), raw(&sqlite, 2)];
    let out = run(&sqlite, raws);
    assert_eq!(
        out,
        vec![
            ("Subject 1".to_owned(), 2, 0),
            ("Subject 2".to_owned(), 0, 0),
        ],
        "two labels on a, none on b, and no server address on either"
    );
}

#[test]
fn the_memory_store_imports_as_sqlite_does() {
    let (sqlite, _dir) = sqlite();
    let memory = MemoryStore::new();
    let raws = [raw(&sqlite, 1), raw(&sqlite, 2)];
    assert_eq!(run(&memory, raws), run(&sqlite, raws));
}

#[test]
fn an_imported_label_is_the_users_own() {
    let (sqlite, _dir) = sqlite();
    sqlite
        .import(
            ACCOUNT,
            Import {
                messages: vec![kept("a@x", 1, raw(&sqlite, 1), &["Receipts"])],
            },
        )
        .unwrap();
    let labels = sqlite.labels(ACCOUNT).unwrap();
    assert_eq!(labels.len(), 1);
    assert_eq!(labels[0].name, "Receipts");
    assert_eq!(labels[0].origin, LabelOrigin::User);
}

#[test]
fn nothing_done_to_imported_mail_is_queued_for_a_server() {
    let (sqlite, _dir) = sqlite();
    let patch = sqlite
        .import(
            ACCOUNT,
            Import {
                messages: vec![kept("a@x", 1, raw(&sqlite, 1), &[])],
            },
        )
        .unwrap();
    let Some(Change::MessageUpsert(message)) = patch.changes.first() else {
        panic!("{patch:?}");
    };
    let queued = sqlite
        .enqueue(
            ACCOUNT,
            RemoteIntent::SetFlags {
                messages: vec![message.id],
                read: Some(ReadState::Unread),
                star: None,
            },
            &Patch {
                id: ChangeId::generate(),
                changes: vec![],
            },
            at(5),
        )
        .unwrap();
    assert_eq!(queued, None);
}

#[test]
fn an_upload_resolves_to_the_same_append_on_both_stores() {
    let (sqlite, _dir) = sqlite();
    let memory = MemoryStore::new();
    memory.import(ACCOUNT, Import { messages: vec![] }).unwrap();
    let intent = RemoteIntent::Append {
        mailbox: MailboxRef {
            account: ACCOUNT,
            path: "Archive".into(),
        },
        flags: vec![SystemFlag::Seen],
        date: Some(at(3)),
        raw: BlobId::generate(),
    };
    let undo = Patch {
        id: ChangeId::generate(),
        changes: vec![],
    };
    for store in [&sqlite as &dyn Store, &memory] {
        store
            .enqueue(ACCOUNT, intent.clone(), &undo, at(4))
            .unwrap()
            .expect("an upload addresses no existing message and is always queued");
        let due = store.outbox_due(ACCOUNT, at(4)).unwrap();
        let RemoteIntent::Append {
            mailbox,
            flags,
            date,
            raw,
        } = intent.clone()
        else {
            unreachable!()
        };
        assert_eq!(
            due.last().map(|e| e.op.clone()),
            Some(ProtoOp::Append {
                mailbox,
                flags,
                date,
                raw
            })
        );
    }
}
