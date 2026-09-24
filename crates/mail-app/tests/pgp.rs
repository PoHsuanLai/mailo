//! OpenPGP from the user's side (`plan.md` 10.15): keys made and moved, drafts sent signed and
//! encrypted, protected mail opened when it is read — against a real store, with the keyring a
//! [`MapSecrets`] so the user's own is never touched.

use chrono::{DateTime, TimeZone, Utc};
use mail_app::{cli, compose, pgp};
use mail_domain::*;
use mail_mime::openpgp::{self, Keys, SecretCert, Unlocking};
use mail_runtime::{Arrival, MapSecrets, Secrets};
use mail_store::{SqliteStore, Store};
use rand::SeedableRng;

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));
const IDENTITY: IdentityId =
    IdentityId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000b1"));
const ME: &str = "me@example.test";
const BEA: &str = "bea@example.test";

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 24, 17, 0, 0).unwrap()
}

fn seed(store: &SqliteStore) {
    let db = store.connection();
    db.execute(
        "INSERT INTO accounts (id, address, plan, created_at)
         VALUES (?1, ?2, '{}', datetime('now'))",
        [ACCOUNT.to_string(), ME.to_owned()],
    )
    .unwrap();
    db.execute(
        "INSERT INTO identities (id, account, from_name, from_email, is_default)
         VALUES (?1, ?2, 'Me', ?3, '\"default\"')",
        [IDENTITY.to_string(), ACCOUNT.to_string(), ME.to_owned()],
    )
    .unwrap();
}

/// A store with one account that can send, as `me@example.test`.
fn seeded() -> (SqliteStore, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::in_memory(dir.path()).unwrap();
    seed(&store);
    (store, dir)
}

/// The same, with a key of the user's own already made.
fn with_key() -> (SqliteStore, tempfile::TempDir, MapSecrets, PgpKey) {
    let (store, dir) = seeded();
    let secrets = MapSecrets::default();
    let key = pgp::keys::generate(&store, &secrets, ME, now()).unwrap();
    (store, dir, secrets, key)
}

/// A correspondent's key, made outside this client.
fn someone_elses(address: &str, seed: u64) -> SecretCert {
    openpgp::generate(
        &format!("Them <{address}>"),
        now(),
        &mut rand::rngs::StdRng::seed_from_u64(seed),
    )
    .unwrap()
}

/// Import `key`'s public half, as `mailo pgp import` would.
fn import_public(store: &SqliteStore, secrets: &MapSecrets, key: &SecretCert) {
    let armored = key.public().armored().unwrap();
    pgp::keys::import(store, secrets, armored.as_bytes(), now()).unwrap();
}

fn to(addresses: &[&str]) -> Vec<Address> {
    addresses
        .iter()
        .map(|email| Address {
            name: None,
            email: (*email).to_owned(),
        })
        .collect()
}

fn draft(store: &SqliteStore, openpgp: OpenPgp, recipients: &[&str], bcc: &[&str]) -> Draft {
    let mut draft = compose::draft_new(
        store,
        ACCOUNT,
        &to(recipients),
        "Secret plans",
        "meet at the usual place",
        now(),
    )
    .unwrap();
    draft.openpgp = openpgp;
    draft.bcc = to(bcc);
    compose::save(store, &draft).unwrap();
    draft
}

fn submissions(store: &SqliteStore) -> Vec<mail_store::OutboxEntry> {
    store
        .outbox_due(ACCOUNT, now() + chrono::TimeDelta::try_days(365).unwrap())
        .unwrap()
}

/// The frozen bytes of the only queued submission.
fn frozen(store: &SqliteStore) -> Vec<u8> {
    let entries = submissions(store);
    assert_eq!(entries.len(), 1, "one submission queued");
    let ProtoOp::Submit { raw, .. } = &entries[0].op else {
        panic!("{:?}", entries[0].op);
    };
    store.blobs().get(&store.connection(), *raw).unwrap()
}

fn send(store: &SqliteStore, secrets: &MapSecrets, draft: DraftId) -> Result<String, String> {
    compose::send_with(store, secrets, &pgp::no_passphrase, draft, now())
}

