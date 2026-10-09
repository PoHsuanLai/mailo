//! What a search of the server leaves in the store: the messages it found, absorbed once however
//! often the search is repeated, marked as found there, and nothing it found already held marked.
//! Both stores, compared, with the numbers each must produce.

use chrono::{DateTime, TimeZone, Utc};
use mail_domain::id::account_id_from_uuid;
use mail_domain::*;
use mail_store::{MemoryStore, SqliteStore, Store, StoreError};
use porter_core::AccountId;

fn acct_account() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"))
}
fn acct_other() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a2"))
}

fn at(n: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000 + n, 0).unwrap()
}

/// Run `scenario` on both stores and hand back what each produced.
fn both<T>(scenario: impl Fn(&dyn Store, BlobId) -> T) -> (T, T) {
    let dir = tempfile::tempdir().unwrap();
    let sqlite = SqliteStore::in_memory(dir.path()).unwrap();
    for (account, address) in [
        (acct_account(), "me@example.test"),
        (acct_other(), "also@example.test"),
    ] {
        sqlite
            .connection()
            .execute(
                "INSERT INTO accounts (id, address, plan, created_at)
                 VALUES (?1, ?2, '{}', datetime('now'))",
                [account.to_string(), address.to_owned()],
            )
            .unwrap();
    }
    let raw = sqlite
        .blobs()
        .put(b"headers")
        .unwrap();
    let memory = MemoryStore::new();
    (scenario(&sqlite, raw), scenario(&memory, raw))
}

fn all_mail(uid: u32) -> RemoteRef {
    RemoteRef::Imap {
        mailbox: "[Gmail]/All Mail".to_owned(),
        uidvalidity: 11,
        uid,
    }
}

fn inbox(uid: u32) -> RemoteRef {
    RemoteRef::Imap {
        mailbox: "INBOX".to_owned(),
        uidvalidity: 7,
        uid,
    }
}

/// Message `n` as its headers build it, with a fresh id each time it is built: the store knows
/// it again by its key, as it knows a message fetched a second time.
fn headers(n: u128) -> Message {
    Message {
        id: MessageId::generate(),
        thread: ThreadId::from_uuid(uuid::Uuid::from_u128(n + 1000)),
        account: acct_account(),
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
        rfc_message_id: Some(format!("m{n}@example.test")),
        read: ReadState::Unread,
        star: Star::Unstarred,
        mailbox: MailboxRole::Archive,
        labels: vec![],
        body: Body::Absent,
        attachments: vec![],
    }
}

