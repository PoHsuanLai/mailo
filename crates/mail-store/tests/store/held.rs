//! Accounts whose sign-in mailo holds itself, set aside while linked to accountd: out of every
//! account list and every listing and count, and never changed.

use chrono::{DateTime, TimeZone, Utc};
use mail_domain::id::account_id_from_uuid;
use mail_domain::*;
use mail_store::{SqliteStore, Store};
use porter_core::AccountId;

fn account(n: u128) -> AccountId {
    account_id_from_uuid(uuid::Uuid::from_u128(n))
}

fn now() -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000, 0).unwrap()
}

const PASSWORD: &str = r#"{"auth":{"kind":"password"},"incoming":{"kind":"imap"}}"#;
const OAUTH: &str = r#"{"auth":{"kind":"o_auth"},"incoming":{"kind":"imap"}}"#;
const GRANTED: &str = r#"{"auth":{"kind":"granted"},"incoming":{"kind":"imap"}}"#;
const LOCAL: &str = r#"{"auth":{"kind":"password"},"incoming":{"kind":"local"}}"#;

fn addresses(store: &SqliteStore) -> Vec<String> {
    let db = store.connection();
    let mut stmt = db
        .prepare(&format!(
            "SELECT address FROM {} ORDER BY address",
            store.accounts()
        ))
        .unwrap();
    stmt.query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn message(store: &SqliteStore, account: AccountId, uid: u32) {
    let raw = store.blobs().put(&store.connection(), b"raw").unwrap();
    let message = Message {
        id: MessageId::generate(),
        thread: ThreadId::generate(),
        account: account.clone(),
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
            account.clone(),
            Ingest {
                mailbox: MailboxRef {
                    account,
                    path: "INBOX".to_owned(),
                },
                validity: UidValidity::Same,
                cursor: None,
                messages: vec![Fetched {
                    remote: RemoteRef::Imap {
                        mailbox: "INBOX".to_owned(),
                        uidvalidity: 1,
                        uid,
                    },
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
}

fn store() -> (SqliteStore, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::in_memory(dir.path()).unwrap();
    for (n, address, plan) in [
        (1, "held-password@example.test", PASSWORD),
        (2, "held-oauth@example.test", OAUTH),
        (3, "granted@example.test", GRANTED),
        (4, "local@example.test", LOCAL),
        (5, "unreadable@example.test", "not json"),
    ] {
        store
            .connection()
            .execute(
                "INSERT INTO accounts (id, address, plan, created_at)
                 VALUES (?1, ?2, ?3, datetime('now'))",
                rusqlite::params![account(n).to_string(), address, plan],
            )
            .unwrap();
    }
    (store, dir)
}

#[test]
fn granted_only_leaves_held_accounts_out_and_writes_nothing() {
    let (store, _dir) = store();
    assert!(!store.granted_only(), "by default: not granted-only");
    assert_eq!(
        addresses(&store).len(),
        5,
        "by default: every account is listed"
    );

    // Set aside, only accounts signed in by mailo are left out.
    store.set_granted_only(true);
    assert_eq!(
        addresses(&store),
        [
            "granted@example.test",
            "local@example.test",
            "unreadable@example.test"
        ],
        "granted-only: held accounts are left out"
    );
    // Held ones are known, whatever the mode, and still stored as they were.
    assert_eq!(store.held_accounts(), [account(1), account(2)]);
    let plan: String = store
        .connection()
        .query_row(
            "SELECT plan FROM accounts WHERE address = 'held-password@example.test'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(plan, PASSWORD, "granted-only: nothing is written");

    // Taken back, they are all there again.
    store.set_granted_only(false);
    assert_eq!(addresses(&store).len(), 5, "taken back: all listed again");
}

#[test]
fn set_aside_their_mail_is_in_no_listing_and_no_count_and_is_still_stored() {
    let (store, _dir) = store();
    message(&store, account(1), 1);
    message(&store, account(3), 2);

    let all = || Query {
        filter: Filter::All,
        sort: Sort {
            property: Property::Date,
            dir: SortDir::Desc,
        },
        page: PageReq {
            after: None,
            limit: 100,
        },
    };
    assert_eq!(store.threads(&all(), now()).unwrap().items.len(), 2);
    assert_eq!(store.count(&Filter::All, now()).unwrap(), 2);

    store.set_granted_only(true);
    let listed = store.threads(&all(), now()).unwrap().items;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].account, account(3));
    assert_eq!(store.count(&Filter::All, now()).unwrap(), 1);

    // Not deleted: the mail is back when the mode is off.
    store.set_granted_only(false);
    assert_eq!(store.count(&Filter::All, now()).unwrap(), 2);
}