/// Store `raw` as a message that arrived in the inbox, the way a sync does, and return it.
fn arrive(store: &SqliteStore, raw: Vec<u8>) -> Message {
    let ingest = mail_runtime::assemble(
        store,
        ACCOUNT,
        MailboxRef {
            account: ACCOUNT,
            path: "INBOX".to_owned(),
        },
        MailboxRole::Inbox,
        None,
        vec![Arrival {
            remote: RemoteRef::Pop {
                uidl: format!("{}", raw.len()),
            },
            raw,
        }],
        now(),
    )
    .unwrap();
    let id = ingest.messages[0].message.id;
    store.ingest(ACCOUNT, ingest).unwrap();
    store.message(id).unwrap()
}

mod keys {
    use super::*;

    #[test]
    fn a_generated_key_keeps_its_secret_in_the_keyring_and_its_public_half_in_the_store() {
        let (store, _dir) = seeded();
        let secrets = MapSecrets::default();
        assert!(store.pgp_keys().unwrap().is_empty());
        let key = pgp::keys::generate(&store, &secrets, ME, now()).unwrap();
        assert_eq!(key.source, KeySource::Generated);
        assert_eq!(key.secret, SecretHeld::Held);
        assert_eq!(key.emails, vec![ME]);
        assert_eq!(key.user_ids, vec!["Me <me@example.test>"]);
        assert_eq!(store.pgp_keys().unwrap(), vec![key.clone()]);
        let held = secrets
            .get(&SecretKey {
                account: ACCOUNT,
                purpose: SecretPurpose::OpenPgp(key.fingerprint),
            })
            .unwrap();
        assert!(matches!(held, Credential::OpenPgp(armored) if armored.contains("PRIVATE KEY")));
        assert_eq!(pgp::own_key(&store, ME).unwrap(), Some(key));
    }

    #[test]
    fn an_identity_with_a_key_is_not_given_a_second_and_a_stranger_gets_none() {
        let (store, _dir, secrets, key) = with_key();
        let again = pgp::keys::generate(&store, &secrets, "ME@example.test", now()).unwrap_err();
        assert!(
            matches!(again, pgp::PgpError::AlreadyHasKey { fingerprint, .. } if fingerprint == key.fingerprint)
        );
        assert!(matches!(
            pgp::keys::generate(&store, &secrets, "stranger@example.test", now()),
            Err(pgp::PgpError::NoIdentity(_))
        ));
        assert_eq!(store.pgp_keys().unwrap().len(), 1);
    }

    #[test]
    fn an_exported_key_imports_into_another_client_as_itself() {
        let (store, _dir, secrets, key) = with_key();
        let public = pgp::run(
            &store,
            &secrets,
            &pgp::parse(&["export".to_owned(), ME.to_owned()]).unwrap(),
            now(),
        )
        .unwrap();
        assert!(public.starts_with("-----BEGIN PGP PUBLIC KEY BLOCK-----"));
        let secret = pgp::run(
            &store,
            &secrets,
            &pgp::parse(&[
                "export".to_owned(),
                key.fingerprint.to_string(),
                "--secret".to_owned(),
            ])
            .unwrap(),
            now(),
        )
        .unwrap();
        assert!(
            secret.starts_with("WARNING: this is the SECRET half"),
            "{secret}"
        );
        assert!(secret.contains("-----BEGIN PGP PRIVATE KEY BLOCK-----"));

        // A second client, the same person: the public key alone, then the secret too.
        let (other, _dir2) = seeded();
        let other_secrets = MapSecrets::default();
        let imported = pgp::keys::import(&other, &other_secrets, public.as_bytes(), now()).unwrap();
        assert_eq!(imported.len(), 1);
        assert_eq!(imported[0].key.fingerprint, key.fingerprint);
        assert_eq!(imported[0].key.secret, SecretHeld::Absent);
        assert_eq!(imported[0].secret, None);
        // The warning above the armor does not stop the file importing.
        let imported = pgp::keys::import(&other, &other_secrets, secret.as_bytes(), now()).unwrap();
        assert_eq!(imported[0].secret, Some(openpgp::Protection::Open));
        let kept = other.pgp_key(key.fingerprint).unwrap().unwrap();
        assert_eq!(kept.secret, SecretHeld::Held);
        assert_eq!(kept.source, KeySource::Imported);
    }

    #[test]
    fn a_secret_key_for_none_of_the_users_addresses_is_refused() {
        let (store, _dir) = seeded();
        let secrets = MapSecrets::default();
        let theirs = someone_elses(BEA, 1).armored().unwrap();
        assert!(matches!(
            pgp::keys::import(&store, &secrets, theirs.as_bytes(), now()),
            Err(pgp::PgpError::NotYours { .. })
        ));
        assert!(store.pgp_keys().unwrap().is_empty());
    }

