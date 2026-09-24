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

// ---------------------------------------------------------------------------------------------
// Held there and still filed there: a move made here takes a message out of the folder view at
// once, while its server address stays until the server has moved it.
// ---------------------------------------------------------------------------------------------

const ELSEWHERE: LabelId = LabelId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000e1"));

/// What the window does: the op applied here, its remote half queued with its undo.
fn act(store: &dyn Store, op: Op, n: u128) {
    let m = store
        .message(MessageId::from_uuid(uuid::Uuid::from_u128(n)))
        .unwrap();
    let thread = store.thread(m.thread).unwrap();
    let messages: Vec<Message> = thread
        .messages
        .iter()
        .map(|id| store.message(*id).unwrap())
        .collect();
    let caps = moving_caps();
    let applied = op.apply(
        &Target::Messages(vec![m.id]),
        &thread,
        &messages,
        &caps,
        at(1),
    );
    store.apply(ACCOUNT, &applied.forward).unwrap();
    let intent = applied.remote.expect("a server with folders is told");
    store
        .enqueue(ACCOUNT, intent, &applied.inverse, at(1))
        .unwrap()
        .expect("the message has an address, so the move is queued");
}

fn moving_caps() -> AccountCaps {
    AccountCaps {
        archive: ArchiveMeans::MoveToFolder("Archive".to_owned()),
        ..caps(vec![("Trash", MailboxRole::Trash)])
    }
}

/// Every listing a folder view would show, in order: INBOX, then Projects.
type Views = Vec<(BTreeSet<ThreadId>, BTreeSet<ThreadId>)>;

fn moved_out_and_back(store: &dyn Store) -> Views {
    store.put_caps(ACCOUNT, &moving_caps(), at(0)).unwrap();
    store
        .apply(
            ACCOUNT,
            &Patch {
                id: ChangeId::generate(),
                changes: vec![Change::LabelUpsert(Label {
                    id: ELSEWHERE,
                    account: ACCOUNT,
                    name: "Elsewhere".to_owned(),
                    color: None,
                    origin: LabelOrigin::Provider,
                })],
            },
        )
        .unwrap();
    deliver(store, &message(1, MailboxRole::Inbox), imap("INBOX", 1));
    for n in 2..=4 {
        deliver(
            store,
            &message(n, MailboxRole::Archive),
            imap(PROJECTS, n as u32),
        );
    }
    // One message the server holds in both, filed in the inbox here.
    deliver(store, &message(5, MailboxRole::Inbox), imap("INBOX", 5));
    deliver(store, &message(5, MailboxRole::Archive), imap(PROJECTS, 5));
    let view = |store: &dyn Store| {
        (
            listed(store, Filter::InFolder(mailbox("INBOX"))),
            listed(store, Filter::InFolder(mailbox(PROJECTS))),
        )
    };
    let mut seen = vec![view(store)];
    act(store, Op::Archive, 1);
    seen.push(view(store));
    act(store, Op::Archive, 5);
    seen.push(view(store));
    assert!(
        !listed(store, Filter::InMailbox(MailboxRole::Inbox)).contains(&thread(5)),
        "the inbox by role drops it at once"
    );
    act(store, Op::Trash, 2);
    seen.push(view(store));
    act(store, Op::File(ELSEWHERE), 3);
    seen.push(view(store));
    // Filing into the folder it is already in is not leaving it.
    act(store, Op::File(label_named(store, PROJECTS)), 4);
    seen.push(view(store));

    // The server refuses every move for good: each undo is applied, and each comes back.
    for entry in store.outbox_due(ACCOUNT, at(10_000)).unwrap() {
        store
            .outbox_settle(
                entry.id,
                mail_store::Settle::Failed {
                    reason: "NO no such folder".to_owned(),
                    retry: Retry::Fatal("NO no such folder".to_owned()),
                },
                at(2),
            )
            .unwrap();
    }
    seen.push(view(store));
    seen
}

/// The label a folder path is, made where new: what `Op::File` names.
fn label_named(store: &dyn Store, path: &str) -> LabelId {
    if let Some(label) = store
        .labels(ACCOUNT)
        .unwrap()
        .into_iter()
        .find(|l| l.name == path)
    {
        return label.id;
    }
    let id = LabelId::generate();
    store
        .apply(
            ACCOUNT,
            &Patch {
                id: ChangeId::generate(),
                changes: vec![Change::LabelUpsert(Label {
                    id,
                    account: ACCOUNT,
                    name: path.to_owned(),
                    color: None,
                    origin: LabelOrigin::Provider,
                })],
            },
        )
        .unwrap();
    id
}

#[test]
fn a_move_from_a_folder_view_leaves_it_at_once_and_a_refused_one_comes_back() {
    let (sqlite, memory) = both(moved_out_and_back);
    let set = |ns: &[u128]| ns.iter().map(|n| thread(*n)).collect::<BTreeSet<_>>();
    assert_eq!(
        sqlite,
        vec![
            // The copy held in both is listed in both.
            (set(&[1, 5]), set(&[2, 3, 4, 5])),
            // Archived from the inbox: its INBOX address stays, the inbox view drops it.
            (set(&[5]), set(&[2, 3, 4, 5])),
            // The copy in both archived from the inbox: still in the folder, as on the server.
            // Also still in the `INBOX` folder view until the server has moved it: archived is
            // what its `Projects` copy is filed as, so it is filed as held, and the rule asks
            // no more than that. The inbox by role (`InMailbox(Inbox)`) drops it at once.
            (set(&[5]), set(&[2, 3, 4, 5])),
            // Trashed from the folder view.
            (set(&[5]), set(&[3, 4, 5])),
            // Moved to another folder, the move queued and unconfirmed.
            (set(&[5]), set(&[4, 5])),
            // Filed where it already is: still there.
            (set(&[5]), set(&[4, 5])),
            // Every move refused and undone: all back where they were.
            (set(&[1, 5]), set(&[2, 3, 4, 5])),
        ]
    );
    assert_eq!(memory, sqlite, "the in-memory store answers the same");
}

/// The server address itself is never moved ahead of the server: the outbox still addresses
/// the message where the server holds it.
#[test]
fn a_move_made_here_leaves_the_server_address_alone() {
    let (sqlite, memory) = both(|store| {
        store.put_caps(ACCOUNT, &moving_caps(), at(0)).unwrap();
        deliver(store, &message(1, MailboxRole::Inbox), imap("INBOX", 1));
        act(store, Op::Archive, 1);
        let id = MessageId::from_uuid(uuid::Uuid::from_u128(1));
        (
            store.remotes_of(id).unwrap(),
            store
                .outbox_due(ACCOUNT, at(10_000))
                .unwrap()
                .into_iter()
                .map(|e| e.op)
                .collect::<Vec<_>>(),
        )
    });
    assert_eq!(sqlite.0, vec![imap("INBOX", 1)]);
    assert_eq!(
        sqlite.1,
        vec![ProtoOp::SetMailbox {
            remotes: vec![imap("INBOX", 1)],
            role: MailboxRole::Archive,
        }]
    );
    assert_eq!(memory, sqlite);
}
