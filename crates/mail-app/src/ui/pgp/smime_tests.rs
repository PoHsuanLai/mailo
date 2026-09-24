//! S/MIME in the reader: every verdict said in words, the certificate problems each named in
//! amber, a changed or forged signature on the danger ground, and an encrypted message shown
//! decrypted with what it has attached.
//!
//! Against a real store, the keyring a [`MapSecrets`], and every certificate made by the
//! throwaway authority the data side's tests use.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use mail_domain::*;
use mail_mime::smime::{Cert, Identity, Sealing};
use mail_runtime::MapSecrets;
use mail_store::SqliteStore;

use super::tests::{ME, arrive, lines, reader, seals, until};
use super::{Said, Tone, doubt, said_smime};
use crate::smime::Protected;
use crate::ui::fixtures::seeded;
use crate::ui::fixtures::smime_support::{Person, identity, pki, rng, stranger_pki};

const BEA: &str = "bea@example.test";
pub(super) const PASSWORD: &str = "p12 password";

pub(super) fn me() -> Identity {
    identity(pki(), &Person::new("Me", &[ME], 20, 2001))
}

fn bea() -> Identity {
    identity(pki(), &Person::new("Bea", &[BEA], 21, 2002))
}

/// The user's identity as a PKCS#12 file, locked with [`PASSWORD`].
pub(super) fn identity_file() -> Vec<u8> {
    mail_mime::smime::write_pkcs12(&me(), PASSWORD, &mut rng(1)).unwrap()
}

/// The user's identity imported, its private key in `secrets`.
pub(super) fn with_identity(store: &SqliteStore, secrets: &MapSecrets) -> SmimeCert {
    let given = || Some(PASSWORD.to_owned());
    crate::smime::certs::import(store, secrets, &identity_file(), &given, Utc::now())
        .unwrap()
        .remove(0)
        .cert
}

/// The test authority's root, imported and trusted, as a user of it would have it.
pub(super) fn trust_root(store: &SqliteStore, secrets: &MapSecrets) {
    crate::smime::certs::import(
        store,
        secrets,
        pki().root.cert.pem().as_bytes(),
        &|| None,
        Utc::now(),
    )
    .unwrap();
    crate::smime::certs::trust(store, pki().root.cert.fingerprint(), KeyTrust::Verified).unwrap();
}

/// A message from bea saying `text`, with `map.bin` attached, unique by `word`.
fn letter(word: &str, text: &str) -> String {
    format!(
        "From: Bea <bea@example.test>\r\nTo: me@example.test\r\nSubject: {word} plans\r\n\
         Date: Thu, 24 Sep 2026 10:00:00 +0000\r\nMessage-ID: <{word}-{}@example.test>\r\n\
         MIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"b\"\r\n\r\n\
         --b\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{text}\r\n\
         --b\r\nContent-Type: application/octet-stream; name=\"map.bin\"\r\n\
         Content-Disposition: attachment; filename=\"map.bin\"\r\n\
         Content-Transfer-Encoding: base64\r\n\r\nAAECAwQFBgc=\r\n--b--\r\n",
        uuid::Uuid::new_v4()
    )
}

/// `raw` sealed by `signer`'s client as `mode`, to `to`.
fn sealed(raw: &str, mode: Smime, signer: &Identity, to: &[Cert]) -> Vec<u8> {
    mail_mime::smime::seal(
        raw.as_bytes(),
        &Sealing {
            mode,
            signer: Some(signer),
            recipients: to,
            now: Utc::now(),
        },
        &mut rng(9),
    )
    .unwrap()
}

fn at(y: i32, m: u32, d: u32) -> DateTime<Utc> {
    crate::ui::fixtures::smime_support::at(y, m, d)
}

