//! S/MIME from the user's side (`plan.md` 10.16): identities imported and forgotten, drafts sent
//! signed and encrypted, protected mail opened when it is read — against a real store, with the
//! keyring a [`MapSecrets`] so the user's own is never touched, and every certificate and key
//! made by the throwaway authority in `mail-mime/tests/smime_support`.

#[path = "../../mail-mime/tests/smime_support/mod.rs"]
mod smime_support;

use mail_app::pgp::WithSecret;
use mail_app::{cli, compose, smime};
use mail_domain::*;
use mail_mime::smime::{self as cms_smime, Cert, Sealing};
use mail_runtime::{Arrival, MapSecrets, Secrets};
use mail_store::{SqliteStore, Store};
use smime_support::*;

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a2"));
const IDENTITY: IdentityId =
    IdentityId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000b2"));
const ME: &str = "me@example.test";
const BEA: &str = "bea@example.test";
const PASSWORD: &str = "p12 password";

/// The count of key changes is one for the whole process, and tests here assert how it moves, so
/// they run one at a time: each holds this for its length.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|held| held.into_inner())
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

fn seeded() -> (SqliteStore, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::in_memory(dir.path()).unwrap();
    seed(&store);
    (store, dir)
}

fn me() -> cms_smime::Identity {
    identity(pki(), &Person::new("Me", &[ME], 20, 2001))
}

fn bea() -> cms_smime::Identity {
    identity(pki(), &Person::new("Bea", &[BEA], 21, 2002))
}

/// The identity `seed` makes, as the composer holds it.
fn my_identity() -> Identity {
    Identity {
        id: IDENTITY,
        account: ACCOUNT,
        from: Address {
            name: Some("Me".to_owned()),
            email: ME.to_owned(),
        },
        reply_to: None,
        signature: None,
        default: IsDefault::Default,
    }
}

fn password() -> Option<String> {
    Some(PASSWORD.to_owned())
}

/// The user's identity imported from a PKCS#12 file, and the test root trusted, as a user of
/// this authority would have it.
fn with_identity() -> (SqliteStore, tempfile::TempDir, MapSecrets) {
    let (store, dir) = seeded();
    let secrets = MapSecrets::default();
    let file = cms_smime::write_pkcs12(&me(), PASSWORD, &mut rng(1)).unwrap();
    smime::certs::import(&store, &secrets, &file, &password, now()).unwrap();
    trust_root(&store, &secrets);
    (store, dir, secrets)
}

fn trust_root(store: &SqliteStore, secrets: &MapSecrets) {
    smime::certs::import(
        store,
        secrets,
        pki().root.cert.pem().as_bytes(),
        &|| None,
        now(),
    )
    .unwrap();
    smime::certs::trust(store, pki().root.cert.fingerprint(), KeyTrust::Verified).unwrap();
}

