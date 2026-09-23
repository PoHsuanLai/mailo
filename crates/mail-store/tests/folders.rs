//! Folders in the store: the listing, what a rename and a delete do to everything keyed by a
//! mailbox's name, and the outbox's part in both.
//!
//! Every scenario runs against the SQLite store and the in-memory one and compares what each
//! answers — the parity the rest of the store is held to, for the three new methods.

use chrono::{DateTime, TimeZone, Utc};
use mail_domain::*;
use mail_store::{MemoryStore, Settle, SqliteStore, Store};

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));

fn at(n: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000 + n, 0).unwrap()
}

fn sqlite() -> (SqliteStore, tempfile::TempDir) {
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

/// Run `scenario` on both stores and hand back what each produced.
fn both<T>(scenario: impl Fn(&dyn Store) -> T) -> (T, T) {
    let (sqlite, _dir) = sqlite();
    let memory = MemoryStore::new();
    (scenario(&sqlite), scenario(&memory))
}

fn folder(path: &str) -> Folder {
    Folder {
        account: ACCOUNT,
        path: path.to_owned(),
        delimiter: Some('/'),
        special: None,
        subscription: Subscription::Subscribed,
        holds: Holds::Mail,
    }
}

fn mailbox(path: &str) -> MailboxRef {
    MailboxRef {
        account: ACCOUNT,
        path: path.to_owned(),
    }
}

fn imap(mailbox: &str, uid: u32) -> RemoteRef {
    RemoteRef::Imap {
        mailbox: mailbox.to_owned(),
        uidvalidity: 7,
        uid,
    }
}

fn message(n: u128) -> Message {
    Message {
        id: MessageId::from_uuid(uuid::Uuid::from_u128(n)),
        thread: ThreadId::from_uuid(uuid::Uuid::from_u128(n + 1000)),
        account: ACCOUNT,
        key: MessageKey::Rfc(format!("m{n}@example.test")),
        date: at(n as i64),
        from: Address {
            name: None,
            email: "ada@example.test".to_owned(),
        },
        reply_to: vec![],
        to: vec![],
        cc: vec![],
        bcc: vec![],
        subject: format!("message {n}"),
        in_reply_to: None,
        references: vec![],
        rfc_message_id: None,
        read: ReadState::Unread,
        star: Star::Unstarred,
        mailbox: MailboxRole::Inbox,
        labels: vec![],
        body: Body::Absent,
        attachments: vec![],
    }
}

/// Deliver `message` at each of `remotes`, as a survey and a header fetch would, with the
/// server's labels for it.
fn deliver(store: &dyn Store, m: &Message, remotes: &[RemoteRef], labels: &[&str]) {
    for remote in remotes {
        let RemoteRef::Imap { mailbox: path, .. } = remote else {
            unreachable!("IMAP only here")
        };
        store
            .ingest(
                ACCOUNT,
                Ingest {
                    mailbox: mailbox(path),
                    validity: UidValidity::Same,
                    cursor: Some(SyncCursor::Imap {
                        uidvalidity: 7,
                        uidnext: 100,
                        modseq: None,
                    }),
                    messages: vec![Fetched {
                        remote: remote.clone(),
                        key: m.key.clone(),
                        raw: BlobId::generate(),
                        message: m.clone(),
                    }],
                    flags: vec![],
                    labels: vec![],
                    label_names: vec![],
                    gone: vec![],
                },
            )
            .unwrap();
    }
    // Through a patch rather than `label_names`, which only the SQLite store reads: this is
    // about what a folder change does to labels, not about how a survey delivers them.
    for name in labels {
        let label = provider_label(name);
        store
            .apply(
                ACCOUNT,
                &patch(vec![
                    Change::LabelUpsert(label.clone()),
                    Change::MessageLabel(m.id, label.id, Membership::In),
                ]),
            )
            .unwrap();
    }
}

/// The server's label called `name`, with an id both stores agree on.
fn provider_label(name: &str) -> Label {
    let n = name
        .bytes()
        .fold(0u128, |h, b| h.wrapping_mul(31).wrapping_add(u128::from(b)));
    Label {
        id: LabelId::from_uuid(uuid::Uuid::from_u128(n)),
        account: ACCOUNT,
        name: name.to_owned(),
        color: None,
        origin: LabelOrigin::Provider,
    }
}

fn paths(store: &dyn Store) -> Vec<String> {
    store
        .folders(ACCOUNT)
        .unwrap()
        .into_iter()
        .map(|f| f.path)
        .collect()
}

/// Who carries the server's label called `name`.
fn carrying(store: &dyn Store, name: &str) -> Vec<MessageId> {
    store.folder_contents(&mailbox(name)).unwrap().labelled
}

#[test]
fn a_listing_is_stored_and_read_back_in_path_order() {
    let (sql, mem) = both(|store| {
        store
            .put_folders(
                ACCOUNT,
                vec![folder("Work"), folder("INBOX"), folder("日本語")],
            )
            .unwrap();
        store.folders(ACCOUNT).unwrap()
    });
    assert_eq!(sql, mem);
    let paths: Vec<&str> = sql.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, vec!["INBOX", "Work", "日本語"]);
}

