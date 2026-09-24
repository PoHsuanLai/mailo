//! OpenPGP through the whole mail path: keys made and read back, messages built by
//! [`mail_mime::build`], sealed as PGP/MIME, and opened and parsed as a reader would.
//!
//! No fixture keys are checked in: each test makes its own with a seeded generator, so the keys
//! are the same on every run and nothing secret sits in the repository pretending to be test data.

use chrono::{DateTime, TimeZone, Utc};
use mail_domain::{
    AccountId, Address, Coverage, Draft, DraftId, Encryption, Identity, IdentityId, IsDefault,
    KeySource, KeyTrust, OpenPgp, PreferEncrypt, ReceiptRequest, SendState, Verification,
};
use mail_mime::openpgp::{
    self, Cert, Keys, KnownCert, Protection, ReadKey, Sealing, SecretCert, Unlocking, wkd,
};
use mail_mime::{Disclosure, MimeError, build, parse, restamp};
use rand::SeedableRng;
use rand::rngs::StdRng;

fn at() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 24, 17, 0, 0).unwrap()
}

fn rng(seed: u64) -> StdRng {
    StdRng::seed_from_u64(seed)
}

fn addr(email: &str) -> Address {
    Address {
        name: None,
        email: email.to_owned(),
    }
}

fn identity() -> Identity {
    Identity {
        id: IdentityId::generate(),
        account: AccountId::generate(),
        from: Address {
            name: Some("Me".to_owned()),
            email: "me@example.test".to_owned(),
        },
        reply_to: None,
        signature: None,
        default: IsDefault::Default,
    }
}

fn draft(subject: &str, text: &str) -> Draft {
    Draft {
        id: DraftId::generate(),
        account: AccountId::generate(),
        identity: IdentityId::generate(),
        to: vec![addr("bea@example.test")],
        cc: Vec::new(),
        bcc: Vec::new(),
        subject: subject.to_owned(),
        in_reply_to: None,
        forward_of: None,
        text: text.to_owned(),
        html: None,
        attachments: Vec::new(),
        receipt: ReceiptRequest::Unrequested,
        openpgp: OpenPgp::None,
        smime: mail_domain::Smime::None,
        state: SendState::Editing,
        updated: at(),
    }
}

fn built(subject: &str, text: &str) -> Vec<u8> {
    build(
        &draft(subject, text),
        &identity(),
        None,
        &[],
        Disclosure::HideBlind,
    )
    .unwrap()
}

fn key(name: &str, seed: u64) -> SecretCert {
    openpgp::generate(
        &format!("{name} <{name}@example.test>"),
        at(),
        &mut rng(seed),
    )
    .unwrap()
}

fn open_key(key: &SecretCert) -> Unlocking {
    Unlocking {
        key: key.clone(),
        passphrase: String::new(),
    }
}

fn known(key: &SecretCert) -> KnownCert {
    KnownCert {
        cert: key.public(),
        trust: KeyTrust::Unverified,
    }
}

fn sealing<'a>(mode: OpenPgp, signer: Option<&'a Unlocking>, to: &'a [Cert]) -> Sealing<'a> {
    Sealing {
        mode,
        signer,
        recipients: to,
        gossip: &[],
        now: at(),
    }
}

fn text_of(message: &[u8]) -> String {
    parse(message).unwrap().text.unwrap_or_default()
}