    #[test]
    fn deleting_a_key_whose_secret_is_held_needs_saying_so() {
        let (store, _dir, secrets, key) = with_key();
        let refused = pgp::keys::delete(&store, &secrets, &key, pgp::WithSecret::Refuse);
        assert!(
            matches!(refused, Err(pgp::PgpError::SecretWouldBeLost(f)) if f == key.fingerprint)
        );
        assert!(
            store.pgp_key(key.fingerprint).unwrap().is_some(),
            "still there"
        );
        pgp::keys::delete(&store, &secrets, &key, pgp::WithSecret::Confirmed).unwrap();
        assert!(store.pgp_key(key.fingerprint).unwrap().is_none());
        assert!(mail_runtime::pgp::secret_key(&secrets, ACCOUNT, key.fingerprint).is_err());

        // A correspondent's key needs no confirmation: nothing is lost that cannot be fetched again.
        let bea = someone_elses(BEA, 2);
        import_public(&store, &secrets, &bea);
        let theirs = store.pgp_key(bea.fingerprint()).unwrap().unwrap();
        pgp::keys::delete(&store, &secrets, &theirs, pgp::WithSecret::Refuse).unwrap();
        assert!(store.pgp_keys().unwrap().is_empty());
    }

    #[test]
    fn verify_marks_a_key_as_checked_by_the_user() {
        let (store, _dir, secrets, _) = with_key();
        let bea = someone_elses(BEA, 3);
        import_public(&store, &secrets, &bea);
        let before = store.pgp_key(bea.fingerprint()).unwrap().unwrap().trust;
        let said = pgp::run(
            &store,
            &secrets,
            &pgp::parse(&["verify".to_owned(), bea.fingerprint().to_string()]).unwrap(),
            now(),
        )
        .unwrap();
        assert!(said.contains("marked"));
        assert_eq!(before, KeyTrust::Unverified);
        assert_eq!(
            store.pgp_key(bea.fingerprint()).unwrap().unwrap().trust,
            KeyTrust::Verified
        );
        let listing = pgp::run(&store, &secrets, &pgp::PgpCommand::Keys, now()).unwrap();
        assert!(listing.contains("verified") && listing.contains(BEA) && listing.contains("yours"));
    }

    #[test]
    fn no_secret_key_is_ever_written_to_the_database_file() {
        // A database on disk, as the application keeps it, WAL and all.
        let dir = tempfile::tempdir().unwrap();
        let blobs = dir.path().join("blobs");
        std::fs::create_dir_all(&blobs).unwrap();
        let store = SqliteStore::open(dir.path().join("mail.db"), &blobs).unwrap();
        seed(&store);
        let secrets = MapSecrets::default();
        let generated = pgp::keys::generate(&store, &secrets, ME, now()).unwrap();
        // And a secret imported with a passphrase, for the address of a second identity.
        store
            .connection()
            .execute(
                "INSERT INTO identities (id, account, from_name, from_email, is_default)
                 VALUES (?1, ?2, NULL, 'alias@example.test', '\"alternate\"')",
                [IdentityId::generate().to_string(), ACCOUNT.to_string()],
            )
            .unwrap();
        let locked = someone_elses("alias@example.test", 4)
            .with_passphrase("pw", &mut rand::rngs::StdRng::seed_from_u64(5))
            .unwrap();
        pgp::keys::import(
            &store,
            &secrets,
            locked.armored().unwrap().as_bytes(),
            now(),
        )
        .unwrap();
        drop(store);

        let mut bytes = Vec::new();
        for entry in std::fs::read_dir(dir.path()).unwrap() {
            let path = entry.unwrap().path();
            if path.is_file() {
                bytes.extend(std::fs::read(&path).unwrap());
            }
        }
        for entry in std::fs::read_dir(&blobs).unwrap() {
            let path = entry.unwrap().path();
            if path.is_file() {
                bytes.extend(std::fs::read(&path).unwrap());
            }
        }
        let contains = |needle: &[u8]| bytes.windows(needle.len()).any(|w| w == needle);
        // The scan reads what the store wrote: the public key is there to be found.
        let public = openpgp::Cert::from_bytes(&generated.key)
            .unwrap()
            .to_bytes();
        assert!(contains(&public[..64]), "the public key is in the database");
        assert!(!contains(b"PRIVATE KEY"), "no armored secret key");
        let secret = mail_runtime::pgp::secret_key(&secrets, ACCOUNT, generated.fingerprint)
            .unwrap()
            .armored()
            .unwrap();
        for line in secret.lines().filter(|l| l.len() > 40) {
            assert!(
                !contains(line.as_bytes()),
                "a line of the secret key's armor: {line}"
            );
        }
    }
}

