//! `SqliteStore::remove_account`: the account's rows go, and with them the blobs only it used.
//! A blob another account still names, even only from inside its JSON, stays.

use chrono::{DateTime, TimeZone, Utc};
use mail_domain::id::account_id_from_uuid;
use mail_domain::*;
use mail_store::{Freed, SqliteStore, Store};
use porter_core::AccountId;

fn acct_gone() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000c1"))
}
fn acct_kept() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000c2"))
}

/// Large enough to be a file rather than a row.
const LARGE: usize = 40 * 1024;
const SMALL: &[u8] = b"a part only the removed account had";

fn at(n: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000 + n, 0).unwrap()
}

fn store() -> (SqliteStore, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::in_memory(dir.path()).unwrap();
    for (account, address) in [
        (acct_gone(), "gone@example.test"),
        (acct_kept(), "kept@example.test"),
    ] {
        store
            .connection()
            .execute(
                "INSERT INTO accounts (id, address, plan, created_at)
                 VALUES (?1, ?2, '{}', datetime('now'))",
                [account.to_string(), address.to_owned()],
            )
            .unwrap();
    }
    (store, dir)
}

fn put(store: &SqliteStore, bytes: &[u8]) -> BlobId {
    store.blobs().put(&store.connection(), bytes).unwrap()
}

fn kept(account: AccountId, key: &str, raw: BlobId, parts: &[BlobId]) -> Kept {
    let key = MessageKey::Rfc(key.to_owned());
    Kept {
        key: key.clone(),
        raw,
        message: Message {
            id: MessageId::generate(),
            thread: ThreadId::generate(),
            account,
            key,
            date: at(1),
            from: Address {
                name: None,
                email: "ada@example.test".into(),
            },
            reply_to: vec![],
            to: vec![],
            cc: vec![],
            bcc: vec![],
            subject: "Subject".to_owned(),
            in_reply_to: None,
            references: vec![],
            rfc_message_id: None,
            read: ReadState::Read,
            star: Star::Unstarred,
            mailbox: MailboxRole::Inbox,
            labels: vec![],
            body: Body::Present { text: None, raw },
            attachments: parts
                .iter()
                .map(|blob| Attachment {
                    name: "part.bin".to_owned(),
                    mime: "application/octet-stream".to_owned(),
                    size: 1,
                    content: PartContent::Held(*blob),
                    inline: Inline::Attached,
                })
                .collect(),
        },
        labels: vec![],
    }
}

fn is_blob(store: &SqliteStore, blob: BlobId) -> bool {
    store.blobs().get(&store.connection(), blob).is_ok()
}

fn files(dir: &std::path::Path) -> usize {
    let mut count = 0;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(at) = stack.pop() {
        for entry in std::fs::read_dir(&at).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_none() {
                count += 1;
            }
        }
    }
    count
}

