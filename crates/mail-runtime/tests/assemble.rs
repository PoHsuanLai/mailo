//! Raw mail in, stored and listable mail out.
//!
//! The seam `mail-proto` cannot cross, tested with real RFC 5322 bytes rather than constructed
//! `Message` values — because what is most likely to be wrong here is what happens to a message
//! with a missing header, a broken date, or no `Message-ID` at all, and those only exist in
//! bytes.

use chrono::{DateTime, TimeZone, Utc};
use mail_domain::*;
use mail_runtime::{Arrival, absorb};
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

fn mailbox() -> MailboxRef {
    MailboxRef {
        account: ACCOUNT,
        path: "INBOX".to_owned(),
    }
}

fn pop(uidl: &str, raw: &str) -> Arrival {
    Arrival {
        remote: RemoteRef::Pop {
            uidl: uidl.to_owned(),
        },
        raw: raw.replace('\n', "\r\n").into_bytes(),
    }
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

#[test]
fn a_whole_message_becomes_something_listable_and_searchable() {
    let (store, _dir) = store();
    let raw = "From: Ada Lovelace <ada@example.test>\n\
               To: me@example.test\n\
               Subject: lunch on friday\n\
               Date: Tue, 14 Nov 2023 22:13:20 +0000\n\
               Message-ID: <first@example.test>\n\
               \n\
               Shall we say one o'clock?\n";
    absorb(
        &store,
        ACCOUNT,
        mailbox(),
        SyncCursor::Pop,
        vec![pop("0000000166aaf64b", raw)],
        false,
        now(),
    )
    .unwrap();

    let listed = store
        .threads(&query(Filter::InMailbox(MailboxRole::Inbox)), now())
        .unwrap();
    assert_eq!(listed.items.len(), 1);
    assert_eq!(listed.items[0].subject, "lunch on friday");
    assert_eq!(listed.items[0].from.email, "ada@example.test");

    // Searchable through the full-text path, which means the body reached FTS5.
    let found = store
        .threads(
            &query(Filter::Text(TextMatch::Contains("clock".into()))),
            now(),
        )
        .unwrap();
    assert_eq!(found.items.len(), 1, "the body should be searchable");
}

/// The same message arriving twice must not become two.
#[test]
fn a_message_fetched_twice_is_stored_once() {
    let (store, _dir) = store();
    let raw = "From: a@example.test\nSubject: once\nMessage-ID: <dup@example.test>\n\nbody\n";
    for _ in 0..2 {
        absorb(
            &store,
            ACCOUNT,
            mailbox(),
            SyncCursor::Pop,
            vec![pop("uidl-1", raw)],
            false,
            now(),
        )
        .unwrap();
    }
    let count: i64 = store
        .connection()
        .query_row("SELECT count(*) FROM messages", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1, "identity is MessageKey, so a refetch re-maps");
}

/// A reply must join its parent's conversation rather than starting a new one.
#[test]
fn a_reply_joins_the_thread_it_answers_even_across_syncs() {
    let (store, _dir) = store();
    let first = "From: a@example.test\nSubject: plan\nMessage-ID: <p1@example.test>\n\nfirst\n";
    let reply = "From: b@example.test\nSubject: Re: plan\n\
                 Message-ID: <p2@example.test>\nIn-Reply-To: <p1@example.test>\n\
                 References: <p1@example.test>\n\nsecond\n";

    // Two separate passes, because that is what an incremental sync does — and joining across
    // them is what `existing` in the threading call is for.
    absorb(
        &store,
        ACCOUNT,
        mailbox(),
        SyncCursor::Pop,
        vec![pop("u1", first)],
        false,
        now(),
    )
    .unwrap();
    absorb(
        &store,
        ACCOUNT,
        mailbox(),
        SyncCursor::Pop,
        vec![pop("u2", reply)],
        false,
        now(),
    )
    .unwrap();

    let threads = store.threads(&query(Filter::All), now()).unwrap();
    assert_eq!(
        threads.items.len(),
        1,
        "a reply must join, not renumber: {:?}",
        threads.items.iter().map(|t| &t.subject).collect::<Vec<_>>()
    );
    assert_eq!(threads.items[0].message_count, 2);
}

/// Mail in the wild is broken in exactly these ways, and none of them is an error.
#[test]
fn malformed_headers_are_recorded_rather_than_rejected() {
    let (store, _dir) = store();
    let no_message_id = "From: a@example.test\nSubject: anonymous\n\nbody\n";
    let bad_date = "From: b@example.test\nSubject: whenever\n\
                    Date: not a date at all\nMessage-ID: <bd@example.test>\n\nbody\n";
    let no_subject = "From: c@example.test\nMessage-ID: <ns@example.test>\n\nbody\n";

    absorb(
        &store,
        ACCOUNT,
        mailbox(),
        SyncCursor::Pop,
        vec![
            pop("u1", no_message_id),
            pop("u2", bad_date),
            pop("u3", no_subject),
        ],
        false,
        now(),
    )
    .unwrap();

    let threads = store.threads(&query(Filter::All), now()).unwrap();
    assert_eq!(threads.items.len(), 3, "all three are real messages");
    // The unparseable date falls back to the server's, rather than rejecting the message.
    assert!(
        threads
            .items
            .iter()
            .all(|t| t.last_date == now() || t.last_date.timestamp() > 0)
    );
}

/// Two messages with no Message-ID and identical headers must not collapse into one.
#[test]
fn messages_with_no_message_id_stay_distinct() {
    let (store, _dir) = store();
    // Same subject, same sender, no Message-ID, no date. Only the remote address differs —
    // which is why it is part of the synthetic key. Without it the user silently loses one.
    let raw = "From: a@example.test\nSubject: identical\n\nbody\n";
    absorb(
        &store,
        ACCOUNT,
        mailbox(),
        SyncCursor::Pop,
        vec![pop("uidl-a", raw), pop("uidl-b", raw)],
        false,
        now(),
    )
    .unwrap();

    let count: i64 = store
        .connection()
        .query_row("SELECT count(*) FROM messages", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 2, "two arrivals are two messages");
}

/// A headers-only pass produces listable mail with no body — the whole point of Body::Absent.
#[test]
fn a_headers_only_pass_is_listable_without_a_body() {
    let (store, _dir) = store();
    let raw =
        "From: a@example.test\nSubject: headers first\nMessage-ID: <h@example.test>\n\nbody\n";
    absorb(
        &store,
        ACCOUNT,
        mailbox(),
        SyncCursor::Pop,
        vec![pop("u1", raw)],
        true,
        now(),
    )
    .unwrap();

    let threads = store.threads(&query(Filter::All), now()).unwrap();
    assert_eq!(threads.items.len(), 1);
    assert_eq!(threads.items[0].subject, "headers first");

    let message_id = store.thread(threads.items[0].id).unwrap().messages[0];
    assert_eq!(
        store.message(message_id).unwrap().body,
        Body::Absent,
        "the body must be absent, not empty"
    );

    // And it is exactly the work `Store::unfetched` will find on the next pass.
    assert_eq!(store.unfetched(ACCOUNT, 10).unwrap().len(), 1);
}

/// One unparseable message must not stop the rest of a maildrop arriving.
#[test]
fn bytes_that_are_not_a_message_are_skipped_not_fatal() {
    let (store, _dir) = store();
    let good = "From: a@example.test\nSubject: fine\nMessage-ID: <g@example.test>\n\nbody\n";
    absorb(
        &store,
        ACCOUNT,
        mailbox(),
        SyncCursor::Pop,
        vec![
            Arrival {
                remote: RemoteRef::Pop {
                    uidl: "junk".into(),
                },
                raw: vec![0u8; 64],
            },
            pop("u2", good),
        ],
        false,
        now(),
    )
    .expect("one bad message must not fail the pass");

    let threads = store.threads(&query(Filter::All), now()).unwrap();
    assert_eq!(threads.items.len(), 1, "the good one still arrived");
}
