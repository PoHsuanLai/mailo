//! The launcher's unread count against a real store: what it counts, and that it moves when the
//! mail does. Nothing here reaches a desktop; the message the count becomes is a table in
//! `launcher/unity.rs`.

use chrono::{DateTime, TimeZone, Utc};
use mail_app::ui::launcher::{Unread, unread};
use mail_app::ui::space::Scope;
use mail_core::snooze;
use mail_domain::id::account_id_from_uuid;
use mail_domain::*;
use mail_runtime::{Arrival, absorb};
use mail_store::{SqliteStore, Store};
use porter_core::AccountId;

fn acct_work() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"))
}
fn acct_home() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000b2"))
}

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 27, 9, 0, 0).unwrap()
}

fn arrive(store: &SqliteStore, account: AccountId, id: &str, extra: &str) {
    let raw = format!(
        "From: ada@example.test\r\nSubject: about {id}\r\nMessage-ID: <{id}@example.test>\r\n\
         {extra}Date: Sat, 26 Sep 2026 12:00:00 +0000\r\n\r\nbody\r\n"
    );
    absorb(
        store,
        account.clone(),
        MailboxRef {
            account,
            path: "INBOX".to_owned(),
        },
        Some(SyncCursor::Pop),
        vec![Arrival {
            remote: RemoteRef::Pop {
                uidl: id.to_owned(),
            },
            raw: raw.into_bytes(),
        }],
        false,
        now(),
    )
    .unwrap();
}

/// Work has three unread conversations in its inbox; Home has one, of two unread messages.
fn seeded() -> (SqliteStore, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::in_memory(dir.path()).unwrap();
    for (account, address) in [(acct_work(), "me@work.test"), (acct_home(), "me@home.test")] {
        store
            .connection()
            .execute(
                "INSERT INTO accounts (id, address, plan, created_at)
                 VALUES (?1, ?2, '{}', datetime('now'))",
                [account.to_string(), address.to_owned()],
            )
            .unwrap();
    }
    for id in ["w1", "w2", "w3"] {
        arrive(&store, acct_work(), id, "");
    }
    arrive(&store, acct_home(), "h1", "");
    arrive(
        &store,
        acct_home(),
        "h2",
        "In-Reply-To: <h1@example.test>\r\nReferences: <h1@example.test>\r\n",
    );
    (store, dir)
}

/// The thread holding the message `<id@example.test>`, and that message.
fn find(store: &SqliteStore, id: &str) -> (ThreadId, MessageId) {
    let query = Query {
        filter: Filter::All,
        sort: Sort {
            property: Property::Date,
            dir: SortDir::Desc,
        },
        page: PageReq {
            after: None,
            limit: 50,
        },
    };
    for summary in store.threads(&query, now()).unwrap().items {
        for message in store.thread(summary.id).unwrap().messages {
            let held = store.message(message).unwrap();
            if held.rfc_message_id.as_deref() == Some(&format!("{id}@example.test")) {
                return (summary.id, message);
            }
        }
    }
    panic!("no message {id}");
}

fn change(store: &SqliteStore, account: AccountId, change: Change) {
    store
        .apply(
            account,
            &Patch {
                id: ChangeId::generate(),
                changes: vec![change],
            },
        )
        .unwrap();
}

/// The count over the accounts named.
fn count(store: &SqliteStore, scope: &[AccountId]) -> u64 {
    unread(store, &Scope::Accounts(scope.to_vec()), now())
        .unwrap()
        .0
}

#[test]
fn it_counts_unread_conversations_in_the_space_s_accounts() {
    let (store, _dir) = seeded();
    let cases: Vec<(&str, Vec<AccountId>, u64)> = vec![
        ("a Space of no account", vec![], 0),
        ("a Space of Work", vec![acct_work()], 3),
        (
            "a Space of Home: two unread messages, one conversation",
            vec![acct_home()],
            1,
        ),
        ("a Space naming both", vec![acct_work(), acct_home()], 4),
    ];
    for (name, scope, expect) in cases {
        assert_eq!(count(&store, &scope), expect, "{name}");
    }
    assert_eq!(
        unread(&store, &Scope::All, now()).unwrap(),
        Unread(4),
        "every account"
    );
}

#[test]
fn reading_snoozing_or_archiving_one_takes_it_off_the_count() {
    let (store, _dir) = seeded();
    let before = count(&store, &[acct_work()]);

    let (_, w1) = find(&store, "w1");
    change(
        &store,
        acct_work(),
        Change::MessageRead(w1, ReadState::Read),
    );
    assert_eq!(count(&store, &[acct_work()]), before - 1, "read");

    let (w2, _) = find(&store, "w2");
    snooze::snooze(&store, w2, "tomorrow", now()).unwrap();
    assert_eq!(count(&store, &[acct_work()]), before - 2, "snoozed");

    let (_, w3) = find(&store, "w3");
    change(
        &store,
        acct_work(),
        Change::MessageMailbox(w3, MailboxRole::Archive),
    );
    assert_eq!(count(&store, &[acct_work()]), before - 3, "archived");

    // Home was not touched, and one of its two unread messages read leaves its conversation
    // unread.
    let home = count(&store, &[acct_home()]);
    let (_, h2) = find(&store, "h2");
    change(
        &store,
        acct_home(),
        Change::MessageRead(h2, ReadState::Read),
    );
    assert_eq!(
        count(&store, &[acct_home()]),
        home,
        "half read is still unread"
    );

    // And back: the snooze wakes by itself, the unread count with it.
    let later = now() + chrono::TimeDelta::try_days(3).unwrap();
    assert_eq!(
        unread(&store, &Scope::Accounts(vec![acct_work()]), later).unwrap(),
        Unread(1)
    );
}
