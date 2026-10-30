use super::*;
use crate::support_tests::account;
use chrono::TimeZone;
use mail_domain::{Address, Body, ChangeId, Message, MessageKey, Patch, ReadState, Star, ThreadId};
use mail_store::Store;

/// Noon, 2026-09-28, and whole days from it.
fn day(n: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 28, 12, 0, 0).unwrap() + TimeDelta::days(n)
}

fn id(n: u128) -> MessageId {
    MessageId::from_uuid(uuid::Uuid::from_u128(0x9000 + n))
}

fn thread(n: u128) -> ThreadId {
    ThreadId::from_uuid(uuid::Uuid::from_u128(0x7000 + n))
}

fn binned(mailbox: MailboxRole, since: i64, soon: Soon, settled: Settled) -> Binned {
    Binned {
        message: id(1),
        thread: thread(1),
        account: account(),
        mailbox,
        since: day(since),
        soon,
        settled,
    }
}

/// Erasing's whole table: what is due, and what waits, on day 0.
#[test]
fn what_is_erased_and_what_waits() {
    let week = Policy {
        delete: Delete::ToTrash,
        trash: Keep::Week,
        spam: Keep::Day,
    };
    let now_mode = Policy {
        delete: Delete::Now,
        ..Policy::default()
    };
    use MailboxRole::{Inbox, Spam, Trash};
    use Settled::{Pending, Unplaced, Yes};
    use Soon::{Asked, Waits};
    // (case, policy, binned, local, due)
    let cases: Vec<(&str, Policy, Binned, bool, bool)> = vec![
        (
            "in Trash a week",
            week,
            binned(Trash, -7, Waits, Yes),
            false,
            true,
        ),
        (
            "in Trash six days",
            week,
            binned(Trash, -6, Waits, Yes),
            false,
            false,
        ),
        (
            "in Spam a day",
            week,
            binned(Spam, -1, Waits, Yes),
            false,
            true,
        ),
        (
            "in Spam an hour",
            week,
            binned(Spam, 0, Waits, Yes),
            false,
            false,
        ),
        (
            "a move still on its way",
            week,
            binned(Trash, -30, Waits, Pending),
            false,
            false,
        ),
        (
            "not yet found on the server",
            week,
            binned(Trash, -30, Waits, Unplaced),
            false,
            false,
        ),
        (
            "local folders hold it wholly",
            week,
            binned(Trash, -30, Waits, Unplaced),
            true,
            true,
        ),
        (
            "kept always",
            Policy::default(),
            binned(Trash, -300, Waits, Yes),
            false,
            false,
        ),
        (
            "deleted now, reached Trash, past the Undo",
            now_mode,
            binned(Trash, -1, Asked, Yes),
            false,
            true,
        ),
        (
            "deleted now, reached Trash, Undo still on offer",
            now_mode,
            binned(Trash, 0, Asked, Yes),
            false,
            false,
        ),
        (
            "deleted now, still moving",
            now_mode,
            binned(Trash, 0, Asked, Pending),
            false,
            false,
        ),
        (
            "deleted now, then the setting went back",
            week,
            binned(Trash, 0, Asked, Yes),
            false,
            false,
        ),
        (
            "marked, but in Spam",
            now_mode,
            binned(Spam, 0, Asked, Yes),
            false,
            false,
        ),
        (
            "not a bin",
            week,
            binned(Inbox, -30, Waits, Yes),
            false,
            false,
        ),
    ];
    for (case, policy, one, local, want) in cases {
        assert_eq!(due(&policy, &one, local, day(0)), want, "{case}");
    }
}

fn message(n: u128, mailbox: MailboxRole) -> Message {
    Message {
        id: id(n),
        thread: thread(n),
        account: account(),
        key: MessageKey::Rfc(format!("m{n}@example.test")),
        date: day(-400),
        from: Address {
            name: None,
            email: "ada@example.test".to_owned(),
        },
        reply_to: vec![],
        to: vec![],
        cc: vec![],
        bcc: vec![],
        subject: format!("subject {n}"),
        in_reply_to: None,
        references: vec![],
        rfc_message_id: Some(format!("m{n}@example.test")),
        read: ReadState::Read,
        star: Star::Unstarred,
        flag: mail_domain::Hue::Red,
        mailbox,
        labels: vec![],
        body: Body::Absent,
        attachments: vec![],
    }
}