fn signer() -> SmimeCert {
    SmimeCert {
        fingerprint: CertFingerprint([0xAB; 32]),
        subject: "CN=Bea".to_owned(),
        issuer: "CN=Example Test Mail CA,O=Example".to_owned(),
        serial: "15".to_owned(),
        emails: vec![BEA.to_owned()],
        not_before: at(2026, 1, 1),
        not_after: at(2028, 1, 1),
        der: Vec::new(),
        chain: Vec::new(),
        source: CertSource::Received,
        first_seen: at(2026, 9, 1),
        last_seen: at(2026, 9, 1),
        trust: KeyTrust::Unverified,
        secret: SecretHeld::Absent,
    }
}

fn protected(encryption: SmimeEncryption, verification: SmimeVerification) -> Protected {
    Protected {
        encryption,
        verification,
        signer: Some(signer()),
        shown: None,
        raw: None,
    }
}

fn line(tone: Tone, text: &str) -> Said {
    Said {
        tone,
        text: text.to_owned(),
    }
}

const DETAIL: &str = "Certificate of CN=Bea · bea@example.test · issued by CN=Example Test Mail \
                      CA,O=Example · valid 2026-01-01 to 2028-01-01";

#[test]
fn every_smime_verdict_is_said_in_words_and_a_changed_or_forged_one_is_unmissable() {
    let whole = Coverage::Whole;
    let bad = |why| SmimeVerification::Bad {
        why,
        coverage: whole,
    };
    let cases = [
        (
            "good",
            protected(
                SmimeEncryption::NotEncrypted,
                SmimeVerification::Good {
                    signer: signer().fingerprint,
                    coverage: whole,
                },
            ),
            vec![
                line(Tone::Plain, "Not encrypted"),
                line(
                    Tone::Good,
                    "Good S/MIME signature by bea@example.test, from a certificate an \
                     authority you trust issued",
                ),
                line(Tone::Detail, DETAIL),
            ],
        ),
        (
            "altered",
            protected(SmimeEncryption::Decrypted, bad(BadSignature::Altered)),
            vec![
                line(
                    Tone::Good,
                    "Encrypted with S/MIME, and decrypted for reading",
                ),
                line(
                    Tone::Bad,
                    "Bad S/MIME signature: the message was changed after it was signed",
                ),
                line(Tone::Detail, DETAIL),
            ],
        ),
        (
            "forged",
            protected(SmimeEncryption::NotEncrypted, bad(BadSignature::Forged)),
            vec![
                line(Tone::Plain, "Not encrypted"),
                line(
                    Tone::Bad,
                    "Bad S/MIME signature: it does not match its certificate, so it is forged \
                     or damaged",
                ),
                line(Tone::Detail, DETAIL),
            ],
        ),
        (
            "weak",
            protected(
                SmimeEncryption::NotEncrypted,
                bad(BadSignature::Weak {
                    algorithm: "SHA-1".to_owned(),
                }),
            ),
            vec![
                line(Tone::Plain, "Not encrypted"),
                line(
                    Tone::Warn,
                    "S/MIME signature made with SHA-1, which is too weak to mean anything now",
                ),
                line(Tone::Detail, DETAIL),
            ],
        ),
        (
            "unsupported",
            protected(
                SmimeEncryption::NotEncrypted,
                bad(BadSignature::Unsupported {
                    algorithm: "Ed448".to_owned(),
                }),
            ),
            vec![
                line(Tone::Plain, "Not encrypted"),
                line(
                    Tone::Unknown,
                    "S/MIME signature made with Ed448, which this client does not check",
                ),
                line(Tone::Detail, DETAIL),
            ],
        ),
        (
            "malformed",
            protected(
                SmimeEncryption::NotEncrypted,
                bad(BadSignature::Malformed {
                    why: "no digest".to_owned(),
                }),
            ),
            vec![
                line(Tone::Plain, "Not encrypted"),
                line(
                    Tone::Warn,
                    "S/MIME signature that cannot be read: no digest",
                ),
                line(Tone::Detail, DETAIL),
            ],
        ),
        (
            "unknown signer, part signed",
            Protected {
                signer: None,
                ..protected(
                    SmimeEncryption::NotEncrypted,
                    SmimeVerification::UnknownSigner {
                        coverage: Coverage::Part,
                    },
                )
            },
            vec![
                line(Tone::Plain, "Not encrypted"),
                line(
                    Tone::Unknown,
                    "Signed with S/MIME by a certificate neither the message nor you hold, so \
                     the signature cannot be checked",
                ),
                line(
                    Tone::Unknown,
                    "Only part of this message is signed; the rest could say anything",
                ),
            ],
        ),
        (
            "encrypted to someone else",
            Protected {
                signer: None,
                ..protected(
                    SmimeEncryption::CannotDecrypt {
                        to: vec!["CN=Cara, serial 07".to_owned()],
                    },
                    SmimeVerification::NoSignature,
                )
            },
            vec![
                line(
                    Tone::Unknown,
                    "Encrypted with S/MIME to certificates you hold no key for (CN=Cara, serial \
                     07), so it cannot be read here",
                ),
                line(Tone::Plain, "Not signed"),
            ],
        ),
        (
            "unreadable",
            protected(
                SmimeEncryption::Unreadable {
                    why: "tampered".to_owned(),
                },
                SmimeVerification::NoSignature,
            ),
            vec![
                line(
                    Tone::Bad,
                    "Encrypted with S/MIME, and could not be read: tampered",
                ),
                line(Tone::Plain, "Not signed"),
            ],
        ),
    ];
    for (case, given, want) in cases {
        assert_eq!(said_smime(&given), want, "{case}");
    }
}

