//! Which mailboxes a sync fetches, and what a message remembers about where it came from.
//!
//! Only `INBOX` was ever fetched, and nothing said so. A user who sends mail from their phone
//! opened Sent and found only what this client had sent; mail archived elsewhere vanished from
//! the inbox and appeared nowhere. And every absorbed message was `MailboxRole::Inbox` whatever
//! folder it came from — invisible while one folder was synced, and the first thing to go wrong
//! when a second one is.

use chrono::{DateTime, TimeZone, Utc};
use mail_domain::*;
use mail_runtime::{Arrival, Destination, absorb_into};
use mail_store::{SqliteStore, Store};

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));

fn now() -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000, 0).unwrap()
}

fn store() -> (SqliteStore, tempfile::TempDir) {
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

fn deliver(store: &SqliteStore, path: &str, role: MailboxRole, uidl: &str, subject: &str) {
    absorb_into(
        store,
        ACCOUNT,
        Destination {
            mailbox: MailboxRef {
                account: ACCOUNT,
                path: path.to_owned(),
            },
            role,
        },
        None,
        vec![Arrival {
            remote: RemoteRef::Pop {
                uidl: uidl.to_owned(),
            },
            raw: format!(
                "From: a@example.test\r\nSubject: {subject}\r\n\
                 Message-ID: <{uidl}@example.test>\r\n\r\nbody\r\n"
            )
            .into_bytes(),
        }],
        false,
        now(),
    )
    .unwrap();
}

fn subjects(store: &SqliteStore, role: MailboxRole) -> Vec<String> {
    store
        .threads(
            &Query {
                filter: Filter::InMailbox(role),
                sort: Sort {
                    property: Property::Date,
                    dir: SortDir::Desc,
                },
                page: PageReq {
                    after: None,
                    limit: 50,
                },
            },
            now(),
        )
        .unwrap()
        .items
        .into_iter()
        .map(|t| t.subject)
        .collect()
}

#[test]
fn a_message_from_sent_is_in_sent_and_not_in_the_inbox() {
    // The whole point. Absorbed as Inbox whatever folder it came from, mail the user sent would
    // be listed among the mail they received, and archiving it would be the only way to make it
    // go away — an operation that means something else entirely.
    let (store, _dir) = store();
    deliver(&store, "INBOX", MailboxRole::Inbox, "u1", "arrived");
    deliver(
        &store,
        "[Gmail]/Sent Mail",
        MailboxRole::Sent,
        "u2",
        "I sent this",
    );

    assert_eq!(subjects(&store, MailboxRole::Inbox), vec!["arrived"]);
    assert_eq!(subjects(&store, MailboxRole::Sent), vec!["I sent this"]);
}

#[test]
fn archived_mail_lands_in_archive() {
    let (store, _dir) = store();
    deliver(
        &store,
        "[Gmail]/All Mail",
        MailboxRole::Archive,
        "u1",
        "old thread",
    );
    assert_eq!(subjects(&store, MailboxRole::Archive), vec!["old thread"]);
    assert!(subjects(&store, MailboxRole::Inbox).is_empty());
}

#[test]
fn a_message_in_two_folders_is_stored_once_and_keeps_the_first_role() {
    // Gmail files by label: the same message is in INBOX and in All Mail under different uids.
    // Identity is `MessageKey`, so it is stored once — and `Message.mailbox` is *one* role, so
    // the second folder to see it does not move it.
    //
    // That is why `sync::to_sync` fetches Sent and not Archive. A pass over All Mail would
    // otherwise be a pass that marks inbox mail archived, and the user's inbox would empty
    // itself. Fixing it properly means a message carrying a set of mailboxes rather than one,
    // which is a domain change; this test is here so the limitation is written down in a place
    // that fails if the model changes underneath it.
    let (store, _dir) = store();
    let raw = "From: a@example.test\r\nSubject: both\r\n\
               Message-ID: <both@example.test>\r\n\r\nbody\r\n";
    for (path, role, uid) in [
        ("INBOX", MailboxRole::Inbox, 1u32),
        ("[Gmail]/All Mail", MailboxRole::Archive, 2),
    ] {
        absorb_into(
            &store,
            ACCOUNT,
            Destination {
                mailbox: MailboxRef {
                    account: ACCOUNT,
                    path: path.to_owned(),
                },
                role,
            },
            None,
            vec![Arrival {
                remote: RemoteRef::Imap {
                    mailbox: path.to_owned(),
                    uidvalidity: 1,
                    uid,
                },
                raw: raw.as_bytes().to_vec(),
            }],
            false,
            now(),
        )
        .unwrap();
    }

    let count: i64 = store
        .connection()
        .query_row("SELECT count(*) FROM messages", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1, "the same message was stored twice");
    assert_eq!(
        subjects(&store, MailboxRole::Inbox),
        vec!["both"],
        "the folder it was first seen in is where it stays"
    );
    assert!(
        subjects(&store, MailboxRole::Archive).is_empty(),
        "one role per message, so it cannot be in both"
    );
}

#[test]
fn a_folder_with_no_role_is_absorbed_as_the_inbox() {
    // A user folder the server did not flag. Not synced today, but if it ever is, mail must
    // land somewhere the user looks rather than vanishing into a role that has no place.
    let (store, _dir) = store();
    deliver(
        &store,
        "Work/Projects",
        MailboxRole::Inbox,
        "u1",
        "a project",
    );
    assert_eq!(subjects(&store, MailboxRole::Inbox), vec!["a project"]);
}