/// A store holding message 1 in Trash and message 2 in Spam, each at an address on the server,
/// and message 3 in the inbox.
fn store(dir: &std::path::Path) -> SqliteStore {
    let store = SqliteStore::open(dir.join("mail.db"), dir.join("blobs")).unwrap();
    mail_store::testing::seed_account(&store, account(), "me@example.test");
    for (n, mailbox, folder) in [
        (1, MailboxRole::Trash, "Trash"),
        (2, MailboxRole::Spam, "Junk"),
        (3, MailboxRole::Inbox, "INBOX"),
    ] {
        store
            .apply(
                account(),
                &Patch {
                    id: ChangeId::generate(),
                    changes: vec![Change::MessageUpsert(Box::new(message(n, mailbox)))],
                },
            )
            .unwrap();
        mail_store::testing::seed_remote_uid(&store, account(), folder, n as u32, id(n));
    }
    store
}

/// Time in Trash counts from when a sweep first found the message there, not from its date:
/// a year-old message is not erased by the first sweep, and is a week later. Spam keeps its own
/// time, and the inbox is never touched.
#[test]
fn a_week_in_trash_is_counted_from_when_it_was_found_there() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    let policy = Policy {
        delete: Delete::ToTrash,
        trash: Keep::Week,
        spam: Keep::Always,
    };
    assert!(sweep(&store, &policy, day(0)).is_empty(), "erased on sight");
    assert!(sweep(&store, &policy, day(6)).is_empty(), "erased early");
    assert_eq!(sweep(&store, &policy, day(7)), vec![id(1)]);
    assert!(store.message(id(1)).is_err(), "still here");
    assert!(store.message(id(2)).is_ok(), "Spam was erased");
    assert!(store.message(id(3)).is_ok(), "the inbox was erased");
    let destroys = mail_store::testing::outbox_ops(&store)
        .into_iter()
        .filter(|op| matches!(op, mail_domain::ProtoOp::Destroy { .. }))
        .count();
    assert_eq!(destroys, 1, "the server is told once");
}

/// Under "Delete deletes it now", what Delete moved is erased by the next sweep once the move
/// has reached the server, and not before; what was in Trash already keeps its time.
#[test]
fn deleting_now_erases_once_the_move_has_landed() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    let policy = Policy {
        delete: Delete::Now,
        ..Policy::default()
    };
    // A server that files mail in named folders, so Delete's move is queued for it.
    let caps = mail_domain::AccountCaps {
        archive: mail_domain::ArchiveMeans::MoveToFolder("Archive".to_owned()),
        folders: mail_domain::FolderRoles(vec![
            ("Trash".to_owned(), MailboxRole::Trash),
            ("Junk".to_owned(), MailboxRole::Spam),
        ]),
        ..crate::support_tests::caps()
    };
    mail_store::testing::seed_caps(&store, account(), &caps, chrono::Utc::now()).unwrap();
    // Message 1 was in Trash already; Delete moves message 3 there now.
    assert!(sweep(&store, &policy, day(0)).is_empty());
    let undo =
        crate::act::perform(&store, thread(3), mail_domain::Op::Trash).expect("moved to Trash");
    mark_deleted(&store, &[undo], day(0));
    assert!(
        sweep(&store, &policy, day(0)).is_empty(),
        "erased before the move reached the server"
    );
    // The server took the move: nothing waits, and it holds the message in Trash.
    mail_store::testing::clear_outbox(&store);
    let at = |mailbox: &str| mail_domain::RemoteRef::Imap {
        mailbox: mailbox.to_owned(),
        uidvalidity: 1,
        uid: 3,
    };
    store.remap(account(), &at("INBOX"), &at("Trash")).unwrap();
    assert!(
        sweep(&store, &policy, day(0)).is_empty(),
        "erased inside the Undo the Delete offered"
    );
    assert_eq!(sweep(&store, &policy, day(1)), vec![id(3)]);
    assert!(
        store.message(id(1)).is_ok(),
        "what was there already went too"
    );
}

/// A message trashed, restored and trashed again between two sweeps starts its time again: the
/// store forgets when it left, not only at the next sweep.
#[test]
fn a_message_that_leaves_trash_and_returns_between_sweeps_starts_again() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(dir.path());
    let policy = Policy {
        delete: Delete::ToTrash,
        trash: Keep::Week,
        spam: Keep::Always,
    };
    assert!(sweep(&store, &policy, day(0)).is_empty());
    let move_to = |role| {
        store
            .apply(
                account(),
                &Patch {
                    id: ChangeId::generate(),
                    changes: vec![mail_domain::Change::MessageMailbox(id(1), role)],
                },
            )
            .unwrap();
    };
    move_to(MailboxRole::Inbox);
    move_to(MailboxRole::Trash);
    assert!(
        sweep(&store, &policy, day(7)).is_empty(),
        "it kept the time from before it left"
    );
    assert_eq!(sweep(&store, &policy, day(14)), vec![id(1)]);
}
