//! `SqliteStore::sweep_blobs`: the stored bytes nothing refers to any more are taken back, once
//! they have gone unused for an hour, and nothing that is still named is.

use crate::support::*;
use chrono::{Duration, Utc};
use mail_domain::id::account_id_from_uuid;
use mail_domain::*;
use mail_store::testing::{seed_account, seed_identity_for};
use mail_store::{SqliteStore, Store};
use porter_core::AccountId;
use std::path::Path;

const IDENTITY: IdentityId =
    IdentityId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000b1"));

fn account() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000d1"))
}

fn opened(dir: &Path) -> SqliteStore {
    let store = SqliteStore::open(dir.join("mail.db"), dir.join("blobs")).unwrap();
    seed_account(&store, account(), "me@example.test");
    seed_identity_for(&store, IDENTITY, account(), "me@example.test", Some("Me"));
    store
}

/// Large enough to be a file rather than a row.
fn large(seed: u8) -> Vec<u8> {
    vec![seed; 64 * 1024]
}

fn kept(key: &str, raw: BlobId, parts: &[BlobId]) -> Kept {
    let key = MessageKey::Rfc(key.to_owned());
    Kept {
        key: key.clone(),
        raw,
        message: Message {
            id: MessageId::generate(),
            thread: ThreadId::generate(),
            account: account(),
            key,
            date: at(1),
            subject: "Subject".to_owned(),
            read: ReadState::Read,
            body: Body::Present { text: None, raw },
            attachments: parts
                .iter()
                .map(|blob| Attachment {
                    name: "part.bin".to_owned(),
                    mime: "application/octet-stream".to_owned(),
                    size: 1,
                    content: PartContent::Held(*blob),
                    inline: Inline::Attached,
                })
                .collect(),
            ..message(0)
        },
        labels: vec![],
    }
}

fn held(store: &SqliteStore, blob: BlobId) -> bool {
    store.blobs().get(blob).is_ok()
}

/// Make every blob look as old as one stored hours ago.
fn age_all(store: &SqliteStore) {
    store
        .raw_connection()
        .execute(
            "UPDATE blobs SET created_at = datetime('now', '-3 hours')",
            [],
        )
        .unwrap();
}

fn later() -> chrono::DateTime<Utc> {
    Utc::now() + Duration::hours(2)
}

/// Two sweeps a day apart, the second's, after the first has seen what is unnamed: what it takes
/// is only what both saw.
fn sweep_twice(store: &SqliteStore) -> mail_store::BlobsSwept {
    let first = store.sweep_blobs(later()).unwrap();
    assert_eq!(
        (first.blobs, first.files),
        (0, 0),
        "the first sweep takes nothing"
    );
    store.sweep_blobs(later() + Duration::hours(25)).unwrap()
}

#[test]
fn the_blobs_of_a_deleted_message_go_with_their_files_and_a_kept_message_keeps_its_own() {
    let dir = tempfile::tempdir().unwrap();
    let store = opened(dir.path());
    let gone_raw = store.blobs().put(&large(1)).unwrap();
    let gone_part = store.blobs().put(b"a small part").unwrap();
    let kept_raw = store.blobs().put(&large(2)).unwrap();
    let doomed = kept("gone@x", gone_raw, &[gone_part]);
    let doomed_id = doomed.message.id;
    store
        .import(
            account(),
            Import {
                messages: vec![doomed, kept("kept@x", kept_raw, &[])],
            },
        )
        .unwrap();
    store
        .apply(account(), &patch(vec![Change::MessageDelete(doomed_id)]))
        .unwrap();

    // Not yet an hour: it may be about to be named again.
    let nothing = store.sweep_blobs(Utc::now()).unwrap();
    assert_eq!(nothing.blobs, 0);
    assert!(held(&store, gone_raw));

    // Seen unnamed by one sweep is not enough; a day later, by a second, it is.
    let first = store.sweep_blobs(later()).unwrap();
    assert_eq!(first.blobs, 0);
    assert!(held(&store, gone_raw));
    let swept = store.sweep_blobs(later() + Duration::hours(25)).unwrap();

    assert_eq!((swept.blobs, swept.files, swept.left), (2, 1, 0));
    assert!(!held(&store, gone_raw));
    assert!(!held(&store, gone_part));
    assert!(held(&store, kept_raw));
    let files = std::fs::read_dir(dir.path().join("blobs"))
        .unwrap()
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .map(|entry| std::fs::read_dir(entry.path()).unwrap().count())
        .sum::<usize>();
    assert_eq!(files, 1, "only the kept message's file is left");
}