#[test]
fn a_generated_key_exports_and_imports_as_itself() {
    let mine = key("me", 1);
    assert_eq!(mine.protection(), Protection::Open);
    let public = mine.public();
    assert_eq!(public.emails(), vec!["me@example.test"]);
    assert!(
        public.can_encrypt(at()),
        "the Curve25519 subkey is for encryption"
    );
    assert_eq!(public.key_ids().len(), 2, "a primary key and one subkey");

    let armored = public.armored().unwrap();
    assert!(armored.starts_with("-----BEGIN PGP PUBLIC KEY BLOCK-----"));
    match openpgp::read_keys(armored.as_bytes()).unwrap().as_slice() {
        [ReadKey::Public(back)] => assert_eq!(back.fingerprint(), mine.fingerprint()),
        other => panic!("{other:?}"),
    }
    let secret = mine.armored().unwrap();
    assert!(secret.starts_with("-----BEGIN PGP PRIVATE KEY BLOCK-----"));
    assert_eq!(SecretCert::from_armored(&secret).unwrap(), mine);
    // The binary form the store keeps reads back to the same key.
    assert_eq!(Cert::from_bytes(&public.to_bytes()).unwrap(), public);

    let record = public.record(KeySource::Generated, at(), &[]);
    assert_eq!(record.fingerprint, mine.fingerprint());
    assert_eq!(record.emails, vec!["me@example.test"]);
    assert!(!record.key.is_empty());
}

#[test]
fn a_keyring_file_of_several_blocks_reads_every_key() {
    let (a, b) = (key("a", 2), key("b", 3));
    let file = format!(
        "exported by hand\n{}\n{}",
        a.public().armored().unwrap(),
        b.armored().unwrap()
    );
    let keys = openpgp::read_keys(file.as_bytes()).unwrap();
    assert_eq!(keys.len(), 2);
    assert!(matches!(&keys[0], ReadKey::Public(k) if k.fingerprint() == a.fingerprint()));
    assert!(matches!(&keys[1], ReadKey::Secret(k) if k.fingerprint() == b.fingerprint()));
    // The binary keyring GnuPG writes is the same keys without the armor.
    let binary = [a.public().to_bytes(), b.public().to_bytes()].concat();
    assert_eq!(openpgp::read_keys(&binary).unwrap().len(), 2);
    assert!(openpgp::read_keys(b"not a key").is_err());
}

#[test]
fn a_passphrase_locks_a_key_and_only_the_right_one_opens_it() {
    let locked = key("me", 4)
        .with_passphrase("correct horse", &mut rng(5))
        .unwrap();
    assert_eq!(locked.protection(), Protection::Passphrase);
    assert!(locked.unlocks_with("correct horse"));
    assert!(!locked.unlocks_with("wrong"));
    assert!(!locked.unlocks_with(""));
}

#[test]
fn a_signed_message_is_pgp_mime_with_the_content_as_its_first_part() {
    let me = key("me", 6);
    let signer = open_key(&me);
    let sealed = openpgp::seal(
        &built("Lunch", "see you at noon"),
        &sealing(OpenPgp::Sign, Some(&signer), &[]),
        &mut rng(7),
    )
    .unwrap();
    let text = String::from_utf8(sealed.clone()).unwrap();
    let (head, body) = text.split_once("\r\n\r\n").unwrap();
    assert!(
        head.contains("Subject: Lunch"),
        "the subject stays readable: {head}"
    );
    assert!(head.contains(
        "Content-Type: multipart/signed; micalg=pgp-sha256;\r\n protocol=\"application/pgp-signature\"; boundary=\""
    ));
    assert_eq!(
        head.matches("Content-Type:").count(),
        1,
        "one type, the multipart's"
    );
    assert_eq!(head.matches("MIME-Version:").count(), 1);
    // RFC 3156 §5: the first part is the content with its own type, the second the signature.
    let parts: Vec<&str> = body.split("\r\n--mailo-pgp-").skip(1).collect();
    assert_eq!(parts.len(), 3, "two parts and the closing boundary: {body}");
    assert!(
        parts[0].contains("Content-Type: text/plain"),
        "{}",
        parts[0]
    );
    assert!(parts[0].contains("see you at noon"));
    assert!(parts[1].contains("Content-Type: application/pgp-signature"));
    assert!(parts[1].contains("-----BEGIN PGP SIGNATURE-----"));

    let opened = openpgp::open(
        &sealed,
        &Keys {
            certs: vec![known(&me)],
            secrets: Vec::new(),
        },
    )
    .unwrap();
    assert_eq!(opened.encryption, Encryption::NotEncrypted);
    assert_eq!(
        opened.verification,
        Verification::Good {
            signer: me.fingerprint(),
            trust: KeyTrust::Unverified,
            coverage: Coverage::Whole,
        }
    );
    assert_eq!(text_of(&opened.message).trim_end(), "see you at noon");
    // The signature part is not shown as an attachment of the opened message.
    assert!(parse(&opened.message).unwrap().attachments.is_empty());
}