#[test]
fn each_certificate_problem_is_named_in_amber_and_says_what_it_means() {
    let cases = [
        (
            CertProblem::Untrusted,
            "Untrusted: no authority you trust issued its certificate, so anyone could have \
             made it. Trust it in Keys and certificates only after checking it with its owner",
        ),
        (
            CertProblem::Expired {
                not_after: at(2025, 1, 1),
            },
            "Expired: a certificate in its chain stopped being valid on 2025-01-01, before the \
             message says it was signed",
        ),
        (
            CertProblem::NotYetValid {
                not_before: at(2027, 1, 1),
            },
            "Not yet valid: a certificate in its chain was not valid until 2027-01-01, after \
             the message says it was signed",
        ),
        (
            CertProblem::NotForEmail,
            "Not for mail: its certificate was issued for something other than signing mail",
        ),
        (
            CertProblem::NotFrom {
                from: "mallory@example.test".to_owned(),
            },
            "Signed with a certificate for someone else's address: it is not for \
             mallory@example.test, who the message says it is from",
        ),
        (
            CertProblem::NotFrom {
                from: String::new(),
            },
            "The message names no sender to check its certificate against",
        ),
    ];
    for (problem, words) in cases {
        assert_eq!(doubt(&problem), words);
        let said = said_smime(&protected(
            SmimeEncryption::NotEncrypted,
            SmimeVerification::Doubtful {
                signer: signer().fingerprint,
                problems: vec![problem.clone()],
                coverage: Coverage::Whole,
            },
        ));
        assert_eq!(
            said,
            vec![
                line(Tone::Plain, "Not encrypted"),
                line(
                    Tone::Warn,
                    "S/MIME signature by bea@example.test: the message is as it was signed, \
                     but the certificate does not vouch for the sender",
                ),
                line(Tone::Warn, words),
                line(Tone::Detail, DETAIL),
            ],
            "{problem:?}"
        );
    }
    // Amber, and never the danger ground or the good one.
    assert_eq!(Tone::Warn.class(), "seal-line warn");
    assert_eq!(Tone::Detail.class(), "seal-line detail");
}