/// Bea's certificate, as `mailo smime import` of a certificate file keeps it.
fn import_bea(store: &SqliteStore, secrets: &MapSecrets) {
    smime::certs::import(store, secrets, bea().cert.pem().as_bytes(), &|| None, now()).unwrap();
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

fn draft(store: &SqliteStore, mode: Smime, recipients: &[&str], bcc: &[&str]) -> Draft {
    let mut draft = compose::draft_new(
        store,
        ACCOUNT,
        &to(recipients),
        "Secret plans",
        "meet at the usual place",
        now(),
    )
    .unwrap();
    draft.smime = mode;
    draft.bcc = to(bcc);
    compose::save(store, &draft).unwrap();
    draft
}

fn submissions(store: &SqliteStore) -> Vec<mail_store::OutboxEntry> {
    store
        .outbox_due(ACCOUNT, now() + chrono::TimeDelta::try_days(365).unwrap())
        .unwrap()
}

fn frozen(store: &SqliteStore) -> Vec<u8> {
    let entries = submissions(store);
    assert_eq!(entries.len(), 1, "one submission");
    let ProtoOp::Submit { raw, .. } = &entries[0].op else {
        panic!("{:?}", entries[0].op);
    };
    store.blobs().get(&store.connection(), *raw).unwrap()
}

fn send(store: &SqliteStore, secrets: &MapSecrets, draft: DraftId) -> Result<String, String> {
    compose::send_with(store, secrets, &mail_app::pgp::no_passphrase, draft, now())
        .map_err(|e| e.to_string())
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

/// A message from Bea to the user, sealed by Bea's client.
fn from_bea(mode: Smime, to_certs: &[Cert], body: &[u8]) -> Vec<u8> {
    cms_smime::seal(
        body,
        &Sealing {
            mode,
            signer: Some(&bea()),
            recipients: to_certs,
            now: now(),
        },
        &mut rng(9),
    )
    .unwrap()
}

mod identities {
    use super::*;

    #[test]
    fn an_imported_identity_keeps_its_key_in_the_keyring_and_its_certificate_in_the_store() {
        let _serial = serial();
        let (store, _dir) = seeded();
        let secrets = MapSecrets::default();
        assert!(store.smime_certs().unwrap().is_empty());
        let file = cms_smime::write_pkcs12(&me(), PASSWORD, &mut rng(1)).unwrap();
        let imported = smime::certs::import(&store, &secrets, &file, &password, now()).unwrap();
        let mine = &imported[0].cert;
        assert_eq!(mine.fingerprint, me().cert.fingerprint());
        assert_eq!(mine.source, CertSource::Identity);
        assert_eq!(mine.secret, SecretHeld::Held);
        assert_eq!(mine.emails, vec![ME]);
        assert_eq!(
            mine.chain,
            vec![
                pki().intermediate.cert.der().to_vec(),
                pki().root.cert.der().to_vec()
            ]
        );
        assert_eq!(
            smime::own_cert(&store, ME, now())
                .unwrap()
                .map(|c| c.fingerprint),
            Some(mine.fingerprint)
        );
        let held = secrets
            .get(&SecretKey {
                account: ACCOUNT,
                purpose: SecretPurpose::Smime(mine.fingerprint),
            })
            .unwrap();
        let Credential::SmimeKey(pem) = held else {
            panic!("{held:?}");
        };
        assert_eq!(
            mail_mime::smime::PrivateKey::from_pkcs8_pem(&pem).unwrap(),
            me().key
        );
    }

    #[test]
    fn no_private_key_is_ever_written_to_the_database_file() {
        let _serial = serial();
        let dir = tempfile::tempdir().unwrap();
        let blobs = dir.path().join("blobs");
        std::fs::create_dir_all(&blobs).unwrap();
        let store = SqliteStore::open(dir.path().join("mail.db"), &blobs).unwrap();
        seed(&store);
        let secrets = MapSecrets::default();
        let file = cms_smime::write_pkcs12(&me(), PASSWORD, &mut rng(1)).unwrap();
        smime::certs::import(&store, &secrets, &file, &password, now()).unwrap();
        drop(store);

        let mut bytes = Vec::new();
        for entry in std::fs::read_dir(dir.path()).unwrap() {
            let path = entry.unwrap().path();
            if path.is_file() {
                bytes.extend(std::fs::read(&path).unwrap());
            }
        }
        let key = me().key.to_pkcs8_der().unwrap();
        let pem = me().key.to_pkcs8_pem().unwrap();
        let inside = |needle: &[u8]| bytes.windows(needle.len()).any(|w| w == needle);
        assert!(!inside(&key[key.len() - 64..]), "PKCS#8 bytes on disk");
        assert!(!inside(&pem.as_bytes()[40..100]), "PEM text on disk");
        assert!(
            inside(me().cert.der()),
            "the certificate is there, which proves the scan works"
        );
    }

    #[test]
    fn a_wrong_or_missing_password_and_a_stranger_identity_are_refused() {
        let _serial = serial();
        let (store, _dir) = seeded();
        let secrets = MapSecrets::default();
        let file = cms_smime::write_pkcs12(&me(), PASSWORD, &mut rng(1)).unwrap();
        let wrong = smime::certs::import(&store, &secrets, &file, &|| Some("no".to_owned()), now())
            .unwrap_err();
        assert!(wrong.to_string().contains("password"), "{wrong}");
        let none = smime::certs::import(&store, &secrets, &file, &|| None, now()).unwrap_err();
        assert!(matches!(none, smime::SmimeError::NoPassword), "{none}");
        let theirs = cms_smime::write_pkcs12(&bea(), PASSWORD, &mut rng(2)).unwrap();
        let refused =
            smime::certs::import(&store, &secrets, &theirs, &password, now()).unwrap_err();
        assert!(
            matches!(refused, smime::SmimeError::NotYours { .. }),
            "{refused}"
        );
        assert!(store.smime_certs().unwrap().is_empty(), "nothing kept");
    }

    #[test]
    fn deleting_an_identity_needs_its_key_confirmed_and_takes_the_key_too() {
        let _serial = serial();
        let (store, _dir, secrets) = with_identity();
        let mine = smime::certs::find(&store, ME).unwrap();
        let entry = SecretKey {
            account: ACCOUNT,
            purpose: SecretPurpose::Smime(mine.fingerprint),
        };
        let refused =
            smime::certs::delete(&store, &secrets, &mine, WithSecret::Refuse).unwrap_err();
        assert!(matches!(refused, smime::SmimeError::SecretWouldBeLost(_)));
        assert!(secrets.get(&entry).is_ok());
        smime::certs::delete(&store, &secrets, &mine, WithSecret::Confirmed).unwrap();
        assert!(
            secrets.get(&entry).is_err(),
            "the key is gone from the keyring"
        );
        assert_eq!(store.smime_cert(mine.fingerprint).unwrap(), None);
        // A correspondent's certificate, with no key, goes without confirmation.
        import_bea(&store, &secrets);
        let theirs = smime::certs::find(&store, BEA).unwrap();
        smime::certs::delete(&store, &secrets, &theirs, WithSecret::Refuse).unwrap();
    }

    #[test]
    fn export_prints_the_certificate_and_trust_moves_the_epoch() {
        let _serial = serial();
        let (store, _dir, secrets) = with_identity();
        let pem = smime::certs::export(&smime::certs::find(&store, ME).unwrap()).unwrap();
        assert_eq!(
            cms_smime::read_certs(pem.as_bytes()).unwrap(),
            vec![me().cert]
        );
        let before = smime::epoch();
        import_bea(&store, &secrets);
        let imported = smime::epoch();
        assert!(imported > before, "an import is a change");
        smime::certs::trust(&store, bea().cert.fingerprint(), KeyTrust::Verified).unwrap();
        assert!(smime::epoch() > imported, "so is trusting one");
        assert_eq!(mail_app::pgp::epoch(), smime::epoch(), "one count for both");
    }
}

mod sending {
    use super::*;

    #[test]
    fn a_signed_draft_leaves_as_multipart_signed_that_verifies() {
        let _serial = serial();
        let (store, _dir, secrets) = with_identity();
        let d = draft(&store, Smime::Sign, &[BEA], &[]);
        send(&store, &secrets, d.id).unwrap();
        let raw = frozen(&store);
        let text = String::from_utf8_lossy(&raw);
        assert!(text.contains("multipart/signed"), "{text}");
        assert!(text.contains("application/pkcs7-signature"), "{text}");
        let opened = smime::open_bytes(&store, &secrets, &raw, now())
            .unwrap()
            .unwrap();
        assert_eq!(
            opened.verification,
            SmimeVerification::Good {
                signer: me().cert.fingerprint(),
                coverage: Coverage::Whole
            }
        );
        let signer = opened.signer.unwrap();
        assert_eq!(signer.emails, vec![ME]);
    }

    #[test]
    fn an_encrypted_draft_opens_for_its_recipient_and_its_sent_copy_for_the_sender() {
        let _serial = serial();
        let (store, _dir, secrets) = with_identity();
        import_bea(&store, &secrets);
        let d = draft(&store, Smime::SignAndEncrypt, &[BEA], &[]);
        send(&store, &secrets, d.id).unwrap();
        let raw = frozen(&store);
        assert!(
            !String::from_utf8_lossy(&raw).contains("usual place"),
            "the body is not in the clear"
        );
        // Bea's client.
        let theirs = cms_smime::open(
            &raw,
            &cms_smime::Keys {
                identities: &[bea()],
                certs: &[],
                anchors: &[pki().root.cert.clone()],
                now: now(),
            },
        )
        .unwrap();
        assert_eq!(theirs.encryption, SmimeEncryption::Decrypted);
        assert!(matches!(
            theirs.verification,
            SmimeVerification::Good { .. }
        ));
        // The copy in Sent, opened here with the user's own key from the keyring.
        let mine = smime::open_bytes(&store, &secrets, &raw, now())
            .unwrap()
            .unwrap();
        assert_eq!(mine.encryption, SmimeEncryption::Decrypted);
        assert!(
            mine.shown.unwrap().text.unwrap().contains("usual place"),
            "readable"
        );
    }

    #[test]
    fn check_names_what_stands_in_the_way_and_send_queues_nothing() {
        let _serial = serial();
        type Case = (
            &'static str,
            Smime,
            &'static [&'static str],
            &'static [&'static str],
        );
        const CASES: &[Case] = &[
            (
                "certificate to encrypt to for bea@example.test",
                Smime::Encrypt,
                &[BEA],
                &[],
            ),
            (
                "cannot have Bcc",
                Smime::Encrypt,
                &[BEA],
                &["hidden@example.test"],
            ),
        ];
        let (store, _dir, secrets) = with_identity();
        let identity = my_identity();
        for (said, mode, recipients, bcc) in CASES {
            let d = draft(&store, *mode, recipients, bcc);
            let refused = smime::check(&store, &d, &identity, now()).unwrap_err();
            assert!(refused.to_string().contains(said), "{said}: {refused}");
            let sent = send(&store, &secrets, d.id).unwrap_err();
            assert!(sent.contains(said), "{said}: {sent}");
            assert!(submissions(&store).is_empty(), "{said}: nothing queued");
        }
        // Signing needs the user's own certificate.
        let (bare, _dir2) = seeded();
        let d = draft(&bare, Smime::Sign, &[BEA], &[]);
        let refused = smime::check(&bare, &d, &identity, now()).unwrap_err();
        assert!(
            matches!(refused, smime::SmimeError::NoOwnCert(_)),
            "{refused}"
        );
    }

    #[test]
    fn a_draft_asking_both_openpgp_and_smime_is_refused_at_check_and_at_send() {
        let _serial = serial();
        let (store, _dir, secrets) = with_identity();
        mail_app::pgp::keys::generate(&store, &secrets, ME, now()).unwrap();
        let mut d = draft(&store, Smime::Sign, &[BEA], &[]);
        d.openpgp = OpenPgp::Sign;
        compose::save(&store, &d).unwrap();
        let identity = my_identity();
        assert!(matches!(
            smime::check(&store, &d, &identity, now()),
            Err(smime::SmimeError::BothProtections)
        ));
        assert!(matches!(
            mail_app::pgp::check(&store, &d, &identity, now()),
            Err(mail_app::pgp::PgpError::BothProtections)
        ));
        let refused =
            compose::send_with(&store, &secrets, &mail_app::pgp::no_passphrase, d.id, now())
                .unwrap_err();
        assert!(
            matches!(
                refused,
                compose::SendError::Smime(smime::SmimeError::BothProtections)
            ),
            "{refused}"
        );
        assert!(submissions(&store).is_empty());
    }

    #[test]
    fn compose_on_the_command_line_takes_smime_and_says_what_stands_in_the_way() {
        let _serial = serial();
        const CASES: &[(&str, Smime, OpenPgp)] = &[
            ("--sign --smime", Smime::Sign, OpenPgp::None),
            ("--encrypt --smime", Smime::Encrypt, OpenPgp::None),
            (
                "--smime --sign --encrypt",
                Smime::SignAndEncrypt,
                OpenPgp::None,
            ),
            ("--sign", Smime::None, OpenPgp::Sign),
        ];
        for (flags, smime_mode, openpgp) in CASES {
            let mut args: Vec<String> = vec!["compose".into(), "--to".into(), BEA.into()];
            args.extend(flags.split(' ').map(str::to_owned));
            match cli::parse(&args).unwrap() {
                cli::Command::Compose {
                    smime, openpgp: p, ..
                } => {
                    assert_eq!((smime, p), (*smime_mode, *openpgp), "{flags}");
                }
                other => panic!("{flags}: {other:?}"),
            }
        }
        let alone = cli::parse(&[
            "compose".into(),
            "--to".into(),
            BEA.into(),
            "--smime".into(),
        ])
        .unwrap_err();
        assert!(alone.contains("--smime says how"), "{alone}");

        let (store, _dir) = seeded();
        let said = compose::new_sealed_message(
            &store,
            None,
            [&to(&[BEA]), &[], &[]],
            "hi",
            "body",
            (ReceiptRequest::Unrequested, OpenPgp::None, Smime::Encrypt),
            now(),
        )
        .unwrap();
        assert!(said.contains("encrypted with S/MIME"), "{said}");
        assert!(said.contains("no current S/MIME certificate"), "{said}");
    }
}