#[test]
fn restamping_the_date_after_signing_leaves_the_signature_good() {
    // The order the outbox runs them in: signed when Send is pressed, `Date` replaced as the
    // message leaves (`plan.md` 10.8). The signature covers the content entity only, so the
    // header may change under it.
    let me = key("me", 8);
    let signer = open_key(&me);
    let sealed = openpgp::seal(
        &built("Later", "scheduled"),
        &sealing(OpenPgp::Sign, Some(&signer), &[]),
        &mut rng(9),
    )
    .unwrap();
    let leaving = Utc.with_ymd_and_hms(2026, 9, 25, 9, 0, 0).unwrap();
    let stamped = restamp(&sealed, leaving);
    assert_ne!(stamped, sealed, "the restamp changed the bytes");
    assert_eq!(parse(&stamped).unwrap().date, Some(leaving));
    let opened = openpgp::open(
        &stamped,
        &Keys {
            certs: vec![known(&me)],
            secrets: Vec::new(),
        },
    )
    .unwrap();
    assert!(
        matches!(opened.verification, Verification::Good { signer, .. } if signer == me.fingerprint()),
        "{:?}",
        opened.verification
    );
}

#[test]
fn a_changed_signed_body_is_a_bad_signature() {
    let me = key("me", 10);
    let signer = open_key(&me);
    let sealed = openpgp::seal(
        &built("Pay", "send 10 to account 1"),
        &sealing(OpenPgp::Sign, Some(&signer), &[]),
        &mut rng(11),
    )
    .unwrap();
    let tampered = String::from_utf8(sealed)
        .unwrap()
        .replace("send 10 to account 1", "send 99 to account 7")
        .into_bytes();
    let opened = openpgp::open(
        &tampered,
        &Keys {
            certs: vec![known(&me)],
            secrets: Vec::new(),
        },
    )
    .unwrap();
    assert_eq!(
        opened.verification,
        Verification::Bad {
            coverage: Coverage::Whole
        }
    );
}

#[test]
fn a_signature_by_a_key_not_held_names_the_key() {
    let stranger = key("stranger", 12);
    let signer = open_key(&stranger);
    let sealed = openpgp::seal(
        &built("Hello", "who am I"),
        &sealing(OpenPgp::Sign, Some(&signer), &[]),
        &mut rng(13),
    )
    .unwrap();
    let opened = openpgp::open(
        &sealed,
        &Keys {
            certs: vec![known(&key("someone-else", 14))],
            secrets: Vec::new(),
        },
    )
    .unwrap();
    assert_eq!(
        opened.verification,
        Verification::UnknownKey {
            issuer: stranger.fingerprint().key_id(),
            coverage: Coverage::Whole,
        }
    );
}