#[tokio::test]
async fn a_good_smime_signature_says_who_signed_and_their_certificate() {
    let (store, _dir) = seeded();
    let secrets = Arc::new(MapSecrets::default());
    trust_root(&store, &secrets);
    let raw = letter("owl", "the owl note is signed");
    let message = arrive(&store, sealed(&raw, Smime::Sign, &bea(), &[]));
    let (mut dom, mut seen) = reader(store, secrets, message.thread);
    let page = until(&mut dom, &mut seen, |page| page.contains("seal-line")).await;
    let said = lines(&page);
    assert_eq!(
        said[0],
        ("seal-line".to_owned(), "Not encrypted".to_owned())
    );
    assert_eq!(
        said[1],
        (
            "seal-line good".to_owned(),
            "Good S/MIME signature by bea@example.test, from a certificate an authority you \
             trust issued"
                .to_owned()
        )
    );
    assert_eq!(said[2].0, "seal-line detail");
    assert!(
        said[2].1.starts_with(
            "Certificate of CN=Bea · bea@example.test · issued by CN=Example Test Mail CA"
        ),
        "{said:?}"
    );
    assert!(page.contains("aria-label=\"S/MIME\""), "{page}");
    assert!(page.contains("the owl note is signed"), "{page}");
    // What was signed is listed; the signature itself is not an attachment.
    assert!(page.contains("map.bin"), "{page}");
    assert!(!page.contains("smime.p7s"), "{page}");
}

#[tokio::test]
async fn a_changed_smime_message_is_said_on_the_danger_ground() {
    let (store, _dir) = seeded();
    let secrets = Arc::new(MapSecrets::default());
    trust_root(&store, &secrets);
    let raw = letter("heron", "pay the heron invoice");
    let signed = String::from_utf8(sealed(&raw, Smime::Sign, &bea(), &[])).unwrap();
    assert!(signed.contains("pay the heron invoice"));
    let tampered = signed.replace("pay the heron invoice", "pay the forged invoice");
    let message = arrive(&store, tampered.into_bytes());
    let (mut dom, mut seen) = reader(store, secrets, message.thread);
    let page = until(&mut dom, &mut seen, |page| page.contains("seal-line bad")).await;
    let said = lines(&page);
    assert_eq!(
        said[1],
        (
            "seal-line bad".to_owned(),
            "Bad S/MIME signature: the message was changed after it was signed".to_owned()
        )
    );
    assert!(!page.contains("seal-line good"), "{page}");
}

/// `signed` with one byte of its RSA signature value changed: the content and the certificate
/// are as they were, and the signature no longer matches.
fn forged(signed: &str) -> String {
    use base64::Engine as _;
    let engine = base64::engine::general_purpose::STANDARD;
    // The last naming: the first is the outer header's `protocol` parameter.
    let part = signed.rfind("application/pkcs7-signature").unwrap();
    let start = signed[part..].find("\r\n\r\n").unwrap() + part + 4;
    let end = signed[start..].find("\r\n--").unwrap() + start;
    let joined: String = signed[start..end].split_whitespace().collect();
    let mut der = engine.decode(joined).unwrap();
    // An OCTET STRING of 256 bytes, the last in the structure: a 2048-bit RSA signature.
    let at = der
        .windows(4)
        .rposition(|w| w == [0x04, 0x82, 0x01, 0x00])
        .unwrap();
    der[at + 4 + 100] ^= 0x01;
    let encoded = engine.encode(&der);
    let lines: Vec<&str> = encoded
        .as_bytes()
        .chunks(76)
        .map(|c| std::str::from_utf8(c).unwrap())
        .collect();
    format!(
        "{}{}{}",
        &signed[..start],
        lines.join("\r\n"),
        &signed[end..]
    )
}