mod reading {
    use super::*;

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

    const BODY: &[u8] = b"From: Bea <bea@example.test>\r\nTo: me@example.test\r\n\
        Subject: Plans\r\nDate: Thu, 24 Sep 2026 10:00:00 +0000\r\nMessage-ID: <b1@example.test>\r\n\
        MIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"b\"\r\n\r\n\
        --b\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nThe map is attached.\r\n\
        --b\r\nContent-Type: application/octet-stream; name=\"map.bin\"\r\n\
        Content-Disposition: attachment; filename=\"map.bin\"\r\n\
        Content-Transfer-Encoding: base64\r\n\r\nAAECAwQFBgc=\r\n--b--\r\n";

    #[test]
    fn a_signed_message_arriving_teaches_its_senders_certificate_once() {
        let _serial = serial();
        let (store, _dir, secrets) = with_identity();
        assert!(smime::cert_for(&store, BEA, now()).unwrap().is_none());
        let before = smime::epoch();
        arrive(&store, from_bea(Smime::Sign, &[], BODY));
        let learnt = smime::certs::find(&store, BEA).unwrap();
        assert_eq!(learnt.source, CertSource::Received);
        assert_eq!(learnt.fingerprint, bea().cert.fingerprint());
        assert!(smime::epoch() > before, "a new certificate is a change");
        // Seen again, the same certificate changes nothing.
        let again = smime::epoch();
        let second = String::from_utf8(BODY.to_vec())
            .unwrap()
            .replace("<b1@example.test>", "<b2@example.test>");
        arrive(&store, from_bea(Smime::Sign, &[], second.as_bytes()));
        assert_eq!(smime::epoch(), again);
        // And the user can now answer encrypted.
        let d = draft(&store, Smime::Encrypt, &[BEA], &[]);
        send(&store, &secrets, d.id).unwrap();
    }