#[test]
fn an_encrypted_message_round_trips_through_build_and_parse_with_its_subject_protected() {
    let (me, bea) = (key("me", 15), key("bea", 16));
    let signer = open_key(&me);
    let to = [bea.public(), me.public()];
    let sealed = openpgp::seal(
        &built("Secret plans", "meet at the usual place"),
        &sealing(OpenPgp::SignAndEncrypt, Some(&signer), &to),
        &mut rng(17),
    )
    .unwrap();

    // What a sync would store and index: nothing of the content, and not the subject.
    let outside = parse(&sealed).unwrap();
    assert_eq!(outside.subject, "...");
    let wire = String::from_utf8_lossy(&sealed);
    assert!(!wire.contains("meet at the usual place"));
    assert!(!wire.contains("Secret plans"));
    assert!(wire.contains("protocol=\"application/pgp-encrypted\""));
    assert!(wire.contains("Version: 1"));

    // Both the recipient and the sender (the Sent copy) can read it.
    for reader in [&bea, &me] {
        let keys = Keys {
            certs: vec![known(&me)],
            secrets: vec![open_key(reader)],
        };
        let recipients = openpgp::encrypted_to(&sealed).unwrap();
        assert!(
            reader
                .public()
                .key_ids()
                .iter()
                .any(|id| recipients.contains(id)),
            "encrypted to each key's subkey"
        );
        let opened = openpgp::open(&sealed, &keys).unwrap();
        assert_eq!(opened.encryption, Encryption::Decrypted);
        assert!(
            matches!(opened.verification, Verification::Good { signer, coverage: Coverage::Whole, .. } if signer == me.fingerprint()),
            "{:?}",
            opened.verification
        );
        let inside = parse(&opened.message).unwrap();
        assert_eq!(
            inside.subject, "Secret plans",
            "the protected subject replaces `...`"
        );
        assert_eq!(inside.text.unwrap().trim_end(), "meet at the usual place");
        assert_eq!(inside.to, vec![addr("bea@example.test")]);
    }
}

#[test]
fn encrypted_only_to_someone_else_says_so_and_names_their_keys() {
    let (me, other) = (key("me", 18), key("other", 19));
    let to = [other.public()];
    let sealed = openpgp::seal(
        &built("Not yours", "private"),
        &sealing(OpenPgp::Encrypt, None, &to),
        &mut rng(20),
    )
    .unwrap();
    let opened = openpgp::open(
        &sealed,
        &Keys {
            certs: Vec::new(),
            secrets: vec![open_key(&me)],
        },
    )
    .unwrap();
    let Encryption::CannotDecrypt { to } = &opened.encryption else {
        panic!("{:?}", opened.encryption);
    };
    assert!(to.iter().all(|id| other.public().key_ids().contains(id)));
    assert!(!to.is_empty());
    assert_eq!(opened.message, sealed, "nothing is shown but what arrived");
}

#[test]
fn a_locked_key_needs_its_passphrase_to_decrypt() {
    let mine = key("me", 21).with_passphrase("pw", &mut rng(22)).unwrap();
    let to = [mine.public()];
    let sealed = openpgp::seal(
        &built("Locked", "behind a passphrase"),
        &sealing(OpenPgp::Encrypt, None, &to),
        &mut rng(23),
    )
    .unwrap();
    let with = |passphrase: &str| {
        openpgp::open(
            &sealed,
            &Keys {
                certs: Vec::new(),
                secrets: vec![Unlocking {
                    key: mine.clone(),
                    passphrase: passphrase.to_owned(),
                }],
            },
        )
        .unwrap()
    };
    assert_eq!(
        with("").encryption,
        Encryption::Locked {
            key: mine.fingerprint()
        }
    );
    assert_eq!(
        with("nope").encryption,
        Encryption::Locked {
            key: mine.fingerprint()
        }
    );
    let opened = with("pw");
    assert_eq!(opened.encryption, Encryption::Decrypted);
    assert_eq!(text_of(&opened.message).trim_end(), "behind a passphrase");

    // Signing with it is refused up front without the passphrase, naming the key.
    let signer = Unlocking {
        key: mine.clone(),
        passphrase: String::new(),
    };
    let refused = openpgp::seal(
        &built("x", "y"),
        &sealing(OpenPgp::Sign, Some(&signer), &[]),
        &mut rng(24),
    );
    assert_eq!(refused, Err(MimeError::KeyLocked(mine.fingerprint())));
}

