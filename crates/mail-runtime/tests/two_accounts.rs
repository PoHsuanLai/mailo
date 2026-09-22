//! Two accounts in one database, which is what this client is for and what nothing tested.
//!
//! Every other suite here creates exactly one account. The user this was written for has a
//! campus POP3 mailbox, a Gmail one and two Microsoft 365 tenants, so "two accounts" is the ordinary
//! case and not an edge one — and the things that can go wrong with it are the ones that only
//! appear with two: a thread merged across accounts, an operation reaching the wrong copy, a
//! label name meaning two different labels, a count that adds up the wrong mailboxes.

use chrono::{DateTime, TimeZone, Utc};
use mail_domain::*;
use mail_runtime::{Arrival, absorb};
use mail_store::{SqliteStore, Store};

const A: AccountId = AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));
const B: AccountId = AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a2"));

fn now() -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000, 0).unwrap()
}

fn store() -> (SqliteStore, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::in_memory(dir.path()).unwrap();
    for (id, address) in [(A, "me@example.edu"), (B, "me@gmail.test")] {
        store
            .connection()
            .execute(
                "INSERT INTO accounts (id, address, plan, created_at)
                 VALUES (?1, ?2, '{}', datetime('now'))",
                rusqlite::params![id.to_string(), address],
            )
            .unwrap();
    }
    (store, dir)
}

/// Deliver `raw` to `account`, the way a sync does.
fn deliver(store: &SqliteStore, account: AccountId, uidl: &str, raw: &str) {
    absorb(
        store,
        account,
        MailboxRef {
            account,
            path: "INBOX".to_owned(),
        },
        Some(SyncCursor::Pop),
        vec![Arrival {
            remote: RemoteRef::Pop {
                uidl: uidl.to_owned(),
            },
            raw: raw.replace('\n', "\r\n").into_bytes(),
        }],
        false,
        now(),
    )
    .unwrap();
}

fn query(filter: Filter) -> Query {
    Query {
        filter,
        sort: Sort {
            property: Property::Date,
            dir: SortDir::Desc,
        },
        page: PageReq {
            after: None,
            limit: 50,
        },
    }
}

fn listed(store: &SqliteStore, filter: Filter) -> Vec<ThreadSummary> {
    store.threads(&query(filter), now()).unwrap().items
}

const LIST_MAIL: &str = "From: list@example.test\n\
                         To: everyone@example.test\n\
                         Subject: the announcement\n\
                         Message-ID: <shared@example.test>\n\
                         \n\
                         sent to a list both addresses are on\n";

#[test]
fn the_inbox_with_no_account_clause_holds_both() {
    // The plan's claim: "a unified inbox is `Filter::All` with no `Account` clause". Asserted,
    // because the alternative — a list that silently shows one account — is the kind of thing
    // a user only notices when mail goes missing.
    let (store, _dir) = store();
    deliver(
        &store,
        A,
        "u1",
        "From: a@example.edu\nSubject: from campus\nMessage-ID: <n1@x.test>\n\nhi\n",
    );
    deliver(
        &store,
        B,
        "u2",
        "From: b@gmail.test\nSubject: from Gmail\nMessage-ID: <g1@x.test>\n\nhi\n",
    );

    let subjects: Vec<String> = listed(&store, Filter::InMailbox(MailboxRole::Inbox))
        .into_iter()
        .map(|t| t.subject)
        .collect();
    assert_eq!(subjects.len(), 2, "{subjects:?}");
    assert!(subjects.iter().any(|s| s == "from campus"));
    assert!(subjects.iter().any(|s| s == "from Gmail"));
}

#[test]
fn an_account_clause_narrows_to_that_account() {
    let (store, _dir) = store();
    deliver(
        &store,
        A,
        "u1",
        "From: a@example.edu\nSubject: from campus\nMessage-ID: <n1@x.test>\n\nhi\n",
    );
    deliver(
        &store,
        B,
        "u2",
        "From: b@gmail.test\nSubject: from Gmail\nMessage-ID: <g1@x.test>\n\nhi\n",
    );

    let only_a = listed(&store, Filter::Account(A));
    assert_eq!(only_a.len(), 1);
    assert_eq!(only_a[0].subject, "from campus");
    assert_eq!(only_a[0].account, A);
}

#[test]
fn the_same_message_on_two_accounts_is_two_threads() {
    // A mailing list both addresses are on, or an address that forwards to the other. Merging
    // them would make one thread whose messages live on two accounts — and then archiving it
    // would have to act on two servers with two credentials, which no operation here can do.
    let (store, _dir) = store();
    deliver(&store, A, "u1", LIST_MAIL);
    deliver(&store, B, "u2", LIST_MAIL);

    let both = listed(&store, Filter::All);
    assert_eq!(both.len(), 2, "the copies merged into one thread");
    assert_ne!(both[0].id, both[1].id);
    let mut accounts: Vec<AccountId> = both.iter().map(|t| t.account).collect();
    accounts.sort();
    assert_eq!(accounts, vec![A.min(B), A.max(B)], "one thread each");
}