#[test]
fn a_blob_a_draft_or_a_queued_send_names_survives_and_one_nothing_names_does_not() {
    let dir = tempfile::tempdir().unwrap();
    let store = opened(dir.path());
    let in_draft = store.blobs().put(&large(3)).unwrap();
    let in_outbox = store.blobs().put(b"queued with a send").unwrap();
    let nowhere = store.blobs().put(b"nothing names this").unwrap();
    let draft = Draft {
        id: DraftId::generate(),
        account: account(),
        identity: IDENTITY,
        to: vec![],
        cc: vec![],
        bcc: vec![],
        subject: "with a file".to_owned(),
        in_reply_to: None,
        forward_of: None,
        text: String::new(),
        html: None,
        attachments: vec![PendingAttachment {
            name: "a.bin".to_owned(),
            mime: "application/octet-stream".to_owned(),
            blob: in_draft,
            cid: None,
        }],
        receipt: ReceiptRequest::default(),
        openpgp: None,
        smime: None,
        priority: Priority::default(),
        delivery: DeliveryReport::default(),
        automatic: false,
        state: SendState::Editing,
        updated: at(10),
    };
    store
        .apply(
            account(),
            &patch(vec![Change::DraftUpsert(Box::new(draft))]),
        )
        .unwrap();
    store
        .raw_connection()
        .execute(
            "INSERT INTO outbox (account, op, undo, attempts, next_attempt, created_at)
             VALUES (?1, ?2, '{}', 0, datetime('now'), datetime('now'))",
            [
                account().to_string(),
                format!("{{\"raw\":\"{in_outbox}\"}}"),
            ],
        )
        .unwrap();
    age_all(&store);

    sweep_twice(&store);

    assert!(held(&store, in_draft));
    assert!(held(&store, in_outbox));
    assert!(!held(&store, nowhere));
}

#[test]
fn a_blob_put_a_moment_ago_survives_though_nothing_names_it_yet() {
    let dir = tempfile::tempdir().unwrap();
    let store = opened(dir.path());
    let fresh = store.blobs().put(&large(4)).unwrap();

    let swept = store.sweep_blobs(Utc::now()).unwrap();
    let again = store.sweep_blobs(Utc::now() + Duration::hours(25)).unwrap();

    assert_eq!((swept.blobs, again.blobs), (0, 0));
    assert!(held(&store, fresh));
}

#[test]
fn bytes_put_again_are_in_use_again_however_long_they_sat() {
    let dir = tempfile::tempdir().unwrap();
    let store = opened(dir.path());
    let bytes = large(5);
    let id = store.blobs().put(&bytes).unwrap();
    store
        .raw_connection()
        .execute(
            "UPDATE blobs SET created_at = datetime('now', '-5 days')",
            [],
        )
        .unwrap();
    // Seen unnamed two days ago, so the next sweep would take it...
    let seen = store.sweep_blobs(Utc::now() - Duration::days(2)).unwrap();
    assert_eq!(seen.blobs, 0);

    // ...but a message about to name it, as a re-fetch is, puts it again first.
    assert_eq!(store.blobs().put(&bytes).unwrap(), id);
    store.sweep_blobs(Utc::now()).unwrap();

    assert!(held(&store, id));
}