#[test]
fn a_signed_part_beside_an_unsigned_footer_covers_only_part() {
    // A mailing list wraps the signed message and appends its footer as a second part.
    let me = key("me", 25);
    let signer = open_key(&me);
    let sealed = openpgp::seal(
        &built("List post", "signed words"),
        &sealing(OpenPgp::Sign, Some(&signer), &[]),
        &mut rng(26),
    )
    .unwrap();
    let text = String::from_utf8(sealed).unwrap();
    let (head, body) = text.split_once("\r\n\r\n").unwrap();
    let signed_type: String = head
        .split("\r\n")
        .skip_while(|line| !line.starts_with("Content-Type:"))
        .take_while(|line| line.starts_with("Content-Type:") || line.starts_with(' '))
        .collect::<Vec<_>>()
        .join("\r\n");
    let outer: String = head
        .split("\r\n")
        .filter(|line| {
            !line.starts_with("Content-Type:")
                && !line.starts_with(' ')
                && !line.starts_with("MIME")
        })
        .collect::<Vec<_>>()
        .join("\r\n");
    let wrapped = format!(
        "{outer}\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"list\"\r\n\r\n\
         --list\r\n{signed_type}\r\n\r\n{body}\r\n--list\r\nContent-Type: text/plain\r\n\r\n\
         To unsubscribe, write to the list.\r\n--list--\r\n"
    );
    let opened = openpgp::open(
        wrapped.as_bytes(),
        &Keys {
            certs: vec![known(&me)],
            secrets: Vec::new(),
        },
    )
    .unwrap();
    assert!(
        matches!(
            opened.verification,
            Verification::Good {
                coverage: Coverage::Part,
                ..
            }
        ),
        "{:?}",
        opened.verification
    );
}

#[test]
fn a_plain_message_carries_no_openpgp() {
    assert_eq!(openpgp::open(&built("Hi", "plain"), &Keys::default()), None);
    assert_eq!(openpgp::encrypted_to(&built("Hi", "plain")), None);
}

#[test]
fn inline_pgp_in_a_plain_text_body_is_read() {
    let me = key("me", 27);
    // An armored OpenPGP message in a text/plain body: take the armor our own PGP/MIME builder
    // made and put it inline, as older clients send it.
    let to = [me.public()];
    let sealed = openpgp::seal(
        &built("Inline", "hidden words"),
        &sealing(OpenPgp::Encrypt, None, &to),
        &mut rng(28),
    )
    .unwrap();
    let text = String::from_utf8(sealed).unwrap();
    let start = text.find("-----BEGIN PGP MESSAGE-----").unwrap();
    let end = text.find("-----END PGP MESSAGE-----").unwrap() + "-----END PGP MESSAGE-----".len();
    let armor = &text[start..end];
    let inline = format!(
        "From: me@example.test\r\nTo: me@example.test\r\nSubject: inline\r\n\
         Content-Type: text/plain; charset=us-ascii\r\n\r\n{armor}\r\n"
    );
    let opened = openpgp::open(
        inline.as_bytes(),
        &Keys {
            certs: Vec::new(),
            secrets: vec![open_key(&me)],
        },
    )
    .unwrap();
    assert_eq!(opened.encryption, Encryption::Decrypted);
    assert!(text_of(&opened.message).contains("hidden words"));
    assert_eq!(
        openpgp::encrypted_to(inline.as_bytes()).map(|ids| ids.is_empty()),
        Some(false)
    );
}

#[test]
fn an_autocrypt_header_is_written_and_read_back_under_the_specs_rules() {
    let me = key("me", 29).public();
    let field =
        openpgp::autocrypt_field("Autocrypt", "me@example.test", PreferEncrypt::Mutual, &me);
    let text = String::from_utf8(field.clone()).unwrap();
    assert!(
        text.starts_with("Autocrypt: addr=me@example.test; prefer-encrypt=mutual; keydata=\r\n ")
    );
    assert!(text.lines().all(|line| line.len() <= 78), "folded: {text}");

    let with = openpgp::with_field(&built("Hi", "hello"), &field);
    let found = openpgp::autocrypt_of(&with, "Me@Example.TEST").unwrap();
    assert_eq!(found.key.fingerprint(), me.fingerprint());
    assert_eq!(found.prefer_encrypt, PreferEncrypt::Mutual);
    assert_eq!(found.addr, "me@example.test");
    assert_eq!(
        parse(&with).unwrap().text.unwrap().trim_end(),
        "hello",
        "the body is untouched"
    );

    // From someone else: a header for another address speaks for nobody.
    assert_eq!(openpgp::autocrypt_of(&with, "mallory@example.test"), None);
    // Two headers: ambiguous, so neither.
    let twice = openpgp::with_field(&with, &field);
    assert_eq!(openpgp::autocrypt_of(&twice, "me@example.test"), None);
    // An unknown critical attribute makes it unusable; an underscore one is ignored.
    let critical = String::from_utf8(field.clone())
        .unwrap()
        .replace("prefer-encrypt=mutual", "future=1");
    assert_eq!(
        openpgp::autocrypt_of(
            &openpgp::with_field(&built("Hi", "x"), critical.as_bytes()),
            "me@example.test"
        ),
        None
    );
    let optional = String::from_utf8(field)
        .unwrap()
        .replace("prefer-encrypt=mutual", "_future=1");
    let read = openpgp::autocrypt_of(
        &openpgp::with_field(&built("Hi", "x"), optional.as_bytes()),
        "me@example.test",
    )
    .unwrap();
    assert_eq!(read.prefer_encrypt, PreferEncrypt::NoPreference);
}

