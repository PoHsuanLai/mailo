//! A conversation's follow-up reminder, kept alike by both stores: the column, the change that
//! sets it, a summary rebuilt around it, the waiting list in the order it comes due, and the
//! reminder surviving a reopen.

use chrono::{DateTime, TimeZone, Utc};
use mail_domain::id::account_id_from_uuid;
use mail_domain::*;
use mail_store::{MemoryStore, SqliteStore, Store};
use porter_core::AccountId;

fn acct_account() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"))
}

fn at(secs: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000 + secs, 0).unwrap()
}

fn thread(n: u128) -> ThreadId {
    ThreadId::from_uuid(uuid::Uuid::from_u128(0x7000 + n))
}

fn message(n: u128, thread: ThreadId, date: i64) -> Message {
    Message {
        id: MessageId::from_uuid(uuid::Uuid::from_u128(0x9000 + n)),
        thread,
        account: acct_account(),
        key: MessageKey::Rfc(format!("m{n}@example.test")),
        date: at(date),
        from: Address {
            name: None,
            email: "me@example.test".to_owned(),
        },
        reply_to: vec![],
        to: vec![Address {
            name: None,
            email: "ada@example.test".to_owned(),
        }],
        cc: vec![],
        bcc: vec![],
        subject: format!("subject {n}"),
        in_reply_to: None,
        references: vec![],
        rfc_message_id: Some(format!("m{n}@example.test")),
        read: ReadState::Read,
        star: Star::Unstarred,
        mailbox: MailboxRole::Sent,
        labels: vec![],
        body: Body::Absent,
        attachments: vec![],
    }
}

fn patch(changes: Vec<Change>) -> Patch {
    Patch {
        id: ChangeId::generate(),
        changes,
    }
}

/// Both stores over the same four conversations, one message each.
fn both(dir: &std::path::Path) -> (SqliteStore, MemoryStore) {
    let sqlite = SqliteStore::open(dir.join("mail.db"), dir.join("blobs")).unwrap();
    sqlite
        .raw_connection()
        .execute(
            "INSERT INTO accounts (id, address, plan, created_at)
             VALUES (?1, 'me@example.test', '{}', datetime('now'))",
            [acct_account().to_string()],
        )
        .unwrap();
    let memory = MemoryStore::new();
    for n in 1..=4 {
        let upsert = patch(vec![Change::MessageUpsert(Box::new(message(
            n,
            thread(n),
            i64::try_from(n).unwrap() * 10,
        )))]);
        sqlite.apply(acct_account(), &upsert).unwrap();
        memory.apply(acct_account(), &upsert).unwrap();
    }
    (sqlite, memory)
}

fn waiting(due: i64) -> FollowUp {
    FollowUp::Until {
        at: at(due),
        set: at(50),
    }
}

fn returned(due: i64) -> FollowUp {
    FollowUp::Returned {
        at: at(due),
        set: at(50),
    }
}