/// A new listing replaces the old one, except where the outbox still says otherwise.
#[test]
fn a_listing_that_predates_queued_work_does_not_undo_it() {
    let (sql, mem) = both(|store| {
        store
            .put_folders(ACCOUNT, vec![folder("INBOX"), folder("Old")])
            .unwrap();
        let create = FolderWork::Create {
            path: "New".to_owned(),
        };
        store
            .apply(ACCOUNT, &patch(vec![Change::FolderUpsert(folder("New"))]))
            .unwrap();
        store
            .enqueue(ACCOUNT, RemoteIntent::Folder(create), &patch(vec![]), at(0))
            .unwrap()
            .expect("folder work is always queued");
        // The server has not heard yet, and lists what it had — plus something made elsewhere.
        store
            .put_folders(ACCOUNT, vec![folder("INBOX"), folder("Old"), folder("Web")])
            .unwrap();
        paths(store)
    });
    assert_eq!(sql, mem);
    assert_eq!(sql, vec!["INBOX", "New", "Old", "Web"]);
}

fn patch(changes: Vec<Change>) -> Patch {
    Patch {
        id: ChangeId::generate(),
        changes,
    }
}

/// Renaming a folder renames everything beneath it, every address and cursor filed under those
/// names, and the server's labels of the same names — and nothing that merely shares a prefix.
#[test]
fn a_rename_moves_everything_keyed_by_the_old_name() {
    let (sql, mem) = both(|store| {
        let (a, b) = (message(1), message(2));
        deliver(store, &a, &[imap("INBOX", 1), imap("Work", 11)], &["Work"]);
        deliver(
            store,
            &b,
            &[imap("Work/2026", 12), imap("Workshop", 13)],
            &["Work/2026"],
        );
        store
            .put_folders(
                ACCOUNT,
                vec![
                    folder("INBOX"),
                    folder("Work"),
                    folder("Work/2026"),
                    folder("Workshop"),
                ],
            )
            .unwrap();

        store
            .apply(
                ACCOUNT,
                &patch(vec![Change::FolderRename {
                    from: mailbox("Work"),
                    to: "Jobs".to_owned(),
                    delimiter: Some('/'),
                }]),
            )
            .unwrap();

        let refs = |path: &str| store.remote_refs(&mailbox(path)).unwrap().len();
        let cursor = |path: &str| store.cursor(&mailbox(path)).unwrap().is_some();
        (
            paths(store),
            [
                refs("Jobs"),
                refs("Jobs/2026"),
                refs("Work"),
                refs("Work/2026"),
                refs("Workshop"),
            ],
            [
                cursor("Jobs"),
                cursor("Jobs/2026"),
                cursor("Work"),
                cursor("Workshop"),
            ],
            [
                carrying(store, "Jobs"),
                carrying(store, "Jobs/2026"),
                carrying(store, "Work"),
            ],
        )
    });
    assert_eq!(sql, mem);
    let (folders, refs, cursors, labels) = sql;
    assert_eq!(folders, vec!["INBOX", "Jobs", "Jobs/2026", "Workshop"]);
    assert_eq!(refs, [1, 1, 0, 0, 1]);
    assert_eq!(cursors, [true, true, false, true]);
    assert_eq!(labels, [vec![message(1).id], vec![message(2).id], vec![]]);
}

