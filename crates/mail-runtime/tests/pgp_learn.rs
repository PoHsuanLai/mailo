//! Keys learnt from arriving mail, and secret keys in and out of the keyring.
//!
//! Autocrypt headers are read as mail is assembled, from the header alone: a sync never
//! decrypts. The keyring here is [`MapSecrets`], never the user's.

use chrono::{DateTime, TimeZone, Utc};
use mail_domain::*;
use mail_mime::openpgp::{self, SecretCert};
use mail_runtime::{Arrival, MapSecrets, assemble, pgp};
use mail_store::{SqliteStore, Store};
use rand::SeedableRng;

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));

fn day(n: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, n, 12, 0, 0).unwrap()
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

fn key_for(address: &str, seed: u64) -> SecretCert {
    openpgp::generate(
        &format!("Peer <{address}>"),
        day(1),
        &mut rand::rngs::StdRng::seed_from_u64(seed),
    )
    .unwrap()
}

/// A message from `from`, dated `date`, with an Autocrypt header for `key` when there is one.
fn message(from: &str, date: DateTime<Utc>, key: Option<&SecretCert>, id: &str) -> Vec<u8> {
    let raw = format!(
        "From: {from}\r\nTo: me@example.test\r\nSubject: hi\r\nMessage-ID: <{id}@example.test>\r\n\
         Date: {}\r\nContent-Type: text/plain\r\n\r\nhello\r\n",
        date.to_rfc2822()
    )
    .into_bytes();
    match key {
        Some(key) => openpgp::with_field(
            &raw,
            &openpgp::autocrypt_field("Autocrypt", from, PreferEncrypt::Mutual, &key.public()),
        ),
        None => raw,
    }
}

fn arrive(store: &SqliteStore, role: MailboxRole, raw: Vec<u8>, uidl: &str, now: DateTime<Utc>) {
    let ingest = assemble(
        store,
        ACCOUNT,
        MailboxRef {
            account: ACCOUNT,
            path: "INBOX".to_owned(),
        },
        role,
        None,
        vec![Arrival {
            remote: RemoteRef::Pop {
                uidl: uidl.to_owned(),
            },
            raw,
        }],
        now,
    )
    .unwrap();
    assert_eq!(ingest.messages.len(), 1, "the message itself still arrives");
}

#[test]
fn an_autocrypt_header_on_arriving_mail_keeps_the_senders_key() {
    let (store, _dir) = store();
    let peer = key_for("peer@example.test", 1);
    assert!(store.pgp_keys_for("peer@example.test").unwrap().is_empty());

    arrive(
        &store,
        MailboxRole::Inbox,
        message("peer@example.test", day(5), Some(&peer), "a"),
        "a",
        day(5),
    );

    let keys = store.pgp_keys_for("peer@example.test").unwrap();
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].fingerprint, peer.fingerprint());
    assert_eq!(keys[0].source, KeySource::Autocrypt);
    assert_eq!(keys[0].trust, KeyTrust::Unverified);
    let state = store.autocrypt_peer("peer@example.test").unwrap().unwrap();
    assert_eq!(state.key, Some(peer.fingerprint()));
    assert_eq!(state.autocrypt_timestamp, Some(day(5)));
    assert_eq!(state.prefer_encrypt, PreferEncrypt::Mutual);
}