/// Each step's changes, then what the waiting list must say afterwards, by thread number.
#[test]
fn both_stores_keep_the_reminder_and_list_the_waiting_in_due_order() {
    let dir = tempfile::tempdir().unwrap();
    let (sqlite, memory) = both(dir.path());
    let stores: [(&str, &dyn Store); 2] = [("sqlite", &sqlite), ("memory", &memory)];

    // (name, changes, the waiting list after it, by thread number)
    let steps: Vec<(&str, Vec<Change>, Vec<u128>)> = vec![
        ("nothing set yet", vec![], vec![]),
        (
            "two reminders, the later set first",
            vec![
                Change::ThreadFollowUp(thread(2), waiting(900)),
                Change::ThreadFollowUp(thread(3), waiting(500)),
            ],
            vec![3, 2],
        ),
        (
            "one comes back: still listed, at its own time",
            vec![Change::ThreadFollowUp(thread(3), returned(500))],
            vec![3, 2],
        ),
        (
            "a third, due between them",
            vec![Change::ThreadFollowUp(thread(1), waiting(700))],
            vec![3, 1, 2],
        ),
        (
            "a new message in a waiting conversation rebuilds its summary around the reminder",
            vec![Change::MessageUpsert(Box::new(message(5, thread(1), 60)))],
            vec![3, 1, 2],
        ),
        (
            "a reply clears one",
            vec![Change::ThreadFollowUp(thread(3), FollowUp::Inactive)],
            vec![1, 2],
        ),
        (
            "two due at once are in thread order",
            vec![Change::ThreadFollowUp(thread(4), waiting(700))],
            vec![1, 4, 2],
        ),
    ];

    for (name, changes, expected) in steps {
        let step = patch(changes);
        for (store_name, store) in stores {
            store.apply(acct_account(), &step).unwrap();
            let listed: Vec<ThreadId> = store
                .follow_ups()
                .unwrap()
                .into_iter()
                .map(|summary| summary.id)
                .collect();
            let want: Vec<ThreadId> = expected.iter().map(|n| thread(*n)).collect();
            assert_eq!(listed, want, "{store_name}: {name}");
        }
        // Every conversation reads back the same from both, reminder included.
        for n in 1..=4 {
            let a = sqlite.thread(thread(n)).unwrap().summary;
            let b = memory.thread(thread(n)).unwrap().summary;
            assert_eq!(a.follow_up, b.follow_up, "{name}: thread {n}");
            assert_eq!(a, b, "{name}: thread {n}, whole summary");
        }
    }
    assert_eq!(
        sqlite.thread(thread(1)).unwrap().summary.follow_up,
        waiting(700),
        "the summary rebuilt by a new message kept its reminder"
    );

    // Kept across a reopen, in the thread row and in the cached summary.
    drop(sqlite);
    let reopened = SqliteStore::open(dir.path().join("mail.db"), dir.path().join("blobs")).unwrap();
    assert_eq!(
        reopened.thread(thread(4)).unwrap().summary.follow_up,
        waiting(700)
    );
    let row: String = reopened
        .raw_connection()
        .query_row(
            "SELECT follow_up FROM threads WHERE id = ?1",
            [thread(4).to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<FollowUp>(&row).unwrap(),
        waiting(700),
        "the column holds serde(FollowUp)"
    );
}

/// A patch and its undo, through each store: the reminder goes back to exactly what it was.
#[test]
fn an_undone_reminder_is_what_it_was_in_both_stores() {
    let dir = tempfile::tempdir().unwrap();
    let (sqlite, memory) = both(dir.path());
    for store in [&sqlite as &dyn Store, &memory] {
        let loaded = store.thread(thread(2)).unwrap();
        let messages: Vec<Message> = loaded
            .messages
            .iter()
            .map(|id| store.message(*id).unwrap())
            .collect();
        let caps = AccountCaps {
            labels: ServerLabels::LocalOnly,
            threads: ServerThreads::Jwz,
            watch: WatchMode::Idle,
            archive: ArchiveMeans::LocalOnly,
            folders: FolderRoles::default(),
            condstore: Condstore::Absent,
            move_ext: MoveExt::Absent,
            expunge: ExpungeMeans::Forbidden,
            top: Supported::Absent,
            pipelining: Supported::Absent,
            connections: ConnectionBudget::default(),
            observed_at: at(0),
        };
        let applied = Op::SetFollowUp(waiting(900)).apply(
            &Target::Threads(vec![thread(2)]),
            &loaded,
            &messages,
            &caps,
            at(60),
        );
        store.apply(acct_account(), &applied.forward).unwrap();
        assert_eq!(
            store.thread(thread(2)).unwrap().summary.follow_up,
            waiting(900)
        );
        assert_eq!(store.follow_ups().unwrap().len(), 1);
        store.apply(acct_account(), &applied.inverse).unwrap();
        assert_eq!(
            store.thread(thread(2)).unwrap().summary.follow_up,
            FollowUp::Inactive
        );
        assert!(store.follow_ups().unwrap().is_empty());
    }
}