#[test]
fn a_file_no_row_names_goes_once_it_is_an_hour_old_and_so_does_a_half_written_one() {
    let dir = tempfile::tempdir().unwrap();
    let store = opened(dir.path());
    let fan = dir.path().join("blobs/ab");
    std::fs::create_dir_all(&fan).unwrap();
    let orphan = fan.join("cdef0123");
    let half = fan.join(".cdef0124.part");
    std::fs::write(&orphan, b"left by a row that was never written").unwrap();
    std::fs::write(&half, b"cut off").unwrap();

    store.sweep_blobs(Utc::now()).unwrap();
    assert!(orphan.exists() && half.exists(), "an hour has not passed");

    // The half-written one is only an hour's wait; the orphan a day's more, over two sweeps.
    let swept = store.sweep_blobs(later()).unwrap();
    assert_eq!(swept.files, 1);
    assert!(orphan.exists() && !half.exists());
    let swept = store.sweep_blobs(later() + Duration::hours(25)).unwrap();
    assert_eq!(swept.files, 1);
    assert!(!orphan.exists());
}

#[test]
fn a_file_a_row_names_is_kept_however_old() {
    let dir = tempfile::tempdir().unwrap();
    let store = opened(dir.path());
    let held_file = store.blobs().put(&large(6)).unwrap();
    // Named by a message, so the row stays, and so must the file whatever its age.
    store
        .import(
            account(),
            Import {
                messages: vec![kept("named@x", held_file, &[])],
            },
        )
        .unwrap();
    age_all(&store);

    let swept = sweep_twice(&store);

    assert_eq!((swept.blobs, swept.files), (0, 0));
    assert!(held(&store, held_file));
}

#[test]
fn the_daily_sweep_runs_once_a_day() {
    let dir = tempfile::tempdir().unwrap();
    let store = opened(dir.path());
    let now = Utc::now();

    assert!(store.sweep_blobs_daily(now).unwrap().is_some());
    assert!(store.sweep_blobs_daily(now).unwrap().is_none());
    assert!(
        store
            .sweep_blobs_daily(now + Duration::hours(23))
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .sweep_blobs_daily(now + Duration::hours(25))
            .unwrap()
            .is_some()
    );
}

#[test]
fn a_blob_named_again_without_a_put_between_two_sweeps_survives() {
    let dir = tempfile::tempdir().unwrap();
    let store = opened(dir.path());
    let id = store.blobs().put(&large(7)).unwrap();
    age_all(&store);
    let first = store.sweep_blobs(later()).unwrap();
    assert_eq!(first.blobs, 0, "the first sweep ever takes nothing");

    // Named again by a row that reuses its id, as an undo restores one: no put.
    store
        .import(
            account(),
            Import {
                messages: vec![kept("restored@x", id, &[])],
            },
        )
        .unwrap();
    store.sweep_blobs(later() + Duration::hours(25)).unwrap();
    let third = store.sweep_blobs(later() + Duration::hours(50)).unwrap();

    assert_eq!(third.blobs, 0);
    assert!(held(&store, id));
}

#[test]
fn a_blob_unnamed_at_two_sweeps_a_day_apart_goes() {
    let dir = tempfile::tempdir().unwrap();
    let store = opened(dir.path());
    let id = store.blobs().put(&large(8)).unwrap();
    age_all(&store);

    let swept = sweep_twice(&store);

    assert_eq!((swept.blobs, swept.files), (1, 1));
    assert!(!held(&store, id));
}

#[test]
fn a_list_of_candidates_that_does_not_read_deletes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let store = opened(dir.path());
    let id = store.blobs().put(&large(9)).unwrap();
    age_all(&store);
    std::fs::write(
        dir.path().join("blobs/.sweep-candidates"),
        b"\xff\xfegarbage",
    )
    .unwrap();

    let swept = store.sweep_blobs(later() + Duration::hours(25)).unwrap();

    assert_eq!(swept.blobs, 0);
    assert!(held(&store, id));
}
