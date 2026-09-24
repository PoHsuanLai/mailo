//! A folder's mail, by where the server holds it: `Filter::InFolder`, the body backlog of one
//! mailbox, and what a message held in several mailboxes is filed as.
//!
//! Every scenario runs against the SQLite store and the in-memory one and compares what each
//! answers, the parity the rest of the store is held to.

use chrono::{DateTime, TimeZone, Utc};
use mail_domain::*;
use mail_store::{MemoryStore, SqlValue, SqliteStore, Store, compile};
use std::collections::BTreeSet;

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));

const PROJECTS: &str = "Projects/2026";

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

/// A message as a header fetch from a folder filed as `role` builds it: no body yet.
fn message(n: u128, role: MailboxRole) -> Message {
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
        mailbox: role,
        labels: vec![],
        body: Body::Absent,
        attachments: vec![],
    }
}

fn ingest(path: &str) -> Ingest {
    Ingest {
        mailbox: mailbox(path),
        validity: UidValidity::Same,
        cursor: None,
        messages: vec![],
        flags: vec![],
        labels: vec![],
        label_names: vec![],
        gone: vec![],
    }
}

/// Deliver `m` at `remote`, as a header fetch from that mailbox would.
fn deliver(store: &dyn Store, m: &Message, remote: RemoteRef) {
    let RemoteRef::Imap { mailbox: path, .. } = &remote else {
        unreachable!("IMAP only here")
    };
    let mut batch = ingest(path);
    batch.messages.push(Fetched {
        remote: remote.clone(),
        key: m.key.clone(),
        raw: BlobId::generate(),
        message: m.clone(),
    });
    store.ingest(ACCOUNT, batch).unwrap();
}

/// The server says `remote` has gone from its mailbox.
fn gone(store: &dyn Store, remote: RemoteRef) {
    let RemoteRef::Imap { mailbox: path, .. } = &remote else {
        unreachable!("IMAP only here")
    };
    let mut batch = ingest(path);
    batch.gone.push(remote);
    store.ingest(ACCOUNT, batch).unwrap();
}

fn listed(store: &dyn Store, filter: Filter) -> BTreeSet<ThreadId> {
    store
        .threads(
            &Query {
                filter,
                sort: Sort {
                    property: Property::Date,
                    dir: SortDir::Desc,
                },
                page: PageReq {
                    after: None,
                    limit: 100,
                },
            },
            at(0),
        )
        .unwrap()
        .items
        .into_iter()
        .map(|s| s.id)
        .collect()
}

fn thread(n: u128) -> ThreadId {
    ThreadId::from_uuid(uuid::Uuid::from_u128(n + 1000))
}

fn caps(folders: Vec<(&str, MailboxRole)>) -> AccountCaps {
    AccountCaps {
        labels: ServerLabels::LocalOnly,
        threads: ServerThreads::Jwz,
        watch: WatchMode::Idle,
        archive: ArchiveMeans::LocalOnly,
        folders: FolderRoles(
            folders
                .into_iter()
                .map(|(p, r)| (p.to_owned(), r))
                .collect(),
        ),
        condstore: Condstore::Absent,
        move_ext: MoveExt::Absent,
        expunge: ExpungeMeans::Forbidden,
        top: Supported::Absent,
        pipelining: Supported::Absent,
        connections: ConnectionBudget { max: 1 },
        observed_at: at(0),
    }
}

#[test]
fn a_folder_lists_the_threads_with_a_message_addressed_there() {
    let (sqlite, memory) = both(|store| {
        deliver(store, &message(1, MailboxRole::Inbox), imap("INBOX", 1));
        deliver(store, &message(2, MailboxRole::Archive), imap(PROJECTS, 1));
        // One message, a copy in each: one thread, listed in both.
        deliver(store, &message(3, MailboxRole::Inbox), imap("INBOX", 2));
        deliver(store, &message(3, MailboxRole::Archive), imap(PROJECTS, 2));
        let projects = Filter::InFolder(mailbox(PROJECTS));
        (
            listed(store, projects.clone()),
            listed(store, Filter::InFolder(mailbox("INBOX"))),
            listed(store, Filter::Not(Box::new(projects.clone()))),
            store.count(&projects, at(0)).unwrap(),
            listed(
                store,
                Filter::InFolder(MailboxRef {
                    account: AccountId::from_uuid(uuid::Uuid::from_u128(9)),
                    path: PROJECTS.to_owned(),
                }),
            ),
        )
    });
    assert_eq!(sqlite, memory);
    let (projects, inbox, not_projects, counted, elsewhere) = sqlite;
    assert_eq!(projects, BTreeSet::from([thread(2), thread(3)]));
    assert_eq!(inbox, BTreeSet::from([thread(1), thread(3)]));
    assert_eq!(not_projects, BTreeSet::from([thread(1)]));
    assert_eq!(counted, 2);
    assert!(elsewhere.is_empty(), "the path on another account matched");
}

