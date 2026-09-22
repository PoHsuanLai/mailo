//! What a message with very many parts costs on the way in.
//!
//! Written looking for an amplification: a small message that makes the client do a large amount
//! of work is a denial of service anyone can post, and a mail client accepts input from
//! strangers by definition. The answer is that there isn't one — the cost is linear in the size
//! of the message and the blob store collapses repeated parts by content — but "we checked" is
//! only worth anything if it keeps being checked, so the measurements are assertions now.

use chrono::{DateTime, TimeZone, Utc};
use mail_domain::*;
use mail_runtime::{Arrival, absorb};
use mail_store::{SqliteStore, Store};
use std::time::Instant;

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));

fn now() -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000, 0).unwrap()
}

/// A multipart message with `parts` attachments. `unique` decides whether their contents differ,
/// which is what defeats the blob store's content addressing.
fn flood(parts: usize, unique: bool) -> Vec<u8> {
    let mut m = Vec::new();
    m.extend_from_slice(
        b"From: a@b.test\r\nSubject: s\r\nMessage-ID: <f@b.test>\r\nMIME-Version: 1.0\r\n",
    );
    m.extend_from_slice(b"Content-Type: multipart/mixed; boundary=\"b\"\r\n\r\n");
    for i in 0..parts {
        m.extend_from_slice(b"--b\r\nContent-Type: application/octet-stream\r\n");
        m.extend_from_slice(
            format!("Content-Disposition: attachment; filename=\"f{i}\"\r\n\r\n").as_bytes(),
        );
        if unique {
            m.extend_from_slice(format!("part-{i}-unique\r\n").as_bytes());
        } else {
            m.extend_from_slice(b"x\r\n");
        }
    }
    m.extend_from_slice(b"--b--\r\n");
    m
}

fn seeded() -> (SqliteStore, tempfile::TempDir) {
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

fn ingest(store: &SqliteStore, raw: Vec<u8>) {
    absorb(
        store,
        ACCOUNT,
        MailboxRef {
            account: ACCOUNT,
            path: "INBOX".to_owned(),
        },
        Some(SyncCursor::Pop),
        vec![Arrival {
            remote: RemoteRef::Pop {
                uidl: "u1".to_owned(),
            },
            raw,
        }],
        false,
        now(),
    )
    .expect("a message with many parts is still a message");
}

fn blob_rows(store: &SqliteStore) -> i64 {
    store
        .connection()
        .query_row("SELECT count(*) FROM blobs", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn ten_thousand_parts_cost_ten_thousand_blobs_and_no_more() {
    // Linear, which is the property that matters: the work is bounded by the size of the
    // message, so the worst a sender can do is send a big message. Anything superlinear here
    // would be a message that costs more to receive than to write.
    let (store, _dir) = seeded();
    let raw = flood(10_000, true);
    let started = Instant::now();
    ingest(&store, raw);
    let elapsed = started.elapsed();

    let message = store
        .connection()
        .query_row("SELECT count(*) FROM messages", [], |r| r.get::<_, i64>(0))
        .unwrap();
    assert_eq!(message, 1, "one message, however many parts it has");
    // One blob per distinct part, plus the raw message.
    assert_eq!(blob_rows(&store), 10_001);
    // Measured at about 0.4s; this catches a change of complexity, not a slow machine.
    assert!(
        elapsed.as_secs() < 5,
        "ten thousand parts took {elapsed:?}, which is not linear any more"
    );
}

#[test]
fn ten_thousand_identical_parts_cost_one_blob() {
    // The blob store is content-addressed, so the obvious flood — one byte repeated — collapses
    // to a single stored part. Worth pinning: it is the difference between a hostile message
    // costing what it weighs and costing what it claims.
    let (store, _dir) = seeded();
    ingest(&store, flood(10_000, false));
    assert_eq!(
        blob_rows(&store),
        2,
        "the raw message and one shared part, not ten thousand copies"
    );
}

#[test]
fn every_part_is_still_reachable_afterwards() {
    // Cheapness must not have cost correctness: each of the ten thousand is a separate
    // attachment with its own name, even where the bytes are shared.
    let (store, _dir) = seeded();
    ingest(&store, flood(1_000, false));
    let id: String = store
        .connection()
        .query_row("SELECT id FROM messages", [], |r| r.get(0))
        .unwrap();
    let message = store
        .message(id.parse::<uuid::Uuid>().map(MessageId::from_uuid).unwrap())
        .unwrap();
    assert_eq!(message.attachments.len(), 1_000);
    assert_eq!(message.attachments[0].name, "f0");
    assert_eq!(message.attachments[999].name, "f999");
    assert_eq!(
        message.attachments[0].blob(),
        message.attachments[999].blob(),
        "identical bytes share a blob"
    );
}
