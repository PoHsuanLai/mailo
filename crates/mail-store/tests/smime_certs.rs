//! S/MIME certificates survive a round trip, in both [`Store`] implementations, and the two
//! agree — asked of both and compared, for the reason `pgp_keys.rs` gives. The certificate bytes
//! here are placeholders: the store keeps them and never reads them.

use chrono::{DateTime, TimeZone, Utc};
use mail_domain::*;
use mail_store::{MemoryStore, SqliteStore, Store, StoreError};

fn at(n: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000 + n, 0).unwrap()
}

fn fp(byte: u8) -> CertFingerprint {
    CertFingerprint([byte; 32])
}

fn cert(byte: u8, email: &str, source: CertSource, seen: i64, expires: i64) -> SmimeCert {
    SmimeCert {
        fingerprint: fp(byte),
        subject: format!("CN=Someone {byte},emailAddress={email}"),
        issuer: "CN=Example CA".to_owned(),
        serial: format!("{byte:02X}"),
        emails: vec![email.to_ascii_lowercase()],
        not_before: at(0),
        not_after: at(expires),
        der: vec![0x30, byte],
        chain: vec![vec![0x30, 0x01, byte]],
        source,
        first_seen: at(seen),
        last_seen: at(seen),
        trust: KeyTrust::Unverified,
        secret: SecretHeld::Absent,
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
fn a_certificate_round_trips_and_is_found_by_its_address_whole() {
    let stores = both();
    let kept = cert(1, "Peer@Example.TEST", CertSource::Received, 0, 100);
    assert_eq!(stores.agree(|s| s.smime_certs().unwrap()), vec![]);
    stores.agree(|s| s.put_smime_cert(kept.clone()).unwrap());

    assert_eq!(
        stores.agree(|s| s.smime_cert(fp(1)).unwrap()),
        Some(kept.clone())
    );
    assert_eq!(
        stores.agree(|s| s.smime_certs().unwrap()),
        vec![kept.clone()]
    );
    assert_eq!(
        stores.agree(|s| s.smime_certs_for("PEER@example.test").unwrap()),
        vec![kept.clone()]
    );
    for other in ["xpeer@example.test", "peer@example.test.evil", "peer"] {
        assert_eq!(
            stores.agree(|s| s.smime_certs_for(other).unwrap()),
            vec![],
            "{other}"
        );
    }
    assert_eq!(stores.agree(|s| s.smime_cert(fp(2)).unwrap()), None);
}

#[test]
fn keeping_a_certificate_again_merges_it_with_what_is_held() {
    let stores = both();
    let mut mine = cert(1, "me@example.test", CertSource::Identity, 0, 100);
    mine.secret = SecretHeld::Held;
    stores.agree(|s| s.put_smime_cert(mine.clone()).unwrap());
    stores.agree(|s| s.set_smime_trust(fp(1), KeyTrust::Verified).unwrap());

    // The same certificate arriving on a signed message of the user's own, later, with another
    // issuer beside it.
    let mut again = cert(1, "me@example.test", CertSource::Received, 50, 100);
    again.chain = vec![vec![0x30, 0x02]];
    let merged = stores.agree(|s| s.put_smime_cert(again.clone()).unwrap());
    assert_eq!(merged.source, CertSource::Identity);
    assert_eq!(merged.secret, SecretHeld::Held);
    assert_eq!(merged.trust, KeyTrust::Verified);
    assert_eq!(merged.first_seen, at(0));
    assert_eq!(merged.last_seen, at(50));
    assert_eq!(merged.chain, vec![vec![0x30, 0x01, 1], vec![0x30, 0x02]]);
    assert_eq!(stores.agree(|s| s.smime_cert(fp(1)).unwrap()), Some(merged));
}

#[test]
fn an_addresss_certificates_come_trusted_first_then_by_source_then_longest_valid() {
    let stores = both();
    let address = "peer@example.test";
    let received_short = cert(1, address, CertSource::Received, 90, 10);
    let received_long = cert(2, address, CertSource::Received, 10, 500);
    let imported = cert(3, address, CertSource::Imported, 0, 5);
    let trusted = cert(4, address, CertSource::Received, 0, 1);
    for c in [&received_short, &received_long, &imported, &trusted] {
        stores.agree(|s| s.put_smime_cert((*c).clone()).unwrap());
    }
    stores.agree(|s| s.set_smime_trust(fp(4), KeyTrust::Verified).unwrap());
    let order: Vec<CertFingerprint> = stores
        .agree(|s| s.smime_certs_for(address).unwrap())
        .into_iter()
        .map(|c| c.fingerprint)
        .collect();
    assert_eq!(order, vec![fp(4), fp(3), fp(2), fp(1)]);
}

#[test]
fn trust_on_a_missing_certificate_is_an_error_and_delete_says_whether_there_was_one() {
    let stores = both();
    for store in [&stores.sqlite as &dyn Store, &stores.memory] {
        assert!(matches!(
            store.set_smime_trust(fp(7), KeyTrust::Verified),
            Err(StoreError::NoSmimeCert(f)) if f == fp(7)
        ));
    }
    stores.agree(|s| {
        s.put_smime_cert(cert(7, "a@example.test", CertSource::Imported, 0, 9))
            .unwrap()
    });
    stores.agree(|s| s.set_smime_trust(fp(7), KeyTrust::Verified).unwrap());
    stores.agree(|s| s.set_smime_trust(fp(7), KeyTrust::Unverified).unwrap());
    assert_eq!(
        stores.agree(|s| s.smime_cert(fp(7)).unwrap().map(|c| c.trust)),
        Some(KeyTrust::Unverified)
    );
    assert!(stores.agree(|s| s.delete_smime_cert(fp(7)).unwrap()));
    assert!(!stores.agree(|s| s.delete_smime_cert(fp(7)).unwrap()));
    assert_eq!(stores.agree(|s| s.smime_certs().unwrap()), vec![]);
}

#[test]
fn a_draft_and_a_template_keep_what_they_ask_smime_to_do() {
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
    let mut draft = Draft::blank(
        &Identity {
            id: identity,
            account,
            from: Address {
                name: None,
                email: "me@example.test".to_owned(),
            },
            reply_to: None,
            signature: None,
            default: IsDefault::Default,
        },
        at(0),
    );
    draft.to = vec![Address {
        name: None,
        email: "you@example.test".to_owned(),
    }];
    draft.smime = Smime::SignAndEncrypt;
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
        assert_eq!(s.draft(draft.id).unwrap().smime, Smime::SignAndEncrypt);
        assert_eq!(s.draft(draft.id).unwrap().openpgp, OpenPgp::None);
    }
    let template = Template::from_draft(&draft, "sealed", at(1));
    for s in [&store as &dyn Store, &memory] {
        s.put_template(&template).unwrap();
        assert_eq!(
            s.template(template.id).unwrap().smime,
            Smime::SignAndEncrypt,
            "a template kept from an encrypted draft never starts a plain one"
        );
    }
}
