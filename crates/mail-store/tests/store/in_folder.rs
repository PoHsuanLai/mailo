//! A folder's mail, by where the server holds it: `Filter::InFolder`, the body backlog of one
//! mailbox, and what a message held in several mailboxes is filed as.
//!
//! Every scenario runs against the SQLite store and the in-memory one and compares what each
//! answers, the parity the rest of the store is held to.

use crate::support::*;
use mail_domain::id::account_id_from_uuid;
use mail_domain::*;
use mail_store::{SqlValue, Store, compile};
use std::collections::BTreeSet;

const PROJECTS: &str = "Projects/2026";

/// Message `n`, as a header fetch from a folder filed as `role` builds it.
fn message(n: u128, role: MailboxRole) -> Message {
    Message {
        mailbox: role,
        ..crate::support::message(n)
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
    store.ingest(acct_account(), batch).unwrap();
}

/// The server says `remote` has gone from its mailbox.
fn gone(store: &dyn Store, remote: RemoteRef) {
    let RemoteRef::Imap { mailbox: path, .. } = &remote else {
        unreachable!("IMAP only here")
    };
    let mut batch = ingest(path);
    batch.gone.push(remote);
    store.ingest(acct_account(), batch).unwrap();
}

fn listed(store: &dyn Store, filter: Filter) -> BTreeSet<ThreadId> {
    store
        .threads(&query(filter, 100), at(0))
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
                    account: account_id_from_uuid(uuid::Uuid::from_u128(9)),
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
    let store = sqlite();
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
    let db = store.raw_connection();
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
            store.unfetched_in(&mailbox("INBOX"), None, 10).unwrap(),
            store.unfetched_in(&mailbox(PROJECTS), None, 10).unwrap(),
            store.unfetched_in(&mailbox(PROJECTS), None, 2).unwrap(),
            store.unfetched_in(&mailbox("Elsewhere"), None, 10).unwrap(),
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

/// An account downloaded only so far back: what is older than the window is left out before the
/// limit cuts, so the newest of the window are never pushed out by mail it would not fetch.
#[test]
fn a_body_pass_reaches_back_only_as_far_as_it_is_told() {
    let (sqlite, memory) = both(|store| {
        for n in 1..=4 {
            deliver(
                store,
                &message(n, MailboxRole::Inbox),
                imap("INBOX", n as u32),
            );
        }
        let inbox = mailbox("INBOX");
        (
            store.unfetched_in(&inbox, Some(at(3)), 10).unwrap(),
            store.unfetched_in(&inbox, Some(at(2)), 1).unwrap(),
            store.unfetched_in(&inbox, Some(at(5)), 10).unwrap(),
        )
    });
    assert_eq!(sqlite, memory);
    let (since_third, cut, none) = sqlite;
    assert_eq!(since_third, vec![imap("INBOX", 4), imap("INBOX", 3)]);
    assert_eq!(cut, vec![imap("INBOX", 4)]);
    assert!(none.is_empty(), "everything is older than the window");
}

#[test]
fn the_backlog_of_named_threads_reaches_below_any_cut_of_the_whole() {
    // Each `message(n)` is its own thread, `n + 1000`.
    let thread = |n: u128| ThreadId::from_uuid(uuid::Uuid::from_u128(n + 1000));
    let (sqlite, memory) = both(|store| {
        deliver(store, &message(1, MailboxRole::Inbox), imap("INBOX", 1));
        deliver(store, &message(2, MailboxRole::Inbox), imap("INBOX", 2));
        deliver(store, &message(3, MailboxRole::Inbox), imap("INBOX", 3));
        deliver(store, &message(4, MailboxRole::Archive), imap(PROJECTS, 4));
        (
            // The oldest is the one the whole backlog cut at one leaves out.
            store.unfetched_in(&mailbox("INBOX"), None, 1).unwrap(),
            store
                .unfetched_in_threads(&mailbox("INBOX"), &[thread(1), thread(3)], 10)
                .unwrap(),
            // A thread held only elsewhere has nothing to fetch from this mailbox.
            store
                .unfetched_in_threads(&mailbox("INBOX"), &[thread(4)], 10)
                .unwrap(),
            store
                .unfetched_in_threads(&mailbox("INBOX"), &[], 10)
                .unwrap(),
        )
    });
    assert_eq!(sqlite, memory);
    let (cut, named, elsewhere, none) = sqlite;
    assert_eq!(cut, vec![imap("INBOX", 3)]);
    // Newest first, each beside its thread, and the oldest is there although the cut missed it.
    assert_eq!(
        named,
        vec![(thread(3), imap("INBOX", 3)), (thread(1), imap("INBOX", 1))]
    );
    assert!(elsewhere.is_empty());
    assert!(none.is_empty());
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
            .put_caps(
                acct_account(),
                &caps(vec![("Trash", MailboxRole::Trash)]),
                at(0),
            )
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
        let filed = patch(vec![Change::MessageMailbox(id, MailboxRole::Archive)]);
        store.apply(acct_account(), &filed).unwrap();
        store
            .enqueue(
                acct_account(),
                RemoteIntent::SetMailbox {
                    messages: vec![id],
                    role: MailboxRole::Archive,
                },
                &patch(vec![Change::MessageMailbox(id, MailboxRole::Inbox)]),
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
    store.apply(acct_account(), &applied.forward).unwrap();
    let intent = applied.remote.expect("a server with folders is told");
    store
        .enqueue(acct_account(), intent, &applied.inverse, at(1))
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
    store
        .put_caps(acct_account(), &moving_caps(), at(0))
        .unwrap();
    store
        .apply(
            acct_account(),
            &patch(vec![Change::LabelUpsert(Label {
                id: ELSEWHERE,
                account: acct_account(),
                name: "Elsewhere".to_owned(),
                color: None,
                origin: LabelOrigin::Provider,
            })]),
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
    for entry in store.outbox_due(acct_account(), at(10_000)).unwrap() {
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
        .labels(acct_account())
        .unwrap()
        .into_iter()
        .find(|l| l.name == path)
    {
        return label.id;
    }
    let id = LabelId::generate();
    store
        .apply(
            acct_account(),
            &patch(vec![Change::LabelUpsert(Label {
                id,
                account: acct_account(),
                name: path.to_owned(),
                color: None,
                origin: LabelOrigin::Provider,
            })]),
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
        store
            .put_caps(acct_account(), &moving_caps(), at(0))
            .unwrap();
        deliver(store, &message(1, MailboxRole::Inbox), imap("INBOX", 1));
        act(store, Op::Archive, 1);
        let id = MessageId::from_uuid(uuid::Uuid::from_u128(1));
        (
            store.remotes_of(id).unwrap(),
            store
                .outbox_due(acct_account(), at(10_000))
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

// ---------------------------------------------------------------------------------------------
// Copied: a copy made here lists the message in the folder it was copied to at once, beside the
// one it is in, and the server's copy is the same message when a sync finds it.
// ---------------------------------------------------------------------------------------------

/// The listings and holdings a copy is judged by: INBOX, then Projects, then every thread held,
/// then the addresses held in Projects.
type Copied = (
    BTreeSet<ThreadId>,
    BTreeSet<ThreadId>,
    BTreeSet<ThreadId>,
    Vec<RemoteRef>,
);

fn copied_view(store: &dyn Store) -> Copied {
    (
        listed(store, Filter::InFolder(mailbox("INBOX"))),
        listed(store, Filter::InFolder(mailbox(PROJECTS))),
        listed(store, Filter::All),
        store.remote_refs(&mailbox(PROJECTS)).unwrap(),
    )
}

/// A sync of Projects that finds the server's copy of message `n` at `uid`, fetched afresh as
/// the runtime builds it: new ids, the folder's role, the original's identity.
fn found_in_projects(store: &dyn Store, n: u128, uid: u32) {
    let fetched = Message {
        id: MessageId::generate(),
        thread: ThreadId::generate(),
        ..message(n, MailboxRole::Archive)
    };
    deliver(store, &fetched, imap(PROJECTS, uid));
}

/// Copy message 1 from the inbox into Projects, and say what each step left.
fn copy_then_sync(store: &dyn Store, copyuid: bool) -> Vec<Copied> {
    store
        .put_caps(acct_account(), &moving_caps(), at(0))
        .unwrap();
    deliver(store, &message(1, MailboxRole::Inbox), imap("INBOX", 1));
    let mut seen = vec![copied_view(store)];
    act(store, Op::Copy(label_named(store, PROJECTS)), 1);
    seen.push(copied_view(store));
    let queued = store.outbox_due(acct_account(), at(10_000)).unwrap();
    assert_eq!(queued.len(), 1);
    assert_eq!(
        queued[0].op,
        ProtoOp::Copy {
            remotes: vec![imap("INBOX", 1)],
            folder: PROJECTS.to_owned(),
        }
    );
    // The server copies it, and says where (`COPYUID`) or does not.
    if copyuid {
        store
            .copied(acct_account(), &imap("INBOX", 1), &imap(PROJECTS, 9))
            .unwrap();
    }
    store
        .outbox_settle(queued[0].id, mail_store::Settle::Ok, at(2))
        .unwrap();
    seen.push(copied_view(store));
    found_in_projects(store, 1, 9);
    seen.push(copied_view(store));
    seen
}

#[test]
fn a_copy_is_listed_in_its_folder_at_once_and_once_after_the_sync() {
    let one = BTreeSet::from([thread(1)]);
    let none = BTreeSet::new();
    let copy = || vec![imap(PROJECTS, 9)];
    for (case, copyuid, settled) in [
        // Said where: kept at once, and the sync finds it held.
        (
            "COPYUID",
            true,
            (one.clone(), one.clone(), one.clone(), copy()),
        ),
        // Not said: out of the folder until the sync of it finds the copy, by its identity.
        (
            "no COPYUID",
            false,
            (one.clone(), none.clone(), one.clone(), vec![]),
        ),
    ] {
        let (sqlite, memory) = both(|store| copy_then_sync(store, copyuid));
        assert_eq!(sqlite, memory, "{case}");
        assert_eq!(
            sqlite,
            vec![
                (one.clone(), none.clone(), one.clone(), vec![]),
                // Listed in both the moment it is copied, with no address there yet.
                (one.clone(), one.clone(), one.clone(), vec![]),
                settled,
                // One message, in both, at one address in each: the sync stored nothing again.
                (one.clone(), one.clone(), one.clone(), copy()),
            ],
            "{case}"
        );
    }
}

#[test]
fn a_copy_the_server_refuses_leaves_the_folder_as_it_was() {
    let (sqlite, memory) = both(|store| {
        store
            .put_caps(acct_account(), &moving_caps(), at(0))
            .unwrap();
        deliver(store, &message(1, MailboxRole::Inbox), imap("INBOX", 1));
        let projects = label_named(store, PROJECTS);
        act(store, Op::Copy(projects), 1);
        let listed_while_queued = copied_view(store);
        for entry in store.outbox_due(acct_account(), at(10_000)).unwrap() {
            store
                .outbox_settle(
                    entry.id,
                    mail_store::Settle::Failed {
                        reason: "NO [TRYCREATE] no such folder".to_owned(),
                        retry: Retry::Fatal("NO no such folder".to_owned()),
                    },
                    at(2),
                )
                .unwrap();
        }
        let labelled = store
            .message(MessageId::from_uuid(uuid::Uuid::from_u128(1)))
            .unwrap()
            .labels
            .contains(&projects);
        (listed_while_queued, copied_view(store), labelled)
    });
    assert_eq!(sqlite, memory);
    let (queued, refused, labelled) = sqlite;
    assert_eq!(queued.1, BTreeSet::from([thread(1)]));
    assert_eq!(
        refused.1,
        BTreeSet::new(),
        "still listed where the copy failed"
    );
    assert_eq!(
        refused.0,
        BTreeSet::from([thread(1)]),
        "the original left the inbox"
    );
    assert!(!labelled, "the refused copy's label stayed");
}

/// A copy names one address of each message, since copying from each would leave as many copies
/// as the message has places, and none of a message the folder already holds.
#[test]
fn a_copy_is_sent_from_one_address_and_not_for_mail_already_there() {
    let (sqlite, memory) = both(|store| {
        store
            .put_caps(acct_account(), &moving_caps(), at(0))
            .unwrap();
        // Message 1 in the inbox and in Archive; message 2 in the inbox and Projects already.
        deliver(store, &message(1, MailboxRole::Inbox), imap("INBOX", 1));
        deliver(store, &message(1, MailboxRole::Archive), imap("Archive", 3));
        deliver(store, &message(2, MailboxRole::Inbox), imap("INBOX", 2));
        deliver(store, &message(2, MailboxRole::Archive), imap(PROJECTS, 4));
        let projects = label_named(store, PROJECTS);
        let ids = [1u128, 2].map(|n| MessageId::from_uuid(uuid::Uuid::from_u128(n)));
        store
            .enqueue(
                acct_account(),
                RemoteIntent::Copy {
                    messages: ids.to_vec(),
                    label: projects,
                },
                &patch(Vec::new()),
                at(1),
            )
            .unwrap()
            .expect("queued");
        let entry = store.outbox_due(acct_account(), at(10_000)).unwrap()[0].id;
        match store.outbox_dispatch(entry).unwrap() {
            mail_store::Dispatch::Send(ProtoOp::Copy { remotes, folder }) => (remotes, folder),
            other => panic!("{other:?}"),
        }
    });
    assert_eq!(sqlite.1, PROJECTS);
    assert_eq!(sqlite.0.len(), 1, "{:?}", sqlite.0);
    assert!(
        [imap("INBOX", 1), imap("Archive", 3)].contains(&sqlite.0[0]),
        "{:?}",
        sqlite.0
    );
    assert_eq!(memory.1, PROJECTS);
    assert_eq!(memory.0.len(), 1, "{:?}", memory.0);
}
