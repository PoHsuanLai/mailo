//! Which of a POP3 account's server copies a pass may remove, and letting go of one: the message
//! stays here, at no address, so the next survey does not read it as deleted elsewhere.

use crate::support::*;
use mail_domain::*;
use mail_store::{SqliteStore, Store};

fn pop(uidl: &str) -> RemoteRef {
    RemoteRef::Pop {
        uidl: uidl.to_owned(),
    }
}

/// Message `n`, dated `n` seconds in, filed as `role`, with its body here or not.
fn put(store: &SqliteStore, n: u128, role: MailboxRole, whole: bool) {
    let body = if whole {
        Body::Present {
            text: Some(format!("body {n}")),
            raw: store.blobs().put(format!("raw {n}").as_bytes()).unwrap(),
        }
    } else {
        Body::Absent
    };
    let m = Message {
        mailbox: role,
        body,
        ..message(n)
    };
    deliver(store, &m, pop(&format!("u{n}")));
}

/// Four messages: 1 and 2 held in full, 3 only as headers, 4 held in full in the Trash.
fn store() -> SqliteStore {
    let store = sqlite();
    put(&store, 1, MailboxRole::Inbox, true);
    put(&store, 2, MailboxRole::Inbox, true);
    put(&store, 3, MailboxRole::Inbox, false);
    put(&store, 4, MailboxRole::Trash, true);
    // Each was stored when it is dated, unless a test says otherwise.
    mail_store::testing::set_stored_at(&store, None);
    store
}

#[test]
fn only_what_is_held_in_full_and_old_enough_may_go() {
    let store = store();
    let inbox = mailbox("INBOX");
    let cases: [(i64, u32, &[&str]); 4] = [
        (0, 10, &[]),
        (2, 10, &["u1"]),
        // The headers-only message is never one, however old.
        (10, 10, &["u1", "u2", "u4"]),
        (10, 1, &["u1"]),
    ];
    for (before, limit, want) in cases {
        let got = store.copies_held_before(&inbox, at(before), limit).unwrap();
        let want: Vec<RemoteRef> = want.iter().map(|u| pop(u)).collect();
        assert_eq!(got, want, "before {before}, at most {limit}");
    }
}

#[test]
fn a_message_stored_lately_is_not_old_whatever_its_date_says() {
    let store = store();
    // Dated long ago, but stored just now, as a late or forged Date header would be.
    mail_store::testing::set_stored_at(&store, Some(at(100)));
    let got = store
        .copies_held_before(&mailbox("INBOX"), at(10), 10)
        .unwrap();
    assert!(got.is_empty(), "{got:?}");
    let got = store
        .copies_held_before(&mailbox("INBOX"), at(200), 10)
        .unwrap();
    assert_eq!(got, [pop("u1"), pop("u2"), pop("u4")]);
}

#[test]
fn what_is_in_the_trash_here_is_deleted_here() {
    let store = store();
    assert_eq!(
        store.copies_deleted_here(&mailbox("INBOX"), 10).unwrap(),
        [pop("u4")]
    );
}

#[test]
fn a_header_only_message_in_the_trash_is_not_deleted_from_the_server() {
    let store = store();
    put(&store, 5, MailboxRole::Trash, false);
    assert_eq!(
        store.copies_deleted_here(&mailbox("INBOX"), 10).unwrap(),
        [pop("u4")],
        "the server holds the only copy of u5"
    );
}

#[test]
fn a_copy_let_go_of_leaves_its_message_here_at_no_address() {
    let store = store();
    let inbox = mailbox("INBOX");
    store.let_go_of(acct_account(), &[pop("u1")]).unwrap();
    let held = store.remote_refs(&inbox).unwrap();
    assert!(!held.contains(&pop("u1")), "{held:?}");
    assert!(held.contains(&pop("u2")), "{held:?}");
    let message = store.message(crate::support::message(1).id).unwrap();
    assert!(matches!(message.body, Body::Present { .. }), "kept whole");
    assert!(store.remotes_of(message.id).unwrap().is_empty());
}
