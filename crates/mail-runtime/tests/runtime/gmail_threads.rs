//! Gmail's own thread id, from the survey to the thread a message lands in.
//!
//! The messages share nothing a header could join them on: each has its own `Message-ID`, no
//! `References` and a subject of its own. Only `X-GM-THRID` says which belong together.

use crate::fixture::{acct_account, inbox, now, store};
use crate::gmail_labels::survey;
use mail_domain::*;
use mail_runtime::{Arrival, assemble};
use mail_store::SqliteStore;
use mail_store::Store;

fn remote(uid: u32) -> RemoteRef {
    RemoteRef::Imap {
        mailbox: "INBOX".to_owned(),
        uidvalidity: 1,
        uid,
    }
}

/// What the server answers to a survey of `uids`, each with the thread id beside it.
fn surveyed(rows: &[(u32, &str)]) -> Ingest {
    let fetches: String = rows
        .iter()
        .enumerate()
        .map(|(n, (uid, thrid))| {
            format!(
                "* {} FETCH (UID {uid} FLAGS () RFC822.SIZE 100 X-GM-LABELS () X-GM-THRID {thrid})\r\n",
                n + 1
            )
        })
        .collect();
    survey(&[
        "* OK Gimap ready\r\n",
        "a001 OK authenticated\r\n",
        "* OK [UIDVALIDITY 1] UIDs valid.\r\n* OK [UIDNEXT 99] next\r\na002 OK [READ-ONLY] done\r\n",
        &format!("{fetches}a003 OK Success\r\n"),
    ])
}

fn arrive(store: &SqliteStore, uid: u32) -> Fetched {
    let raw = format!(
        "From: ada@example.test\r\nTo: me@example.test\r\nSubject: topic {uid}\r\n\
         Message-ID: <m{uid}@example.test>\r\nDate: Tue, 14 Nov 2023 22:13:20 +0000\r\n\r\nbody {uid}\r\n"
    );
    let ingest = assemble(
        store,
        acct_account(),
        inbox(),
        MailboxRole::Inbox,
        None,
        vec![Arrival {
            remote: remote(uid),
            raw: raw.into_bytes(),
        }],
        now(),
    )
    .unwrap();
    let fetched = ingest.messages[0].clone();
    store.ingest(acct_account(), ingest).unwrap();
    fetched
}

#[test]
fn messages_gmail_threads_together_are_one_thread_with_nothing_else_to_join_them() {
    let (store, _dir) = store();

    let seen = surveyed(&[(10, "111"), (11, "111"), (12, "222")]);
    assert_eq!(seen.provider_threads.len(), 3, "the survey carried no ids");
    store.ingest(acct_account(), seen).unwrap();

    let a = arrive(&store, 10);
    let b = arrive(&store, 11);
    let c = arrive(&store, 12);
    assert_eq!(a.message.thread, b.message.thread);
    assert_ne!(a.message.thread, c.message.thread);

    // A reply in the same Gmail thread, arriving on a later pass, joins what is stored.
    store
        .ingest(acct_account(), surveyed(&[(13, "111")]))
        .unwrap();
    let later = arrive(&store, 13);
    assert_eq!(later.message.thread, a.message.thread);
}

#[test]
fn without_a_thread_id_such_messages_stay_apart() {
    let (store, _dir) = store();
    let a = arrive(&store, 20);
    let b = arrive(&store, 21);
    assert_ne!(a.message.thread, b.message.thread);
}