    #[test]
    fn a_signature_from_someone_the_certificate_does_not_name_teaches_nothing() {
        let _serial = serial();
        let (store, _dir, _secrets) = with_identity();
        let forged = String::from_utf8(BODY.to_vec()).unwrap().replace(
            "From: Bea <bea@example.test>",
            "From: Eve <eve@example.test>",
        );
        arrive(&store, from_bea(Smime::Sign, &[], forged.as_bytes()));
        assert!(store.smime_certs_for(BEA).unwrap().is_empty());
        assert!(
            store
                .smime_certs_for("eve@example.test")
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn an_encrypted_message_opens_with_its_attachment_readable_and_is_never_stored_decrypted() {
        let _serial = serial();
        let (store, dir, secrets) = with_identity();
        let message = arrive(&store, from_bea(Smime::SignAndEncrypt, &[me().cert], BODY));
        let opened = smime::open_message(&store, &secrets, &message, now())
            .unwrap()
            .unwrap();
        assert_eq!(opened.encryption, SmimeEncryption::Decrypted);
        assert_eq!(
            opened.verification,
            SmimeVerification::Good {
                signer: bea().cert.fingerprint(),
                coverage: Coverage::Whole,
            }
        );
        let said = smime::describe(&opened);
        assert!(said.contains("S/MIME encrypted; decrypted"), "{said}");
        assert!(
            said.contains("good S/MIME signature by bea@example.test"),
            "{said}"
        );
        // The attachment, from the decrypted message: the stored one is ciphertext.
        let map = opened.attachment(0).unwrap();
        assert_eq!(map.name, "map.bin");
        assert_eq!(map.bytes, vec![0, 1, 2, 3, 4, 5, 6, 7]);
        assert!(opened.attachment(1).is_err());
        let saved = mail_app::attach::save_opened(&map, dir.path()).unwrap();
        assert_eq!(std::fs::read(saved).unwrap(), map.bytes);
        // Nothing decrypted reached the index: the subject, outside, is found; the body is not.
        assert_eq!(text_search(&store, "Plans"), 1);
        assert_eq!(text_search(&store, "attached"), 0);
    }

    #[test]
    fn mail_encrypted_to_someone_else_says_so_and_the_cli_shows_the_status() {
        let _serial = serial();
        let (store, _dir, secrets) = with_identity();
        let message = arrive(&store, from_bea(Smime::Encrypt, &[bea().cert], BODY));
        let opened = smime::open_message(&store, &secrets, &message, now())
            .unwrap()
            .unwrap();
        assert!(matches!(
            opened.encryption,
            SmimeEncryption::CannotDecrypt { .. }
        ));
        assert!(opened.shown.is_none());
        let command = smime::parse(&["show".into(), message.id.to_string()]).unwrap();
        let said = smime::run(&store, &secrets, &|| None, &command, now()).unwrap();
        assert!(said.contains("hold no key for"), "{said}");
    }
}

mod command_line {
    use super::*;

    #[test]
    fn the_smime_verbs_parse_as_the_usage_says() {
        let _serial = serial();
        let fp = me().cert.fingerprint();
        let cases: Vec<(Vec<&str>, smime::SmimeCommand)> = vec![
            (vec!["list"], smime::SmimeCommand::List),
            (
                vec!["import", "me.p12"],
                smime::SmimeCommand::Import {
                    path: "me.p12".into(),
                },
            ),
            (
                vec!["export", ME],
                smime::SmimeCommand::Export {
                    named: ME.to_owned(),
                },
            ),
            (
                vec!["delete", ME, "--with-secret"],
                smime::SmimeCommand::Delete {
                    named: ME.to_owned(),
                    with_secret: WithSecret::Confirmed,
                },
            ),
        ];
        for (args, expected) in cases {
            let owned: Vec<String> = args.iter().map(|a| (*a).to_owned()).collect();
            assert_eq!(smime::parse(&owned).unwrap(), expected, "{args:?}");
        }
        let trust = smime::parse(&["untrust".into(), fp.to_string()]).unwrap();
        assert_eq!(
            trust,
            smime::SmimeCommand::Trust {
                fingerprint: fp,
                trust: KeyTrust::Unverified
            }
        );
        assert!(smime::parse(&["trust".into(), "abc".into()]).is_err());
        assert!(smime::parse(&["delete".into(), ME.into(), "--force".into()]).is_err());
        assert!(matches!(
            cli::parse(&["smime".into(), "list".into()]).unwrap(),
            cli::Command::Smime(smime::SmimeCommand::List)
        ));
    }

    #[test]
    fn list_says_whose_each_certificate_is() {
        let _serial = serial();
        let (store, _dir, secrets) = with_identity();
        import_bea(&store, &secrets);
        let said = cli::run(
            &store,
            &cli::Command::Smime(smime::SmimeCommand::List),
            now(),
        )
        .unwrap();
        assert!(said.contains("your identity") && said.contains("private key in keyring"));
        assert!(said.contains(BEA) && said.contains("imported"));
        assert!(said.contains("trusted by you"), "the root: {said}");
    }
}