#[test]
fn the_account_s_rows_and_the_blobs_only_it_used_go_and_shared_ones_stay() {
    let (store, dir) = store();
    let large = put(&store, &vec![7u8; LARGE]);
    let small = put(&store, SMALL);
    let shared = put(&store, b"forwarded to both accounts");
    let own = put(&store, b"the kept account's own message");
    store
        .import(
            acct_gone(),
            Import {
                messages: vec![
                    kept(acct_gone(), "g1@x", large, &[small]),
                    kept(acct_gone(), "g2@x", shared, &[]),
                ],
            },
        )
        .unwrap();
    // The kept account names the shared blob only from inside its attachments' JSON.
    store
        .import(
            acct_kept(),
            Import {
                messages: vec![kept(acct_kept(), "k1@x", own, &[shared])],
            },
        )
        .unwrap();
    assert_eq!(files(dir.path()), 1, "the large blob is not a file");

    let freed = store.remove_account(acct_gone()).unwrap();

    assert_eq!(
        freed,
        Some(Freed {
            blobs: 2,
            bytes: (LARGE + SMALL.len()) as u64,
        })
    );
    assert!(!is_blob(&store, large));
    assert!(!is_blob(&store, small));
    assert!(
        is_blob(&store, shared),
        "the kept account's attachment lost its bytes"
    );
    assert!(is_blob(&store, own));
    assert_eq!(
        files(dir.path()),
        0,
        "the large blob's file is still on disk"
    );
    let left: i64 = store
        .connection()
        .query_row(
            "SELECT count(*) FROM messages WHERE account = ?1",
            [acct_gone().to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(left, 0);
    assert_eq!(store.offline(acct_kept()).unwrap().messages, 1);
}

#[test]
fn an_account_that_is_not_there_changes_nothing() {
    let (store, _dir) = store();
    let own = put(&store, b"kept");
    store
        .import(
            acct_kept(),
            Import {
                messages: vec![kept(acct_kept(), "k1@x", own, &[])],
            },
        )
        .unwrap();
    store.remove_account(acct_gone()).unwrap();
    for _ in 0..2 {
        assert_eq!(store.remove_account(acct_gone()).unwrap(), None);
    }
    assert!(is_blob(&store, own));
    assert_eq!(store.offline(acct_kept()).unwrap().messages, 1);
}

/// A blob two rows of the removed account shared, and nothing else did, goes once.
#[test]
fn a_blob_the_account_named_twice_goes_once() {
    let (store, _dir) = store();
    let twice = put(&store, b"one attachment in two messages");
    let first = put(&store, b"first");
    let second = put(&store, b"second");
    store
        .import(
            acct_gone(),
            Import {
                messages: vec![
                    kept(acct_gone(), "a@x", first, &[twice]),
                    kept(acct_gone(), "b@x", second, &[twice]),
                ],
            },
        )
        .unwrap();
    let freed = store.remove_account(acct_gone()).unwrap().unwrap();
    assert_eq!(freed.blobs, 3);
    assert!(!is_blob(&store, twice));
}

/// The removed account's message and the kept account's draft hold the same bytes (`put` keeps
/// one blob per content): the draft's attachment keeps them. `drafts` is read by the scan; a
/// `LIKE '%_fts%'` filter, whose `_` is a wildcard, once skipped it as if it were search's.
#[test]
fn a_blob_a_kept_draft_attaches_stays() {
    let (store, _dir) = store();
    let identity = IdentityId::generate();
    store
        .connection()
        .execute(
            "INSERT INTO identities (id, account, from_name, from_email, is_default)
             VALUES (?1, ?2, NULL, 'kept@example.test', '\"default\"')",
            [identity.to_string(), acct_kept().to_string()],
        )
        .unwrap();
    let shared = put(&store, &vec![9u8; LARGE]);
    store
        .import(
            acct_gone(),
            Import {
                messages: vec![kept(acct_gone(), "g1@x", shared, &[])],
            },
        )
        .unwrap();
    let draft = Draft {
        id: DraftId::generate(),
        account: acct_kept(),
        identity,
        to: vec![],
        cc: vec![],
        bcc: vec![],
        subject: "with the same file".to_owned(),
        in_reply_to: None,
        forward_of: None,
        text: String::new(),
        html: None,
        attachments: vec![PendingAttachment {
            name: "report.pdf".to_owned(),
            mime: "application/pdf".to_owned(),
            blob: shared,
        }],
        receipt: ReceiptRequest::Unrequested,
        openpgp: OpenPgp::None,
        smime: Smime::None,
        state: SendState::Editing,
        updated: at(2),
    };
    store
        .apply(
            acct_kept(),
            &Patch {
                id: ChangeId::generate(),
                changes: vec![Change::DraftUpsert(Box::new(draft))],
            },
        )
        .unwrap();

    let freed = store.remove_account(acct_gone()).unwrap().unwrap();

    assert_eq!(freed.blobs, 0, "{freed:?}");
    assert!(
        is_blob(&store, shared),
        "the kept draft's attachment lost its bytes"
    );
}
