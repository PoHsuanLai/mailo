//! OpenPGP keys and Autocrypt peers survive a round trip, in both [`Store`] implementations,
//! and the two agree.
//!
//! Asked of both and compared, for the reason `templates.rs` gives: the parity proptest only
//! sees what a filter can ask, and a key is not something a filter can ask about. The key bytes
//! here are placeholders — the store keeps them and never reads them.

use chrono::{DateTime, TimeZone, Utc};
use mail_domain::*;
use mail_store::{MemoryStore, SqliteStore, Store, StoreError};

fn at(n: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000 + n, 0).unwrap()
}

fn fp(byte: u8) -> Fingerprint {
    Fingerprint::V4([byte; 20])
}

fn key(byte: u8, email: &str, source: KeySource, seen: i64) -> PgpKey {
    PgpKey {
        fingerprint: fp(byte),
        key_ids: vec![fp(byte).key_id(), KeyId([byte ^ 0xFF; 8])],
        user_ids: vec![format!("Someone <{email}>")],
        emails: vec![email.to_ascii_lowercase()],
        key: vec![0x98, byte, byte],
        source,
        first_seen: at(seen),
        last_seen: at(seen),
        trust: KeyTrust::Unverified,
        secret: SecretHeld::Absent,
        // The key's own dates round-trip too: an odd key expires, an even one does not.
        created: Some(at(seen - 1_000)),
        expires: (byte % 2 == 1).then(|| at(90_000)),
    }
}

struct Both {
    sqlite: SqliteStore,
    memory: MemoryStore,
    _dir: tempfile::TempDir,
}

fn both() -> Both {
    let dir = tempfile::tempdir().unwrap();
    Both {
        sqlite: SqliteStore::in_memory(dir.path()).unwrap(),
        memory: MemoryStore::new(),
        _dir: dir,
    }
}

impl Both {
    /// Run `f` on both stores and insist they answer alike.
    fn agree<T: PartialEq + std::fmt::Debug>(&self, f: impl Fn(&dyn Store) -> T) -> T {
        let (sqlite, memory) = (f(&self.sqlite), f(&self.memory));
        assert_eq!(sqlite, memory, "SQLite and memory disagree");
        sqlite
    }
}

#[test]
fn a_key_round_trips_and_is_found_by_address_and_by_any_of_its_ids() {
    let stores = both();
    let kept = key(1, "Peer@Example.TEST", KeySource::Imported, 0);
    stores.agree(|s| s.put_pgp_key(kept.clone()).unwrap());

    assert_eq!(
        stores.agree(|s| s.pgp_key(fp(1)).unwrap()),
        Some(kept.clone())
    );
    assert_eq!(stores.agree(|s| s.pgp_keys().unwrap()), vec![kept.clone()]);
    assert_eq!(
        stores.agree(|s| s.pgp_keys_for("PEER@example.test").unwrap()),
        vec![kept.clone()]
    );
    // An address that only contains the key's address is someone else.
    assert!(
        stores
            .agree(|s| s.pgp_keys_for("xpeer@example.test").unwrap())
            .is_empty()
    );
    for id in &kept.key_ids {
        assert_eq!(
            stores.agree(|s| s.pgp_keys_by_id(*id).unwrap()),
            vec![kept.clone()]
        );
    }
    assert!(
        stores
            .agree(|s| s.pgp_keys_by_id(KeyId([9; 8])).unwrap())
            .is_empty()
    );
    assert_eq!(stores.agree(|s| s.pgp_key(fp(2)).unwrap()), None);
}

#[test]
fn keeping_a_key_again_merges_it_with_what_is_held() {
    let stores = both();
    let mut mine = key(1, "me@example.test", KeySource::Generated, 0);
    mine.secret = SecretHeld::Held;
    stores.agree(|s| s.put_pgp_key(mine.clone()).unwrap());
    stores.agree(|s| s.set_pgp_trust(fp(1), KeyTrust::Verified).unwrap());

    // The same key seen again in someone's gossip, later, with another address on it.
    let mut again = key(1, "alias@example.test", KeySource::Gossip, 50);
    again.key = vec![0x98, 0xEE];
    let merged = stores.agree(|s| s.put_pgp_key(again.clone()).unwrap());
    assert_eq!(
        merged.key, mine.key,
        "a gossiped copy never replaces the owner's"
    );
    assert_eq!(merged.source, KeySource::Generated);
    assert_eq!(merged.first_seen, at(0));
    assert_eq!(merged.last_seen, at(50));
    assert_eq!(merged.trust, KeyTrust::Verified);
    assert_eq!(merged.secret, SecretHeld::Held);
    assert_eq!(merged.emails, vec!["me@example.test", "alias@example.test"]);
    assert_eq!(stores.agree(|s| s.pgp_key(fp(1)).unwrap()), Some(merged));
}