#[test]
fn the_folder_clause_is_answered_from_an_index() {
    // Thousands of threads each probing `remote_map` would be a folder that takes seconds to
    // open. The clause reads the identity index on its leading `(account, mailbox)` columns,
    // once, and each message by its key.
    let (store, _dir) = sqlite();
    for n in 1..50 {
        deliver(
            &store,
            &message(n, MailboxRole::Inbox),
            imap("INBOX", n as u32),
        );
    }
    let compiled = compile(&Filter::InFolder(mailbox(PROJECTS)), at(0));
    let sql = format!(
        "EXPLAIN QUERY PLAN SELECT ts.thread FROM thread_summary ts WHERE {}",
        compiled.where_clause
    );
    let params: Vec<String> = compiled
        .params
        .iter()
        .map(|p| match p {
            SqlValue::Text(t) => t.clone(),
            SqlValue::Int(i) => i.to_string(),
        })
        .collect();
    let db = store.connection();
    let mut stmt = db.prepare(&sql).unwrap();
    let plan: Vec<String> = stmt
        .query_map(rusqlite::params_from_iter(params.iter()), |r| {
            r.get::<_, String>(3)
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let plan = plan.join("\n");
    assert!(
        plan.contains("remote_map_identity (account=? AND mailbox=?)"),
        "{plan}"
    );
    assert!(
        !plan.lines().any(|l| l.contains("SCAN r")),
        "remote_map is scanned: {plan}"
    );
    assert!(
        plan.lines()
            .any(|l| l.contains("SEARCH m USING") && l.contains("(id=?)")),
        "each message by its key: {plan}"
    );
}

#[test]
fn a_body_pass_over_one_mailbox_is_given_only_that_mailboxs_addresses() {
    let (sqlite, memory) = both(|store| {
        deliver(store, &message(1, MailboxRole::Inbox), imap("INBOX", 1));
        deliver(store, &message(2, MailboxRole::Archive), imap(PROJECTS, 1));
        deliver(store, &message(3, MailboxRole::Inbox), imap("INBOX", 2));
        deliver(store, &message(3, MailboxRole::Archive), imap(PROJECTS, 2));
        deliver(store, &message(4, MailboxRole::Archive), imap(PROJECTS, 3));
        (
            store.unfetched_in(&mailbox("INBOX"), 10).unwrap(),
            store.unfetched_in(&mailbox(PROJECTS), 10).unwrap(),
            store.unfetched_in(&mailbox(PROJECTS), 2).unwrap(),
            store.unfetched_in(&mailbox("Elsewhere"), 10).unwrap(),
        )
    });
    assert_eq!(sqlite, memory);
    let (inbox, projects, cut, elsewhere) = sqlite;
    // Newest first, each by its address in the mailbox asked about, never another's.
    assert_eq!(inbox, vec![imap("INBOX", 2), imap("INBOX", 1)]);
    assert_eq!(
        projects,
        vec![imap(PROJECTS, 3), imap(PROJECTS, 2), imap(PROJECTS, 1)]
    );
    assert_eq!(cut, vec![imap(PROJECTS, 3), imap(PROJECTS, 2)]);
    assert!(elsewhere.is_empty());
}

#[test]
fn a_copy_in_the_inbox_brings_a_message_first_found_in_a_folder_into_the_inbox() {
    // A large inbox's first sync can reach a folder's copy before the inbox's own.
    let (sqlite, memory) = both(|store| {
        deliver(store, &message(1, MailboxRole::Archive), imap(PROJECTS, 1));
        let before = store.message(MessageId::from_uuid(uuid::Uuid::from_u128(1)));
        deliver(store, &message(1, MailboxRole::Inbox), imap("INBOX", 9));
        let after = store.message(MessageId::from_uuid(uuid::Uuid::from_u128(1)));
        // A copy found later in a folder does not take it out again.
        deliver(store, &message(1, MailboxRole::Archive), imap("Other", 4));
        let still = store.message(MessageId::from_uuid(uuid::Uuid::from_u128(1)));
        (
            before.unwrap().mailbox,
            after.unwrap().mailbox,
            still.unwrap().mailbox,
        )
    });
    assert_eq!(sqlite, memory);
    assert_eq!(
        sqlite,
        (MailboxRole::Archive, MailboxRole::Inbox, MailboxRole::Inbox)
    );
}

#[test]
fn a_message_moved_out_of_the_inbox_elsewhere_is_filed_where_it_went() {
    let (sqlite, memory) = both(|store| {
        store
            .put_caps(ACCOUNT, &caps(vec![("Trash", MailboxRole::Trash)]), at(0))
            .unwrap();
        let id = |n: u128| MessageId::from_uuid(uuid::Uuid::from_u128(n));
        // Moved to the trash by another client: the trash copy arrived first.
        deliver(store, &message(1, MailboxRole::Inbox), imap("INBOX", 1));
        deliver(store, &message(1, MailboxRole::Trash), imap("Trash", 1));
        gone(store, imap("INBOX", 1));
        // Filed in a folder with no role.
        deliver(store, &message(2, MailboxRole::Inbox), imap("INBOX", 2));
        deliver(store, &message(2, MailboxRole::Archive), imap(PROJECTS, 2));
        gone(store, imap("INBOX", 2));
        // Leaving a folder is not leaving the inbox.
        deliver(store, &message(3, MailboxRole::Inbox), imap("INBOX", 3));
        deliver(store, &message(3, MailboxRole::Archive), imap(PROJECTS, 3));
        gone(store, imap(PROJECTS, 3));
        // Gone from its only mailbox: deleted, as before.
        deliver(store, &message(4, MailboxRole::Inbox), imap("INBOX", 4));
        gone(store, imap("INBOX", 4));
        (
            store.message(id(1)).unwrap().mailbox,
            store.message(id(2)).unwrap().mailbox,
            store.message(id(3)).unwrap().mailbox,
            store.message(id(4)).is_err(),
            listed(store, Filter::InMailbox(MailboxRole::Inbox)),
            listed(store, Filter::InFolder(mailbox("INBOX"))),
        )
    });
    assert_eq!(sqlite, memory);
    let (trashed, filed, kept, deleted, by_role, by_path) = sqlite;
    assert_eq!(trashed, MailboxRole::Trash);
    assert_eq!(filed, MailboxRole::Archive);
    assert_eq!(kept, MailboxRole::Inbox);
    assert!(deleted);
    // And the inbox by role is the inbox by path again.
    assert_eq!(by_role, by_path);
    assert_eq!(by_role, BTreeSet::from([thread(3)]));
}

#[test]
fn a_move_made_here_and_not_yet_sent_is_not_undone_by_the_server() {
    // The user archived it; the server has not been told. A copy arriving in the inbox is the
    // server's view, and the pending change goes back on top of it.
    let (sqlite, memory) = both(|store| {
        let id = MessageId::from_uuid(uuid::Uuid::from_u128(1));
        deliver(store, &message(1, MailboxRole::Inbox), imap("INBOX", 1));
        let patch = Patch {
            id: ChangeId::generate(),
            changes: vec![Change::MessageMailbox(id, MailboxRole::Archive)],
        };
        store.apply(ACCOUNT, &patch).unwrap();
        store
            .enqueue(
                ACCOUNT,
                RemoteIntent::SetMailbox {
                    messages: vec![id],
                    role: MailboxRole::Archive,
                },
                &Patch {
                    id: ChangeId::generate(),
                    changes: vec![Change::MessageMailbox(id, MailboxRole::Inbox)],
                },
                at(0),
            )
            .unwrap();
        deliver(store, &message(1, MailboxRole::Inbox), imap("INBOX", 2));
        store.message(id).unwrap().mailbox
    });
    assert_eq!(sqlite, memory);
    assert_eq!(sqlite, MailboxRole::Archive);
}