#[test]
fn a_reply_on_one_account_does_not_join_the_other_accounts_thread() {
    // The same test one step further on: threading looks up `In-Reply-To` scoped by account, so
    // a reply arriving on B must attach to B's copy and not to A's.
    let (store, _dir) = store();
    deliver(&store, A, "u1", LIST_MAIL);
    deliver(&store, B, "u2", LIST_MAIL);
    deliver(
        &store,
        B,
        "u3",
        "From: someone@example.test\n\
         Subject: Re: the announcement\n\
         Message-ID: <reply@example.test>\n\
         In-Reply-To: <shared@example.test>\n\
         References: <shared@example.test>\n\
         \n\
         replying\n",
    );

    let a_side = listed(&store, Filter::Account(A));
    let b_side = listed(&store, Filter::Account(B));
    assert_eq!(a_side.len(), 1);
    assert_eq!(
        a_side[0].message_count, 1,
        "the reply joined the wrong account"
    );
    assert_eq!(b_side.len(), 1);
    assert_eq!(b_side[0].message_count, 2);
}

#[test]
fn archiving_one_accounts_copy_leaves_the_others_alone() {
    let (store, _dir) = store();
    deliver(&store, A, "u1", LIST_MAIL);
    deliver(&store, B, "u2", LIST_MAIL);
    let a_thread = listed(&store, Filter::Account(A))[0].id;

    let loaded = store.thread(a_thread).unwrap();
    let messages: Vec<Message> = loaded
        .messages
        .iter()
        .filter_map(|id| store.message(*id).ok())
        .collect();
    let applied = Op::Archive.apply(
        &Target::Threads(vec![a_thread]),
        &loaded,
        &messages,
        &caps(),
        now(),
    );
    store.apply(A, &applied.forward).unwrap();

    assert!(
        listed(
            &store,
            Filter::And(vec![
                Filter::Account(A),
                Filter::InMailbox(MailboxRole::Inbox)
            ])
        )
        .is_empty(),
        "A's copy was not archived"
    );
    assert_eq!(
        listed(
            &store,
            Filter::And(vec![
                Filter::Account(B),
                Filter::InMailbox(MailboxRole::Inbox)
            ])
        )
        .len(),
        1,
        "B's copy was archived too"
    );
}

#[test]
fn one_label_name_on_two_accounts_is_two_labels() {
    // `UNIQUE (account, name)`, so "travel" on the Gmail account and "travel" on the campus one are
    // different labels — and a message on one must not acquire the other's.
    let (store, _dir) = store();
    for account in [A, B] {
        store
            .apply(
                account,
                &Patch {
                    id: ChangeId::generate(),
                    changes: vec![Change::LabelUpsert(Label {
                        id: LabelId::generate(),
                        account,
                        name: "travel".to_owned(),
                        color: None,
                        origin: LabelOrigin::Provider,
                    })],
                },
            )
            .unwrap();
    }
    let a_labels = store.labels(A).unwrap();
    let b_labels = store.labels(B).unwrap();
    assert_eq!(a_labels.len(), 1);
    assert_eq!(b_labels.len(), 1);
    assert_ne!(
        a_labels[0].id, b_labels[0].id,
        "one label serving two accounts"
    );
}

#[test]
fn an_unread_count_counts_one_account_or_both_as_asked() {
    let (store, _dir) = store();
    deliver(
        &store,
        A,
        "u1",
        "From: a@example.edu\nSubject: one\nMessage-ID: <n1@x.test>\n\nhi\n",
    );
    deliver(
        &store,
        B,
        "u2",
        "From: b@gmail.test\nSubject: two\nMessage-ID: <g1@x.test>\n\nhi\n",
    );
    deliver(
        &store,
        B,
        "u3",
        "From: b@gmail.test\nSubject: three\nMessage-ID: <g2@x.test>\n\nhi\n",
    );

    let unread = |f: Filter| store.count(&f, now()).unwrap();
    assert_eq!(unread(Filter::Read(ReadState::Unread)), 3, "both accounts");
    assert_eq!(
        unread(Filter::And(vec![
            Filter::Account(B),
            Filter::Read(ReadState::Unread)
        ])),
        2
    );
}

fn caps() -> AccountCaps {
    AccountCaps {
        labels: ServerLabels::LocalOnly,
        threads: ServerThreads::Jwz,
        watch: WatchMode::Poll {
            every: std::time::Duration::from_secs(300),
        },
        archive: ArchiveMeans::LocalOnly,
        folders: FolderRoles::default(),
        condstore: Condstore::Absent,
        move_ext: MoveExt::Absent,
        expunge: ExpungeMeans::Forbidden,
        top: Supported::Absent,
        pipelining: Supported::Absent,
        connections: ConnectionBudget::default(),
        observed_at: now(),
    }
}