#[test]
fn the_peer_state_follows_the_specs_order_rules_through_a_sync() {
    let (store, _dir) = store();
    let (first, second) = (
        key_for("peer@example.test", 2),
        key_for("peer@example.test", 3),
    );
    arrive(
        &store,
        MailboxRole::Inbox,
        message("peer@example.test", day(5), Some(&first), "a"),
        "a",
        day(5),
    );

    // Delivered late: dated before the header already recorded, so it changes nothing.
    arrive(
        &store,
        MailboxRole::Inbox,
        message("peer@example.test", day(4), Some(&second), "b"),
        "b",
        day(6),
    );
    let state = store.autocrypt_peer("peer@example.test").unwrap().unwrap();
    assert_eq!(state.key, Some(first.fingerprint()));
    assert!(
        store.pgp_key(second.fingerprint()).unwrap().is_none(),
        "the stale key is not kept"
    );

    // A later message without a header moves last_seen and leaves the key.
    arrive(
        &store,
        MailboxRole::Inbox,
        message("peer@example.test", day(8), None, "c"),
        "c",
        day(8),
    );
    let state = store.autocrypt_peer("peer@example.test").unwrap().unwrap();
    assert_eq!(state.last_seen, Some(day(8)));
    assert_eq!(state.key, Some(first.fingerprint()));

    // A newer header replaces the key.
    arrive(
        &store,
        MailboxRole::Inbox,
        message("peer@example.test", day(9), Some(&second), "d"),
        "d",
        day(9),
    );
    let state = store.autocrypt_peer("peer@example.test").unwrap().unwrap();
    assert_eq!(state.key, Some(second.fingerprint()));

    // A date in the future counts as the moment it arrived.
    let third = key_for("peer@example.test", 4);
    arrive(
        &store,
        MailboxRole::Inbox,
        message("peer@example.test", day(30), Some(&third), "e"),
        "e",
        day(10),
    );
    let state = store.autocrypt_peer("peer@example.test").unwrap().unwrap();
    assert_eq!(state.autocrypt_timestamp, Some(day(10)));
}

#[test]
fn our_own_sent_copies_teach_nothing_and_strangers_leave_no_state() {
    let (store, _dir) = store();
    let mine = key_for("me@example.test", 5);
    arrive(
        &store,
        MailboxRole::Sent,
        message("me@example.test", day(5), Some(&mine), "s"),
        "s",
        day(5),
    );
    assert!(store.pgp_keys().unwrap().is_empty());
    arrive(
        &store,
        MailboxRole::Inbox,
        message("news@example.test", day(5), None, "n"),
        "n",
        day(5),
    );
    assert_eq!(store.autocrypt_peer("news@example.test").unwrap(), None);
}

#[test]
fn gossip_is_kept_only_for_the_messages_own_recipients() {
    let (store, _dir) = store();
    let (bea, eve) = (
        key_for("bea@example.test", 6),
        key_for("eve@example.test", 7),
    );
    let gossip = [
        openpgp::AutocryptHeader {
            addr: "bea@example.test".to_owned(),
            prefer_encrypt: PreferEncrypt::NoPreference,
            key: bea.public(),
        },
        openpgp::AutocryptHeader {
            addr: "eve@example.test".to_owned(),
            prefer_encrypt: PreferEncrypt::NoPreference,
            key: eve.public(),
        },
    ];
    pgp::learn_gossip(
        &store,
        &gossip,
        &["me@example.test".to_owned(), "Bea@Example.TEST".to_owned()],
        Some(day(5)),
        day(5),
    )
    .unwrap();
    let kept = store.pgp_keys_for("bea@example.test").unwrap();
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].source, KeySource::Gossip);
    let state = store.autocrypt_peer("bea@example.test").unwrap().unwrap();
    assert_eq!(state.gossip_key, Some(bea.fingerprint()));
    assert_eq!(state.key, None, "gossip is not the peer's own header");
    assert!(
        store.pgp_keys_for("eve@example.test").unwrap().is_empty(),
        "not a recipient"
    );
}

#[test]
fn a_secret_key_goes_into_the_keyring_whole_and_comes_back_out() {
    let secrets = MapSecrets::default();
    let mine = key_for("me@example.test", 8);
    pgp::keep_secret_key(&secrets, ACCOUNT, &mine).unwrap();
    assert_eq!(
        pgp::secret_key(&secrets, ACCOUNT, mine.fingerprint()).unwrap(),
        mine
    );
    // Named by the key, not the account: another account's identity with the same key finds it.
    let other = AccountId::generate();
    assert_eq!(
        pgp::secret_key(&secrets, other, mine.fingerprint()).unwrap(),
        mine
    );
    pgp::forget_secret_key(&secrets, ACCOUNT, mine.fingerprint()).unwrap();
    assert!(pgp::secret_key(&secrets, ACCOUNT, mine.fingerprint()).is_err());
}