#[test]
fn an_addresss_keys_come_verified_first_then_by_source_then_newest() {
    let stores = both();
    let address = "peer@example.test";
    let gossiped = key(1, address, KeySource::Gossip, 90);
    let autocrypt_old = key(2, address, KeySource::Autocrypt, 10);
    let autocrypt_new = key(3, address, KeySource::Autocrypt, 20);
    let verified = key(4, address, KeySource::Gossip, 0);
    for k in [&gossiped, &autocrypt_old, &autocrypt_new, &verified] {
        stores.agree(|s| s.put_pgp_key((*k).clone()).unwrap());
    }
    stores.agree(|s| s.set_pgp_trust(fp(4), KeyTrust::Verified).unwrap());
    let order: Vec<Fingerprint> = stores
        .agree(|s| s.pgp_keys_for(address).unwrap())
        .into_iter()
        .map(|k| k.fingerprint)
        .collect();
    assert_eq!(order, vec![fp(4), fp(3), fp(2), fp(1)]);
}

#[test]
fn trust_on_a_missing_key_is_an_error_and_delete_says_whether_there_was_one() {
    let stores = both();
    for store in [&stores.sqlite as &dyn Store, &stores.memory] {
        assert!(matches!(
            store.set_pgp_trust(fp(7), KeyTrust::Verified),
            Err(StoreError::NoPgpKey(f)) if f == fp(7)
        ));
    }
    stores.agree(|s| {
        s.put_pgp_key(key(7, "a@example.test", KeySource::Wkd, 0))
            .unwrap()
    });
    assert!(stores.agree(|s| s.delete_pgp_key(fp(7)).unwrap()));
    assert!(!stores.agree(|s| s.delete_pgp_key(fp(7)).unwrap()));
    assert!(stores.agree(|s| s.pgp_keys().unwrap()).is_empty());
}

#[test]
fn autocrypt_peer_state_round_trips_by_address_in_any_case() {
    let stores = both();
    assert_eq!(
        stores.agree(|s| s.autocrypt_peer("peer@example.test").unwrap()),
        None
    );
    let peer = AutocryptPeer {
        address: "peer@example.test".to_owned(),
        last_seen: Some(at(5)),
        autocrypt_timestamp: Some(at(4)),
        key: Some(fp(3)),
        prefer_encrypt: PreferEncrypt::Mutual,
        gossip_timestamp: Some(at(6)),
        gossip_key: Some(fp(8)),
    };
    stores.agree(|s| s.put_autocrypt_peer(&peer).unwrap());
    assert_eq!(
        stores.agree(|s| s.autocrypt_peer("Peer@Example.TEST").unwrap()),
        Some(peer.clone())
    );
    let gossip_only = AutocryptPeer {
        address: "other@example.test".to_owned(),
        last_seen: None,
        autocrypt_timestamp: None,
        key: None,
        prefer_encrypt: PreferEncrypt::NoPreference,
        gossip_timestamp: Some(at(1)),
        gossip_key: Some(fp(9)),
    };
    stores.agree(|s| s.put_autocrypt_peer(&gossip_only).unwrap());
    assert_eq!(
        stores.agree(|s| s.autocrypt_peer("other@example.test").unwrap()),
        Some(gossip_only)
    );
    // Replaced, not merged: the update rules are the domain's, and the store keeps their result.
    let replaced = AutocryptPeer {
        key: Some(fp(4)),
        ..peer
    };
    stores.agree(|s| s.put_autocrypt_peer(&replaced).unwrap());
    assert_eq!(
        stores.agree(|s| s.autocrypt_peer("peer@example.test").unwrap()),
        Some(replaced)
    );
}

#[test]
fn a_draft_keeps_what_it_asks_openpgp_to_do() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::in_memory(dir.path()).unwrap();
    let account = AccountId::generate();
    let identity = IdentityId::generate();
    {
        let db = store.connection();
        db.execute(
            "INSERT INTO accounts (id, address, plan, created_at)
             VALUES (?1, 'me@example.test', '{}', datetime('now'))",
            [account.to_string()],
        )
        .unwrap();
        db.execute(
            "INSERT INTO identities (id, account, from_name, from_email, is_default)
             VALUES (?1, ?2, 'Me', 'me@example.test', '\"default\"')",
            [identity.to_string(), account.to_string()],
        )
        .unwrap();
    }
    let draft = Draft {
        id: DraftId::generate(),
        account,
        identity,
        to: vec![Address {
            name: None,
            email: "you@example.test".to_owned(),
        }],
        cc: Vec::new(),
        bcc: Vec::new(),
        subject: "sealed".to_owned(),
        in_reply_to: None,
        forward_of: None,
        text: "hi".to_owned(),
        html: None,
        attachments: Vec::new(),
        receipt: ReceiptRequest::Unrequested,
        openpgp: OpenPgp::SignAndEncrypt,
        smime: mail_domain::Smime::None,
        state: SendState::Editing,
        updated: at(0),
    };
    let memory = MemoryStore::new();
    for s in [&store as &dyn Store, &memory] {
        s.apply(
            account,
            &Patch {
                id: ChangeId::generate(),
                changes: vec![Change::DraftUpsert(Box::new(draft.clone()))],
            },
        )
        .unwrap();
        assert_eq!(s.draft(draft.id).unwrap().openpgp, OpenPgp::SignAndEncrypt);
    }
}