mod sending {
    use super::*;

    #[test]
    fn a_signed_send_freezes_pgp_mime_and_still_verifies_after_the_date_is_restamped() {
        let (store, _dir, secrets, key) = with_key();
        let draft = draft(&store, OpenPgp::Sign, &[BEA], &[]);
        send(&store, &secrets, draft.id).unwrap();
        let sealed = frozen(&store);
        let text = String::from_utf8_lossy(&sealed);
        assert!(text.contains("Content-Type: multipart/signed; micalg=pgp-sha256"));
        assert!(
            text.contains("Subject: Secret plans"),
            "signing does not hide the subject"
        );

        // The outbox rewrites `Date` as the message leaves; the signature must survive it.
        let leaving = Utc.with_ymd_and_hms(2026, 9, 25, 9, 0, 0).unwrap();
        let stamped = mail_mime::restamp(&sealed, leaving);
        assert_eq!(mail_mime::parse(&stamped).unwrap().date, Some(leaving));
        let cert = openpgp::Cert::from_bytes(&key.key).unwrap();
        let opened = openpgp::open(
            &stamped,
            &Keys {
                certs: vec![openpgp::KnownCert {
                    cert,
                    trust: KeyTrust::Unverified,
                }],
                secrets: Vec::new(),
            },
        )
        .unwrap();
        assert!(
            matches!(opened.verification, Verification::Good { signer, coverage: Coverage::Whole, .. } if signer == key.fingerprint),
            "{:?}",
            opened.verification
        );
    }

    #[test]
    fn plain_mail_from_an_identity_with_a_key_carries_its_autocrypt_header() {
        let (store, _dir) = seeded();
        let secrets = MapSecrets::default();
        let before = draft(&store, OpenPgp::None, &[BEA], &[]);
        send(&store, &secrets, before.id).unwrap();
        assert!(!String::from_utf8_lossy(&frozen(&store)).contains("Autocrypt:"));
        store
            .connection()
            .execute("DELETE FROM outbox", [])
            .unwrap();

        let key = pgp::keys::generate(&store, &secrets, ME, now()).unwrap();
        let after = draft(&store, OpenPgp::None, &[BEA], &[]);
        send(&store, &secrets, after.id).unwrap();
        let sent = frozen(&store);
        let header = openpgp::autocrypt_of(&sent, ME).expect("an Autocrypt header for the sender");
        assert_eq!(header.key.fingerprint(), key.fingerprint);
        assert_eq!(
            mail_mime::parse(&sent).unwrap().text.unwrap().trim_end(),
            "meet at the usual place",
            "not signed or encrypted: only the header was added"
        );
    }

    #[test]
    fn an_encrypted_send_to_someone_with_no_key_is_refused_by_name_and_nothing_is_queued() {
        let (store, _dir, secrets, _) = with_key();
        let bea = someone_elses(BEA, 6);
        import_public(&store, &secrets, &bea);
        let draft = draft(
            &store,
            OpenPgp::Encrypt,
            &[BEA, "cara@example.test", "dee@example.test"],
            &[],
        );
        let refused = send(&store, &secrets, draft.id).unwrap_err();
        assert!(
            refused.contains("cara@example.test, dee@example.test"),
            "{refused}"
        );
        assert!(!refused.contains(BEA), "bea has a key: {refused}");
        assert!(submissions(&store).is_empty(), "nothing went to the outbox");
        assert_eq!(store.draft(draft.id).unwrap().state, SendState::Editing);
    }

    #[test]
    fn an_encrypted_message_cannot_have_blind_recipients() {
        let (store, _dir, secrets, _) = with_key();
        for (address, seed) in [(BEA, 7), ("blind@example.test", 8)] {
            import_public(&store, &secrets, &someone_elses(address, seed));
        }
        let draft = draft(
            &store,
            OpenPgp::SignAndEncrypt,
            &[BEA],
            &["blind@example.test"],
        );
        let refused = send(&store, &secrets, draft.id).unwrap_err();
        assert!(
            refused.contains("Bcc") && refused.contains("blind@example.test"),
            "{refused}"
        );
        assert!(submissions(&store).is_empty());

        // Signing alone names no keys, so a blind copy is fine.
        let signed = super::draft(&store, OpenPgp::Sign, &[BEA], &["blind@example.test"]);
        send(&store, &secrets, signed.id).unwrap();
        assert_eq!(submissions(&store).len(), 1);
    }