/// What a folder holds, by address and by label, counted the same way by both stores.
#[test]
fn folder_contents_are_what_is_addressed_there_and_what_carries_its_label() {
    let (sql, mem) = both(|store| {
        deliver(store, &message(1), &[imap("Receipts", 1)], &[]);
        deliver(store, &message(2), &[imap("INBOX", 2)], &["Receipts"]);
        deliver(store, &message(3), &[imap("INBOX", 3)], &["Other"]);
        store.folder_contents(&mailbox("Receipts")).unwrap()
    });
    assert_eq!(sql, mem);
    assert_eq!(sql.mapped, vec![message(1).id]);
    assert_eq!(sql.labelled, vec![message(2).id]);
    assert_eq!(sql.count(), 2);
}

/// Only once the server confirms a delete are the mailbox's addresses dropped — and with them
/// any message that was nowhere else. One also in the inbox stays.
#[test]
fn a_confirmed_delete_lets_go_of_the_mailbox_and_what_was_only_there() {
    let (sql, mem) = both(|store| {
        let (only, also) = (message(1), message(2));
        deliver(store, &only, &[imap("Old", 1)], &[]);
        deliver(store, &also, &[imap("Old", 2), imap("INBOX", 2)], &[]);
        store
            .put_folders(ACCOUNT, vec![folder("INBOX"), folder("Old")])
            .unwrap();
        let work = FolderWork::Delete {
            path: "Old".to_owned(),
            non_empty: NonEmpty::Allow,
        };
        store
            .apply(ACCOUNT, &patch(vec![Change::FolderRemove(mailbox("Old"))]))
            .unwrap();
        let queued = store
            .enqueue(
                ACCOUNT,
                RemoteIntent::Folder(work),
                &patch(vec![Change::FolderUpsert(folder("Old"))]),
                at(0),
            )
            .unwrap()
            .unwrap();

        // Queued, not yet confirmed: the addresses are still there, in case it is refused.
        let before = store.remote_refs(&mailbox("Old")).unwrap().len();
        store.outbox_settle(queued, Settle::Ok, at(1)).unwrap();
        (
            before,
            store.remote_refs(&mailbox("Old")).unwrap().len(),
            store.cursor(&mailbox("Old")).unwrap(),
            store.message(only.id).is_ok(),
            store.message(also.id).is_ok(),
            paths(store),
        )
    });
    assert_eq!(sql, mem);
    assert_eq!(sql, (2, 0, None, false, true, vec!["INBOX".to_owned()]));
}

/// The server refused for good: everything the plan changed here is put back.
#[test]
fn a_refused_create_is_taken_back() {
    let (sql, mem) = both(|store| {
        store.put_folders(ACCOUNT, vec![folder("INBOX")]).unwrap();
        store
            .apply(ACCOUNT, &patch(vec![Change::FolderUpsert(folder("New"))]))
            .unwrap();
        let queued = store
            .enqueue(
                ACCOUNT,
                RemoteIntent::Folder(FolderWork::Create {
                    path: "New".to_owned(),
                }),
                &patch(vec![Change::FolderRemove(mailbox("New"))]),
                at(0),
            )
            .unwrap()
            .unwrap();
        let before = paths(store);
        store
            .outbox_settle(
                queued,
                Settle::Failed {
                    reason: "NO [CANNOT] Invalid name".to_owned(),
                    retry: Retry::Fatal("NO".to_owned()),
                },
                at(1),
            )
            .unwrap();
        (
            before,
            paths(store),
            store.outbox_due(ACCOUNT, at(2)).unwrap().len(),
        )
    });
    assert_eq!(sql, mem);
    assert_eq!(
        sql,
        (vec!["INBOX".into(), "New".into()], vec!["INBOX".into()], 0)
    );
}

/// Deleting a label takes it off every message, and the SQLite store's thread summaries — the
/// cache the list is drawn from — follow.
#[test]
fn a_removed_label_leaves_no_trace_on_the_list() {
    let (sql, mem) = both(|store| {
        let m = message(1);
        deliver(store, &m, &[imap("INBOX", 1)], &["Receipts"]);
        let label = store.message(m.id).unwrap().labels[0];
        store
            .apply(ACCOUNT, &patch(vec![Change::LabelRemove(label)]))
            .unwrap();
        (
            store.message(m.id).unwrap().labels,
            store.thread(m.thread).unwrap().summary.labels,
        )
    });
    assert_eq!(sql, mem);
    assert_eq!(sql, (vec![], vec![]));
}