#[test]
fn gossip_travels_inside_the_encryption_and_is_taken_off_what_is_shown() {
    let (me, bea, cara) = (key("me", 30), key("bea", 31), key("cara", 32));
    let signer = open_key(&me);
    let to = [bea.public(), cara.public(), me.public()];
    let gossip = [
        ("bea@example.test".to_owned(), bea.public()),
        ("cara@example.test".to_owned(), cara.public()),
    ];
    let sealed = openpgp::seal(
        &built("Group", "to both of you"),
        &Sealing {
            mode: OpenPgp::SignAndEncrypt,
            signer: Some(&signer),
            recipients: &to,
            gossip: &gossip,
            now: at(),
        },
        &mut rng(33),
    )
    .unwrap();
    assert!(
        !String::from_utf8_lossy(&sealed).contains("Autocrypt-Gossip"),
        "never outside"
    );
    let opened = openpgp::open(
        &sealed,
        &Keys {
            certs: Vec::new(),
            secrets: vec![open_key(&bea)],
        },
    )
    .unwrap();
    let gossiped: Vec<(&str, _)> = opened
        .gossip
        .iter()
        .map(|g| (g.addr.as_str(), g.key.fingerprint()))
        .collect();
    assert_eq!(
        gossiped,
        vec![
            ("bea@example.test", bea.fingerprint()),
            ("cara@example.test", cara.fingerprint())
        ]
    );
    assert!(!String::from_utf8_lossy(&opened.message).contains("Autocrypt-Gossip"));
}

#[test]
fn a_directory_answer_yields_only_the_key_for_the_address_asked() {
    let (joe, other) = (key("joe", 34), key("other", 35));
    let answer = [other.public().to_bytes(), joe.public().to_bytes()].concat();
    let found = wkd::key_from_answer(&answer, "Joe@Example.TEST").unwrap();
    assert_eq!(found.fingerprint(), joe.fingerprint());
    assert_eq!(
        wkd::key_from_answer(&other.public().to_bytes(), "joe@example.test"),
        None
    );
    assert_eq!(
        wkd::key_from_answer(b"<html>not found</html>", "joe@example.test"),
        None
    );
}

#[test]
fn encrypting_needs_a_key_to_encrypt_to_and_signing_a_key_to_sign_with() {
    assert_eq!(
        openpgp::seal(
            &built("x", "y"),
            &sealing(OpenPgp::Encrypt, None, &[]),
            &mut rng(36)
        ),
        Err(MimeError::NoRecipientKeys)
    );
    assert_eq!(
        openpgp::seal(
            &built("x", "y"),
            &sealing(OpenPgp::Sign, None, &[]),
            &mut rng(37)
        ),
        Err(MimeError::NoSigningKey)
    );
    let plain = built("x", "y");
    assert_eq!(
        openpgp::seal(&plain, &sealing(OpenPgp::None, None, &[]), &mut rng(38)).unwrap(),
        plain
    );
}