    #[test]
    fn an_encrypted_send_reaches_the_recipient_and_the_senders_own_copy_reads_back() {
        let (store, _dir, secrets, key) = with_key();
        let (bea, cara) = (
            someone_elses(BEA, 9),
            someone_elses("cara@example.test", 10),
        );
        import_public(&store, &secrets, &bea);
        import_public(&store, &secrets, &cara);
        let draft = draft(
            &store,
            OpenPgp::SignAndEncrypt,
            &[BEA, "cara@example.test"],
            &[],
        );
        let said = send(&store, &secrets, draft.id).unwrap();
        assert!(said.starts_with("queued"), "{said}");
        let sealed = frozen(&store);
        let wire = String::from_utf8_lossy(&sealed);
        assert!(!wire.contains("meet at the usual place") && !wire.contains("Secret plans"));
        assert!(wire.contains("Autocrypt: addr=me@example.test"));

        // Bea reads it with her own key, sees it signed by me, and hears cara's key by gossip.
        let hers = openpgp::open(
            &sealed,
            &Keys {
                certs: vec![openpgp::KnownCert {
                    cert: openpgp::Cert::from_bytes(&key.key).unwrap(),
                    trust: KeyTrust::Unverified,
                }],
                secrets: vec![Unlocking {
                    key: bea.clone(),
                    passphrase: String::new(),
                }],
            },
        )
        .unwrap();
        assert_eq!(hers.encryption, Encryption::Decrypted);
        assert!(
            matches!(hers.verification, Verification::Good { signer, .. } if signer == key.fingerprint)
        );
        let gossiped: Vec<&str> = hers.gossip.iter().map(|g| g.addr.as_str()).collect();
        assert_eq!(gossiped, vec![BEA, "cara@example.test"]);

        // The copy the server files in Sent is readable by the sender, through the reader's API.
        let copy = arrive(&store, sealed);
        let opened = pgp::open_message(&store, &secrets, &copy, &pgp::no_passphrase, now())
            .unwrap()
            .unwrap();
        assert_eq!(opened.encryption, Encryption::Decrypted);
        let shown = opened.shown.unwrap();
        assert_eq!(shown.subject, "Secret plans");
        assert_eq!(shown.text.unwrap().trim_end(), "meet at the usual place");
        assert!(matches!(
            opened.verification,
            Verification::Good {
                trust: KeyTrust::Unverified,
                ..
            }
        ));
        assert_eq!(opened.signer.map(|k| k.fingerprint), Some(key.fingerprint));
    }

    #[test]
    fn a_passphrase_protected_key_is_asked_for_it_and_the_send_waits_without_it() {
        let (store, _dir) = seeded();
        let secrets = MapSecrets::default();
        let locked = someone_elses(ME, 11)
            .with_passphrase("pw", &mut rand::rngs::StdRng::seed_from_u64(12))
            .unwrap();
        pgp::keys::import(
            &store,
            &secrets,
            locked.armored().unwrap().as_bytes(),
            now(),
        )
        .unwrap();
        let draft = draft(&store, OpenPgp::Sign, &[BEA], &[]);

        let refused = send(&store, &secrets, draft.id).unwrap_err();
        assert!(refused.contains("needs its passphrase"), "{refused}");
        let wrong = compose::send_with(
            &store,
            &secrets,
            &|_| Some("nope".to_owned()),
            draft.id,
            now(),
        )
        .unwrap_err();
        assert!(wrong.contains("needs its passphrase"), "{wrong}");
        assert!(submissions(&store).is_empty());

        let asked = std::cell::Cell::new(0);
        let ask = |fingerprint: Fingerprint| {
            asked.set(asked.get() + 1);
            (fingerprint == locked.fingerprint()).then(|| "pw".to_owned())
        };
        compose::send_with(&store, &secrets, &ask, draft.id, now()).unwrap();
        assert_eq!(asked.get(), 1, "asked once");
        assert_eq!(submissions(&store).len(), 1);
    }