#[tokio::test]
async fn a_forged_smime_signature_is_said_on_the_danger_ground() {
    let (store, _dir) = seeded();
    let secrets = Arc::new(MapSecrets::default());
    trust_root(&store, &secrets);
    let raw = letter("lynx", "the lynx is signed by bea");
    let signed = String::from_utf8(sealed(&raw, Smime::Sign, &bea(), &[])).unwrap();
    let message = arrive(&store, forged(&signed).into_bytes());
    let (mut dom, mut seen) = reader(store, secrets, message.thread);
    let page = until(&mut dom, &mut seen, |page| page.contains("seal-line bad")).await;
    let said = lines(&page);
    assert_eq!(
        said[1],
        (
            "seal-line bad".to_owned(),
            "Bad S/MIME signature: it does not match its certificate, so it is forged or \
             damaged"
                .to_owned()
        ),
        "{said:?}"
    );
}

#[tokio::test]
async fn a_signature_from_an_authority_nobody_trusts_is_amber_and_says_why() {
    let (store, _dir) = seeded();
    let secrets = Arc::new(MapSecrets::default());
    trust_root(&store, &secrets);
    let stranger = identity(stranger_pki(), &Person::new("Carol", &[BEA], 13, 1003));
    let raw = letter("crane", "the crane is signed by a stranger");
    let message = arrive(&store, sealed(&raw, Smime::Sign, &stranger, &[]));
    let (mut dom, mut seen) = reader(store, secrets, message.thread);
    let page = until(&mut dom, &mut seen, |page| page.contains("seal-line warn")).await;
    let said = lines(&page);
    assert_eq!(said[1].0, "seal-line warn", "{said:?}");
    assert_eq!(
        said[2],
        (
            "seal-line warn".to_owned(),
            doubt(&CertProblem::Untrusted).replace('\'', "&#39;")
        )
    );
    assert!(!page.contains("seal-line good"), "{page}");
    assert!(page.contains("the crane is signed by a stranger"), "{page}");
    let drawn = seals(&page);
    assert!(drawn.contains("seal-line detail"), "{drawn}");
    let missing = crate::ui::style::tests::unstyled_classes(&drawn, crate::ui::style::STYLE);
    assert!(missing.is_empty(), "unstyled classes: {missing:?}");
}

#[tokio::test]
async fn an_encrypted_smime_message_shows_its_body_and_lists_what_is_attached_inside() {
    let (store, dir) = seeded();
    let secrets = Arc::new(MapSecrets::default());
    trust_root(&store, &secrets);
    with_identity(&store, &secrets);
    let raw = letter("zebra", "the zebra is under the mat");
    let message = arrive(
        &store,
        sealed(&raw, Smime::SignAndEncrypt, &bea(), &[me().cert]),
    );
    assert!(!message.body.text().unwrap_or_default().contains("zebra"));
    let (mut dom, mut seen) = reader(store, secrets, message.thread);
    let page = until(&mut dom, &mut seen, |page| {
        page.contains("the zebra is under the mat")
    })
    .await;
    let said = lines(&page);
    assert_eq!(
        said[0],
        (
            "seal-line good".to_owned(),
            "Encrypted with S/MIME, and decrypted for reading".to_owned()
        )
    );
    assert_eq!(said[1].0, "seal-line good", "{said:?}");
    assert!(
        page.contains("zebra plans"),
        "the subject from inside: {page}"
    );
    // The attachment inside is listed, and the ciphertext is not.
    assert!(page.contains("map.bin"), "{page}");
    assert!(!page.contains("smime.p7m"), "{page}");

    // Saved, it is the bytes that were attached.
    let saved =
        super::save_attachment(message.id, message.body.raw(), 0, &dir.path().join("out")).unwrap();
    assert_eq!(saved.file_name().unwrap(), "map.bin");
    assert_eq!(std::fs::read(saved).unwrap(), [0, 1, 2, 3, 4, 5, 6, 7]);
    assert!(
        super::save_attachment(message.id, message.body.raw(), 1, dir.path()).is_err(),
        "there is one attachment"
    );
}