/// What a search's header fetch hands the store: `n` at `remote`. Returns the ids the patch says
/// are new, which is what the runtime marks.
fn absorb(store: &dyn Store, n: u128, remote: RemoteRef, raw: BlobId) -> Vec<MessageId> {
    let path = match &remote {
        RemoteRef::Imap { mailbox, .. } => mailbox.clone(),
        _ => unreachable!("IMAP only here"),
    };
    let message = headers(n);
    let patch = store
        .ingest(
            acct_account(),
            Ingest {
                mailbox: MailboxRef {
                    account: acct_account(),
                    path,
                },
                validity: UidValidity::Same,
                cursor: None,
                messages: vec![Fetched {
                    remote,
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
    patch
        .changes
        .iter()
        .filter_map(|c| match c {
            Change::MessageUpsert(m) => Some(m.id),
            _ => None,
        })
        .collect()
}

fn listed(store: &dyn Store) -> Vec<(String, ThreadId)> {
    let page = store
        .threads(
            &Query {
                filter: Filter::Account(acct_account()),
                sort: Sort {
                    property: Property::Date,
                    dir: SortDir::Desc,
                },
                page: PageReq {
                    after: None,
                    limit: 50,
                },
            },
            at(1_000),
        )
        .unwrap();
    page.items.into_iter().map(|t| (t.subject, t.id)).collect()
}

#[test]
fn a_hit_searched_for_twice_is_absorbed_and_marked_once() {
    let (sqlite, memory) = both(|store, raw| {
        // Message 1 was synced from the inbox; 2 is only on the server.
        absorb(store, 1, inbox(40), raw);
        let before = listed(store).len();

        // The search finds both in All Mail: nothing of either is held at those addresses.
        let hits = [all_mail(501), all_mail(502)];
        assert_eq!(store.held_at(acct_account(), &hits).unwrap(), vec![]);
        let mut new = absorb(store, 1, all_mail(501), raw);
        new.extend(absorb(store, 2, all_mail(502), raw));
        store.mark_found(acct_account(), &new, at(10)).unwrap();
        assert_eq!(new.len(), 1, "message 1 was already held: only 2 is new");
        assert_eq!(listed(store).len(), before + 1, "one conversation more");

        // Searched again: both are held at the addresses found, so nothing is fetched, and
        // absorbing them again (a race, or a caller that did not ask) adds nothing either.
        let held = store.held_at(acct_account(), &hits).unwrap();
        assert_eq!(held.len(), 2, "{held:?}");
        let mut again = absorb(store, 1, all_mail(501), raw);
        again.extend(absorb(store, 2, all_mail(502), raw));
        assert_eq!(again, vec![], "nothing is new the second time");
        store.mark_found(acct_account(), &new, at(20)).unwrap();
        assert_eq!(listed(store).len(), before + 1, "no duplicate conversation");

        let threads: Vec<ThreadId> = listed(store).into_iter().map(|(_, t)| t).collect();
        let marked: Vec<String> = store
            .found_in(&[threads.clone(), threads.clone()].concat())
            .unwrap()
            .into_iter()
            .map(|t| store.thread(t).unwrap().summary.subject)
            .collect();
        (held.len(), marked)
    });
    assert_eq!(sqlite, (2, vec!["message 2".to_owned()]));
    assert_eq!(sqlite, memory);
}

#[test]
fn held_at_names_only_this_accounts_addresses() {
    let (sqlite, memory) = both(|store, raw| {
        absorb(store, 3, inbox(9), raw);
        let mine = store
            .held_at(acct_account(), &[inbox(9), inbox(10)])
            .unwrap();
        let theirs = store.held_at(acct_other(), &[inbox(9)]).unwrap();
        (
            mine.into_iter().map(|(r, _)| r).collect::<Vec<_>>(),
            theirs.len(),
        )
    });
    assert_eq!(sqlite, (vec![inbox(9)], 0));
    assert_eq!(sqlite, memory);
}

#[test]
fn marking_a_message_not_held_marks_nothing() {
    let (sqlite, memory) = both(|store, raw| {
        let new = absorb(store, 4, all_mail(7), raw);
        let stranger = MessageId::from_uuid(uuid::Uuid::from_u128(99));
        let refused = matches!(
            store.mark_found(acct_account(), &[new[0], stranger], at(1)),
            Err(StoreError::NoMessage(id)) if id == stranger
        );
        let wrong_account = store.mark_found(acct_other(), &new, at(1)).is_err();
        let thread = store.message(new[0]).unwrap().thread;
        (refused, wrong_account, store.found_in(&[thread]).unwrap())
    });
    assert_eq!(sqlite, (true, true, vec![]));
    assert_eq!(sqlite, memory);
}

#[test]
fn the_mark_goes_with_the_message() {
    let (sqlite, memory) = both(|store, raw| {
        let new = absorb(store, 5, all_mail(8), raw);
        store.mark_found(acct_account(), &new, at(1)).unwrap();
        let thread = store.message(new[0]).unwrap().thread;
        let marked = store.found_in(&[thread]).unwrap().len();
        // The server says it is gone.
        store
            .ingest(
                acct_account(),
                Ingest {
                    mailbox: MailboxRef {
                        account: acct_account(),
                        path: "[Gmail]/All Mail".to_owned(),
                    },
                    validity: UidValidity::Same,
                    cursor: None,
                    messages: vec![],
                    flags: vec![],
                    labels: vec![],
                    label_names: vec![],
                    gone: vec![all_mail(8)],
                },
            )
            .unwrap();
        (marked, store.found_in(&[thread]).unwrap().len())
    });
    assert_eq!(sqlite, (1, 0));
    assert_eq!(sqlite, memory);
}