    #[test]
    fn a_draft_that_cannot_be_sent_as_it_asks_says_so_when_it_is_made() {
        let (store, _dir) = seeded();
        let said = compose::new_sealed_message(
            &store,
            None,
            [&to(&[BEA]), &[], &[]],
            "hi",
            "body",
            (ReceiptRequest::Unrequested, OpenPgp::SignAndEncrypt),
            now(),
        )
        .unwrap();
        assert!(said.contains("signed and encrypted with OpenPGP"), "{said}");
        assert!(
            said.contains("has no OpenPGP key; make one with `mailo pgp generate"),
            "{said}"
        );
    }
}

mod reading {
    use super::*;

    /// A message from bea to me, encrypted and signed, as her client would send it.
    fn from_bea(bea: &SecretCert, to_key: &openpgp::Cert, secret_word: &str) -> Vec<u8> {
        let raw = format!(
            "From: Bea <bea@example.test>\r\nTo: me@example.test\r\nSubject: {secret_word} plan\r\n\
             Message-ID: <{secret_word}@example.test>\r\nDate: Thu, 24 Sep 2026 16:00:00 +0000\r\n\
             MIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n\
             the {secret_word} is under the mat\r\n"
        );
        let signer = Unlocking {
            key: bea.clone(),
            passphrase: String::new(),
        };
        openpgp::seal(
            raw.as_bytes(),
            &openpgp::Sealing {
                mode: OpenPgp::SignAndEncrypt,
                signer: Some(&signer),
                recipients: std::slice::from_ref(to_key),
                gossip: &[],
                now: now(),
            },
            &mut rand::rngs::StdRng::seed_from_u64(13),
        )
        .unwrap()
    }

    fn text_search(store: &SqliteStore, word: &str) -> usize {
        store
            .threads(
                &Query {
                    filter: Filter::Text(TextMatch::Contains(word.to_owned())),
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
            .len()
    }

    #[test]
    fn an_encrypted_message_opens_when_read_and_its_text_never_reaches_the_index() {
        let (store, _dir, secrets, key) = with_key();
        let bea = someone_elses(BEA, 14);
        let mine = openpgp::Cert::from_bytes(&key.key).unwrap();
        let message = arrive(&store, from_bea(&bea, &mine, "zebra"));

        // What the sync stored and indexed: the outside only.
        assert_eq!(message.subject, "...");
        assert!(!message.body.text().unwrap_or_default().contains("zebra"));
        assert_eq!(
            text_search(&store, "zebra"),
            0,
            "the decrypted word is not searchable"
        );
        assert_eq!(text_search(&store, "mat"), 0);

        let opened = pgp::open_message(&store, &secrets, &message, &pgp::no_passphrase, now())
            .unwrap()
            .unwrap();
        assert_eq!(opened.encryption, Encryption::Decrypted);
        let shown = opened.shown.clone().unwrap();
        assert_eq!(shown.subject, "zebra plan");
        assert_eq!(shown.text.unwrap().trim_end(), "the zebra is under the mat");
        // Bea's key is not held, so her signature can only be named.
        assert_eq!(
            opened.verification,
            Verification::UnknownKey {
                issuer: bea.fingerprint().key_id(),
                coverage: Coverage::Whole,
            }
        );
        // Reading it wrote nothing that search can see.
        assert_eq!(text_search(&store, "zebra"), 0);

        // With her key imported, the same signature is good.
        import_public(&store, &secrets, &bea);
        let again = pgp::open_message(&store, &secrets, &message, &pgp::no_passphrase, now())
            .unwrap()
            .unwrap();
        assert!(
            matches!(again.verification, Verification::Good { signer, .. } if signer == bea.fingerprint()),
            "{:?}",
            again.verification
        );
        let said = pgp::describe(&again);
        assert!(
            said.contains("decrypted") && said.contains("good signature by bea@example.test"),
            "{said}"
        );
    }

    #[test]
    fn a_message_encrypted_to_someone_else_says_it_cannot_be_read_here() {
        let (store, _dir, secrets, _) = with_key();
        let (bea, cara) = (
            someone_elses(BEA, 15),
            someone_elses("cara@example.test", 16),
        );
        let message = arrive(&store, from_bea(&bea, &cara.public(), "lion"));
        let opened = pgp::open_message(&store, &secrets, &message, &pgp::no_passphrase, now())
            .unwrap()
            .unwrap();
        assert!(matches!(&opened.encryption, Encryption::CannotDecrypt { to } if !to.is_empty()));
        assert!(opened.shown.is_none());
        assert!(pgp::describe(&opened).contains("keys you do not hold"));
    }

    #[test]
    fn show_says_a_message_is_signed_and_by_whom() {
        let (store, _dir, secrets, key) = with_key();
        let signer = Unlocking {
            key: mail_runtime::pgp::secret_key(&secrets, ACCOUNT, key.fingerprint).unwrap(),
            passphrase: String::new(),
        };
        let raw = "From: me@example.test\r\nTo: bea@example.test\r\nSubject: note\r\n\
                   Message-ID: <note@example.test>\r\nContent-Type: text/plain\r\n\r\nsigned note\r\n";
        let sealed = openpgp::seal(
            raw.as_bytes(),
            &openpgp::Sealing {
                mode: OpenPgp::Sign,
                signer: Some(&signer),
                recipients: &[],
                gossip: &[],
                now: now(),
            },
            &mut rand::rngs::StdRng::seed_from_u64(17),
        )
        .unwrap();
        let message = arrive(&store, sealed);
        let shown = cli::run_with_clients(
            &store,
            &cli::Command::Show {
                thread: message.thread,
            },
            now(),
            &mail_runtime::OAuthRegistry::default(),
        )
        .unwrap();
        assert!(
            shown.contains("good signature by me@example.test"),
            "{shown}"
        );
        assert!(shown.contains("signed note"), "{shown}");
        assert!(
            !shown.contains("BEGIN PGP SIGNATURE"),
            "the signature part is not printed: {shown}"
        );
    }
}

mod parsing {
    use super::*;

    fn args(s: &str) -> Vec<String> {
        s.split(' ').map(str::to_owned).collect()
    }

    #[test]
    fn the_pgp_commands_parse() {
        let fp: Fingerprint = "0123456789ABCDEF0123456789ABCDEF01234567".parse().unwrap();
        const_cases(&[
            ("pgp keys", pgp::PgpCommand::Keys),
            (
                "pgp generate me@example.test",
                pgp::PgpCommand::Generate {
                    address: ME.to_owned(),
                },
            ),
            (
                "pgp import keys.asc",
                pgp::PgpCommand::Import {
                    path: "keys.asc".into(),
                },
            ),
            (
                "pgp export me@example.test",
                pgp::PgpCommand::Export {
                    named: ME.to_owned(),
                    secret: pgp::Secret::Public,
                },
            ),
            (
                "pgp export me@example.test --secret",
                pgp::PgpCommand::Export {
                    named: ME.to_owned(),
                    secret: pgp::Secret::Included,
                },
            ),
            (
                "pgp delete me@example.test --with-secret",
                pgp::PgpCommand::Delete {
                    named: ME.to_owned(),
                    with_secret: pgp::WithSecret::Confirmed,
                },
            ),
            (
                "pgp lookup bea@example.test",
                pgp::PgpCommand::Lookup {
                    address: BEA.to_owned(),
                },
            ),
            (
                "pgp verify 0123456789ABCDEF0123456789ABCDEF01234567",
                pgp::PgpCommand::Verify { fingerprint: fp },
            ),
        ]);
        for bad in [
            "pgp",
            "pgp frobnicate",
            "pgp export",
            "pgp export x --public",
            "pgp verify zz",
        ] {
            assert!(cli::parse(&args(bad)).is_err(), "{bad}");
        }
    }

    fn const_cases(cases: &[(&str, pgp::PgpCommand)]) {
        for (line, expected) in cases {
            assert_eq!(
                cli::parse(&args(line)).unwrap(),
                cli::Command::Pgp(expected.clone()),
                "{line}"
            );
        }
    }

    #[test]
    fn compose_takes_sign_and_encrypt() {
        const CASES: &[(&str, OpenPgp)] = &[
            ("", OpenPgp::None),
            (" --sign", OpenPgp::Sign),
            (" --encrypt", OpenPgp::Encrypt),
            (" --encrypt --sign", OpenPgp::SignAndEncrypt),
        ];
        for (rest, expected) in CASES {
            match cli::parse(&args(&format!("compose --to bea@example.test{rest}"))).unwrap() {
                cli::Command::Compose { openpgp, .. } => assert_eq!(openpgp, *expected, "{rest}"),
                other => panic!("{other:?}"),
            }
        }
    }
}
