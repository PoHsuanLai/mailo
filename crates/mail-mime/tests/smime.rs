//! S/MIME (`plan.md` 10.16) on bytes: messages sealed and opened again, and messages built by
//! hand the way other clients write them — BER, opaque signed-data, triple DES, AES-GCM, SHA-1 —
//! against a throwaway certificate authority made in `smime_support`.

mod smime_support;

use cms::content_info::{CmsVersion, ContentInfo};
use cms::enveloped_data::{
    EncryptedContentInfo, EnvelopedData, KeyTransRecipientInfo, RecipientIdentifier, RecipientInfo,
    RecipientInfos,
};
use cms::signed_data::{
    CertificateSet, EncapsulatedContentInfo, SignedData, SignerIdentifier, SignerInfo, SignerInfos,
};
use der::asn1::SetOfVec;
use der::{Any, Decode, Encode};
use mail_domain::*;
use mail_mime::MimeError;
use mail_mime::smime::{self, Cert, Identity, Keys, Sealing};
use smime_support::*;
use spki::AlgorithmIdentifierOwned;

const ALICE: &str = "alice@example.test";
const BOB: &str = "bob@example.test";

fn alice() -> Identity {
    identity(pki(), &Person::new("Alice", &[ALICE], 10, 1001))
}

fn bob() -> Identity {
    identity(pki(), &Person::new("Bob", &[BOB], 11, 1002))
}

fn anchors() -> Vec<Cert> {
    vec![pki().root.cert.clone()]
}

/// Keys for opening as `me`, trusting the test root and holding no other certificates.
fn opened_by(raw: &[u8], me: &[Identity], anchors: &[Cert]) -> smime::Opened {
    smime::open(
        raw,
        &Keys {
            identities: me,
            certs: &[],
            anchors,
            now: now(),
        },
    )
    .expect("an S/MIME message")
}

fn sealed(mode: Smime, signer: Option<&Identity>, to: &[Cert], raw: &[u8]) -> Vec<u8> {
    smime::seal(
        raw,
        &Sealing {
            mode,
            signer,
            recipients: to,
            now: now(),
        },
        &mut rng(7),
    )
    .unwrap()
}

fn signed_by_alice() -> Vec<u8> {
    sealed(Smime::Sign, Some(&alice()), &[], &message(ALICE, BOB, DATE))
}

fn text_of(opened: &smime::Opened) -> String {
    mail_mime::parse(&opened.message)
        .unwrap()
        .text
        .unwrap_or_default()
        .replace("\r\n", "\n")
}

#[test]
fn a_signed_message_is_good_for_its_sender_through_a_trusted_chain() {
    let raw = signed_by_alice();
    let text = String::from_utf8_lossy(&raw);
    assert!(text.contains("multipart/signed"), "{text}");
    assert!(text.contains("micalg=sha-256"), "{text}");
    let opened = opened_by(&raw, &[], &anchors());
    assert_eq!(
        opened.verification,
        SmimeVerification::Good {
            signer: alice().cert.fingerprint(),
            coverage: Coverage::Whole,
        }
    );
    assert_eq!(opened.encryption, SmimeEncryption::NotEncrypted);
    let signer = opened.signer.clone().unwrap();
    assert_eq!(signer.cert, alice().cert);
    // The issuers the message carried, for the chain the client keeps beside the certificate.
    assert!(signer.chain.contains(&pki().intermediate.cert));
    assert_eq!(text_of(&opened), "Meet at noon.  \nBring the map. \n");
}

#[test]
fn a_signature_survives_line_ends_turned_to_lf_and_keeps_trailing_whitespace() {
    let raw = signed_by_alice();
    let lf: Vec<u8> = raw.iter().copied().filter(|b| *b != b'\r').collect();
    let opened = opened_by(&lf, &[], &anchors());
    assert!(
        matches!(opened.verification, SmimeVerification::Good { .. }),
        "{:?}",
        opened.verification
    );
    // A space taken off the end of a line is a change, and the signature says so.
    let trimmed = String::from_utf8(raw.clone()).unwrap().replacen(
        "Meet at noon.  \r\n",
        "Meet at noon.\r\n",
        1,
    );
    let opened = opened_by(trimmed.as_bytes(), &[], &anchors());
    assert_eq!(
        opened.verification,
        SmimeVerification::Bad {
            why: BadSignature::Altered,
            coverage: Coverage::Whole
        }
    );
}

#[test]
fn a_tampered_body_is_bad_and_says_it_was_altered() {
    let raw =
        String::from_utf8(signed_by_alice())
            .unwrap()
            .replacen("Meet at noon.", "Meet at dawn.", 1);
    let opened = opened_by(raw.as_bytes(), &[], &anchors());
    assert_eq!(
        opened.verification,
        SmimeVerification::Bad {
            why: BadSignature::Altered,
            coverage: Coverage::Whole
        }
    );
}

#[test]
fn a_signature_from_a_certificate_for_someone_else_names_the_from_it_does_not_match() {
    let raw = sealed(
        Smime::Sign,
        Some(&alice()),
        &[],
        &message("mallory@example.test", BOB, DATE),
    );
    let opened = opened_by(&raw, &[], &anchors());
    assert_eq!(
        opened.verification,
        SmimeVerification::Doubtful {
            signer: alice().cert.fingerprint(),
            problems: vec![CertProblem::NotFrom {
                from: "mallory@example.test".to_owned()
            }],
            coverage: Coverage::Whole,
        }
    );
}

#[test]
fn an_expired_certificate_is_doubtful_and_says_when_it_expired() {
    let mut person = Person::new("Old Alice", &[ALICE], 12, 1001);
    person.valid = (at(2024, 1, 1), at(2025, 1, 1));
    let old = identity(pki(), &person);
    let raw = sealed(Smime::Sign, Some(&old), &[], &message(ALICE, BOB, DATE));
    let opened = opened_by(&raw, &[], &anchors());
    assert_eq!(
        opened.verification,
        SmimeVerification::Doubtful {
            signer: old.cert.fingerprint(),
            problems: vec![CertProblem::Expired {
                not_after: at(2025, 1, 1)
            }],
            coverage: Coverage::Whole,
        }
    );
}

#[test]
fn a_chain_to_nothing_trusted_is_untrusted_until_the_user_trusts_a_certificate_in_it() {
    let stranger = identity(stranger_pki(), &Person::new("Carol", &[ALICE], 13, 1003));
    let raw = sealed(
        Smime::Sign,
        Some(&stranger),
        &[],
        &message(ALICE, BOB, DATE),
    );
    let opened = opened_by(&raw, &[], &anchors());
    assert_eq!(
        opened.verification,
        SmimeVerification::Doubtful {
            signer: stranger.cert.fingerprint(),
            problems: vec![CertProblem::Untrusted],
            coverage: Coverage::Whole,
        }
    );
    // The user trusts that root: the chain now ends somewhere.
    let opened = opened_by(&raw, &[], &[stranger_pki().root.cert.clone()]);
    assert!(
        matches!(opened.verification, SmimeVerification::Good { .. }),
        "{:?}",
        opened.verification
    );
    // Or trusts the certificate itself, which vouches for itself and nothing else.
    let opened = opened_by(&raw, &[], std::slice::from_ref(&stranger.cert));
    assert!(
        matches!(opened.verification, SmimeVerification::Good { .. }),
        "{:?}",
        opened.verification
    );
    let other = identity(stranger_pki(), &Person::new("Dan", &[ALICE], 14, 1004));
    let raw = sealed(Smime::Sign, Some(&other), &[], &message(ALICE, BOB, DATE));
    let opened = opened_by(&raw, &[], std::slice::from_ref(&stranger.cert));
    assert!(
        matches!(&opened.verification, SmimeVerification::Doubtful { problems, .. } if problems == &[CertProblem::Untrusted]),
        "a trusted end-entity certificate vouches for no one else: {:?}",
        opened.verification
    );
}

#[test]
fn a_certificate_not_for_mail_is_doubtful() {
    let mut person = Person::new("Server", &[ALICE], 15, 1001);
    person.purposes = Some(vec![SERVER_AUTH]);
    let server = identity(pki(), &person);
    let raw = sealed(Smime::Sign, Some(&server), &[], &message(ALICE, BOB, DATE));
    let opened = opened_by(&raw, &[], &anchors());
    assert_eq!(
        opened.verification,
        SmimeVerification::Doubtful {
            signer: server.cert.fingerprint(),
            problems: vec![CertProblem::NotForEmail],
            coverage: Coverage::Whole,
        }
    );
    assert!(!server.cert.can_encrypt());
}

#[test]
fn an_address_only_in_the_subject_counts_when_there_is_no_alternative_name() {
    // RFC 8550 §3: the subject's emailAddress is read only in the absence of rfc822Names.
    let cert = alice().cert;
    assert_eq!(cert.emails(), vec![ALICE]);
    assert!(cert.is_for("Alice@Example.Test"));
    assert!(!cert.is_for("xalice@example.test"));
}

#[test]
fn an_encrypted_message_opens_for_each_recipient_and_for_the_sender() {
    let raw = sealed(
        Smime::Encrypt,
        None,
        &[bob().cert, alice().cert],
        &message(ALICE, BOB, DATE),
    );
    let text = String::from_utf8_lossy(&raw);
    assert!(text.contains("smime-type=enveloped-data"), "{text}");
    assert!(
        !text.contains("Meet at noon"),
        "the body is not in the clear"
    );
    assert!(
        text.contains("Subject: The plan"),
        "the header stays outside"
    );
    for me in [bob(), alice()] {
        let opened = opened_by(&raw, std::slice::from_ref(&me), &anchors());
        assert_eq!(opened.encryption, SmimeEncryption::Decrypted);
        assert_eq!(opened.verification, SmimeVerification::NoSignature);
        assert_eq!(text_of(&opened), "Meet at noon.  \nBring the map. \n");
    }
    let named = smime::recipients(&raw).unwrap();
    assert_eq!(named.len(), 2);
    assert!(named.iter().any(|id| id.names(&bob().cert)));
    assert!(named.iter().any(|id| id.names(&alice().cert)));
    assert!(smime::recipients(&message(ALICE, BOB, DATE)).is_none());
}

#[test]
fn signed_then_encrypted_opens_to_a_good_signature() {
    let raw = sealed(
        Smime::SignAndEncrypt,
        Some(&alice()),
        &[bob().cert, alice().cert],
        &message(ALICE, BOB, DATE),
    );
    let opened = opened_by(&raw, &[bob()], &anchors());
    assert_eq!(opened.encryption, SmimeEncryption::Decrypted);
    assert_eq!(
        opened.verification,
        SmimeVerification::Good {
            signer: alice().cert.fingerprint(),
            coverage: Coverage::Whole,
        }
    );
    assert_eq!(text_of(&opened), "Meet at noon.  \nBring the map. \n");
}

#[test]
fn a_message_encrypted_to_someone_else_names_who_it_is_for() {
    let raw = sealed(
        Smime::Encrypt,
        None,
        &[bob().cert],
        &message(ALICE, BOB, DATE),
    );
    let opened = opened_by(&raw, &[alice()], &anchors());
    let SmimeEncryption::CannotDecrypt { to } = &opened.encryption else {
        panic!("{:?}", opened.encryption);
    };
    assert_eq!(to.len(), 1);
    assert!(to[0].contains(&bob().cert.serial()), "{to:?}");
    assert!(to[0].contains("Example Test Mail CA"), "{to:?}");
    assert_eq!(opened.message, raw, "nothing is shown that was not there");
}

#[test]
fn an_enveloped_message_written_as_streaming_ber_opens() {
    let raw = sealed(
        Smime::SignAndEncrypt,
        Some(&alice()),
        &[bob().cert],
        &message(ALICE, BOB, DATE),
    );
    let der = cms_of(&raw);
    let ber = as_streaming_ber(&der);
    assert_ne!(ber, der);
    assert!(
        ContentInfo::from_der(&ber).is_err(),
        "the der crate alone refuses it"
    );
    let head = format!("From: {ALICE}\r\nTo: {BOB}\r\nSubject: The plan\r\nDate: {DATE}\r\n");
    let raw = pkcs7_message(&head, "enveloped-data", &ber);
    let opened = opened_by(&raw, &[bob()], &anchors());
    assert_eq!(opened.encryption, SmimeEncryption::Decrypted);
    assert!(
        matches!(opened.verification, SmimeVerification::Good { .. }),
        "{:?}",
        opened.verification
    );
    assert_eq!(smime::recipients(&raw).unwrap().len(), 1);
}

/// SignedData over `content` by `signer`, encapsulating the content when `encapsulate` says, with
/// `digest` and signed attributes stating `signed_at`.
fn signed_data(
    signer: &Identity,
    content: &[u8],
    encapsulate: bool,
    sha1: bool,
    signed_at: chrono::DateTime<chrono::Utc>,
) -> Vec<u8> {
    use sha2::Digest;
    let (digest_oid, hash): (&str, Vec<u8>) = if sha1 {
        ("1.3.14.3.2.26", sha1::Sha1::digest(content).to_vec())
    } else {
        (
            "2.16.840.1.101.3.4.2.1",
            sha2::Sha256::digest(content).to_vec(),
        )
    };
    let signing_time = x509_cert::time::Time::UtcTime(
        der::asn1::UtcTime::from_unix_duration(std::time::Duration::from_secs(
            signed_at.timestamp() as u64,
        ))
        .unwrap(),
    );
    let attrs = SetOfVec::try_from(vec![
        attribute(
            "1.2.840.113549.1.9.3",
            &const_oid::ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.1"),
        ),
        attribute("1.2.840.113549.1.9.5", &signing_time),
        attribute("1.2.840.113549.1.9.4", &octets(&hash)),
    ])
    .unwrap();
    let to_sign = attrs.to_der().unwrap();
    let smime::PrivateKey::Rsa(key) = &signer.key else {
        panic!("an RSA identity");
    };
    let signature = if sha1 {
        key.sign(
            rsa::Pkcs1v15Sign::new::<sha1::Sha1>(),
            &sha1::Sha1::digest(&to_sign),
        )
    } else {
        key.sign(
            rsa::Pkcs1v15Sign::new::<sha2::Sha256>(),
            &sha2::Sha256::digest(&to_sign),
        )
    }
    .unwrap();
    let digest_alg = AlgorithmIdentifierOwned {
        oid: const_oid::ObjectIdentifier::new_unwrap(digest_oid),
        parameters: None,
    };
    let parsed = x509_cert::Certificate::from_der(signer.cert.der()).unwrap();
    let data = SignedData {
        version: CmsVersion::V1,
        digest_algorithms: SetOfVec::try_from(vec![digest_alg.clone()]).unwrap(),
        encap_content_info: EncapsulatedContentInfo {
            econtent_type: const_oid::ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.1"),
            econtent: encapsulate
                .then(|| Any::from_der(&octets(content).to_der().unwrap()).unwrap()),
        },
        certificates: Some(
            CertificateSet::try_from(
                std::iter::once(parsed)
                    .chain(
                        signer
                            .chain
                            .iter()
                            .map(|c| x509_cert::Certificate::from_der(c.der()).unwrap()),
                    )
                    .map(cms::cert::CertificateChoices::Certificate)
                    .collect::<Vec<_>>(),
            )
            .unwrap(),
        ),
        crls: None,
        signer_infos: SignerInfos::try_from(vec![SignerInfo {
            version: CmsVersion::V1,
            sid: SignerIdentifier::IssuerAndSerialNumber(issuer_and_serial(&signer.cert)),
            digest_alg,
            signed_attrs: Some(attrs),
            signature_algorithm: AlgorithmIdentifierOwned {
                oid: const_oid::ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.1"),
                parameters: None,
            },
            signature: octets(&signature),
            unsigned_attrs: None,
        }])
        .unwrap(),
    };
    ContentInfo {
        content_type: const_oid::ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.2"),
        content: Any::from_der(&data.to_der().unwrap()).unwrap(),
    }
    .to_der()
    .unwrap()
}

const INNER: &[u8] = b"Content-Type: text/plain; charset=utf-8\r\n\r\nInside the envelope.\r\n";

fn head() -> String {
    format!("From: Alice <{ALICE}>\r\nTo: {BOB}\r\nSubject: Opaque\r\nDate: {DATE}\r\n")
}

#[test]
fn an_opaque_signed_data_message_is_read_and_checked() {
    let der = signed_data(&alice(), INNER, true, false, now());
    let raw = pkcs7_message(&head(), "signed-data", &der);
    let opened = opened_by(&raw, &[], &anchors());
    assert_eq!(
        opened.verification,
        SmimeVerification::Good {
            signer: alice().cert.fingerprint(),
            coverage: Coverage::Whole,
        }
    );
    assert_eq!(text_of(&opened), "Inside the envelope.\n");
    assert!(
        String::from_utf8_lossy(&opened.message).contains("Subject: Opaque"),
        "the outer header is kept"
    );
}

#[test]
fn sha1_is_refused_for_a_signature_made_after_2020_and_read_before() {
    let recent = signed_data(&alice(), INNER, true, true, now());
    let opened = opened_by(
        &pkcs7_message(&head(), "signed-data", &recent),
        &[],
        &anchors(),
    );
    assert_eq!(
        opened.verification,
        SmimeVerification::Bad {
            why: BadSignature::Weak {
                algorithm: "SHA-1".to_owned()
            },
            coverage: Coverage::Whole,
        }
    );
    // Signed in 2020 by a certificate valid then: the old signature is still read.
    let mut person = Person::new("Alice 2020", &[ALICE], 16, 1001);
    person.valid = (at(2020, 3, 1), at(2030, 1, 1));
    let then = identity(pki(), &person);
    let old = signed_data(&then, INNER, true, true, at(2020, 6, 1));
    let opened = opened_by(
        &pkcs7_message(&head(), "signed-data", &old),
        &[],
        &anchors(),
    );
    assert!(
        matches!(opened.verification, SmimeVerification::Good { .. }),
        "{:?}",
        opened.verification
    );
}

#[test]
fn a_signer_neither_carried_nor_held_is_unknown() {
    let mut der = signed_data(&alice(), INNER, true, false, now());
    // The same SignedData with its certificates taken out.
    let info = ContentInfo::from_der(&der).unwrap();
    let mut data = SignedData::from_der(&info.content.to_der().unwrap()).unwrap();
    data.certificates = None;
    der = ContentInfo {
        content_type: info.content_type,
        content: Any::from_der(&data.to_der().unwrap()).unwrap(),
    }
    .to_der()
    .unwrap();
    let raw = pkcs7_message(&head(), "signed-data", &der);
    let opened = opened_by(&raw, &[], &anchors());
    assert_eq!(
        opened.verification,
        SmimeVerification::UnknownSigner {
            coverage: Coverage::Whole
        }
    );
    // Held by the client from an earlier message, it is found.
    let held = smime::open(
        &raw,
        &Keys {
            identities: &[],
            certs: &[alice().cert, pki().intermediate.cert.clone()],
            anchors: &anchors(),
            now: now(),
        },
    )
    .unwrap();
    assert!(
        matches!(held.verification, SmimeVerification::Good { .. }),
        "{:?}",
        held.verification
    );
}

#[test]
fn a_signed_part_beside_unsigned_text_covers_only_part() {
    let signed = signed_by_alice();
    let (head_end, body_at) = {
        let text = String::from_utf8_lossy(&signed);
        let at = text.find("\r\n\r\n").unwrap();
        (at, at + 4)
    };
    let outer: String = String::from_utf8_lossy(&signed[..head_end])
        .lines()
        .filter(|l| !l.starts_with("Content-Type") && !l.starts_with(' ') && !l.starts_with("MIME"))
        .map(|l| format!("{l}\r\n"))
        .collect();
    let content_type = String::from_utf8_lossy(&signed[..head_end])
        .split("Content-Type: ")
        .nth(1)
        .unwrap()
        .to_owned();
    let mut raw = format!(
        "{outer}MIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"outer\"\r\n\r\n\
         --outer\r\nContent-Type: {content_type}\r\n\r\n"
    )
    .into_bytes();
    raw.extend_from_slice(&signed[body_at..]);
    raw.extend_from_slice(
        b"\r\n--outer\r\nContent-Type: text/plain\r\n\r\nA footer the list added.\r\n--outer--\r\n",
    );
    let opened = opened_by(&raw, &[], &anchors());
    assert_eq!(
        opened.verification,
        SmimeVerification::Good {
            signer: alice().cert.fingerprint(),
            coverage: Coverage::Part,
        }
    );
}

/// EnvelopedData to `to` with the content encrypted as `algorithm` says, built by hand the way
/// another client might.
fn envelope(to: &Cert, content: &[u8], algorithm: &str) -> Vec<u8> {
    use cbc::cipher::{BlockEncryptMut, KeyIvInit, block_padding::Pkcs7};
    let mut rng = rng(99);
    let (oid, key, iv, ciphertext) = match algorithm {
        "3des" => {
            let key = [0x11u8; 24];
            let iv = [0x22u8; 8];
            let ct = cbc::Encryptor::<des::TdesEde3>::new_from_slices(&key, &iv)
                .unwrap()
                .encrypt_padded_vec_mut::<Pkcs7>(content);
            ("1.2.840.113549.3.7", key.to_vec(), iv.to_vec(), ct)
        }
        _ => unreachable!(),
    };
    let public = rsa::RsaPublicKey::from_public_key_der_for_test(to);
    let wrapped = public
        .encrypt(&mut rng, rsa::Pkcs1v15Encrypt, &key)
        .unwrap();
    let data = EnvelopedData {
        version: CmsVersion::V0,
        originator_info: None,
        recip_infos: RecipientInfos(
            SetOfVec::try_from(vec![RecipientInfo::Ktri(KeyTransRecipientInfo {
                version: CmsVersion::V0,
                rid: RecipientIdentifier::IssuerAndSerialNumber(issuer_and_serial(to)),
                key_enc_alg: AlgorithmIdentifierOwned {
                    oid: const_oid::ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.1"),
                    parameters: None,
                },
                enc_key: octets(&wrapped),
            })])
            .unwrap(),
        ),
        encrypted_content: EncryptedContentInfo {
            content_type: const_oid::ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.1"),
            content_enc_alg: AlgorithmIdentifierOwned {
                oid: const_oid::ObjectIdentifier::new_unwrap(oid),
                parameters: Some(Any::from_der(&octets(&iv).to_der().unwrap()).unwrap()),
            },
            encrypted_content: Some(octets(&ciphertext)),
        },
        unprotected_attrs: None,
    };
    ContentInfo {
        content_type: const_oid::ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.3"),
        content: Any::from_der(&data.to_der().unwrap()).unwrap(),
    }
    .to_der()
    .unwrap()
}

trait FromCert {
    fn from_public_key_der_for_test(cert: &Cert) -> Self;
}

impl FromCert for rsa::RsaPublicKey {
    fn from_public_key_der_for_test(cert: &Cert) -> Self {
        use spki::DecodePublicKey;
        let parsed = x509_cert::Certificate::from_der(cert.der()).unwrap();
        let spki = parsed
            .tbs_certificate
            .subject_public_key_info
            .to_der()
            .unwrap();
        rsa::RsaPublicKey::from_public_key_der(&spki).unwrap()
    }
}

#[test]
fn triple_des_is_read_in_old_mail_and_refused_in_new() {
    let der = envelope(&bob().cert, INNER, "3des");
    let old_head = format!(
        "From: {ALICE}\r\nTo: {BOB}\r\nSubject: Old\r\nDate: Mon, 3 Mar 2014 09:00:00 +0000\r\n"
    );
    let opened = opened_by(
        &pkcs7_message(&old_head, "enveloped-data", &der),
        &[bob()],
        &[],
    );
    assert_eq!(opened.encryption, SmimeEncryption::Decrypted);
    assert_eq!(text_of(&opened), "Inside the envelope.\n");

    let opened = opened_by(
        &pkcs7_message(&head(), "enveloped-data", &der),
        &[bob()],
        &[],
    );
    let SmimeEncryption::Unreadable { why } = &opened.encryption else {
        panic!("{:?}", opened.encryption);
    };
    assert!(why.contains("triple DES"), "{why}");
}

/// AuthEnvelopedData with AES-128-GCM (RFC 5083, RFC 5084), built by hand.
fn auth_envelope(to: &Cert, content: &[u8], tamper: bool) -> Vec<u8> {
    use aes_gcm::aead::{Aead, KeyInit};
    let key = [0x33u8; 16];
    let nonce = [0x44u8; 12];
    let mut sealed = aes_gcm::Aes128Gcm::new_from_slice(&key)
        .unwrap()
        .encrypt(aes_gcm::Nonce::from_slice(&nonce), content)
        .unwrap();
    if tamper {
        sealed[0] ^= 1;
    }
    let (ciphertext, tag) = sealed.split_at(sealed.len() - 16);
    let public = rsa::RsaPublicKey::from_public_key_der_for_test(to);
    let wrapped = public
        .encrypt(&mut rng(5), rsa::Pkcs1v15Encrypt, &key)
        .unwrap();
    #[derive(der::Sequence)]
    struct Gcm {
        nonce: der::asn1::OctetString,
        icv_len: u8,
    }
    #[derive(der::Sequence)]
    struct AuthEnveloped {
        version: CmsVersion,
        recip_infos: RecipientInfos,
        auth_encrypted_content_info: EncryptedContentInfo,
        mac: der::asn1::OctetString,
    }
    let params = Gcm {
        nonce: octets(&nonce),
        icv_len: 16,
    };
    let data = AuthEnveloped {
        version: CmsVersion::V0,
        recip_infos: RecipientInfos(
            SetOfVec::try_from(vec![RecipientInfo::Ktri(KeyTransRecipientInfo {
                version: CmsVersion::V0,
                rid: RecipientIdentifier::IssuerAndSerialNumber(issuer_and_serial(to)),
                key_enc_alg: AlgorithmIdentifierOwned {
                    oid: const_oid::ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.1"),
                    parameters: None,
                },
                enc_key: octets(&wrapped),
            })])
            .unwrap(),
        ),
        auth_encrypted_content_info: EncryptedContentInfo {
            content_type: const_oid::ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.1"),
            content_enc_alg: AlgorithmIdentifierOwned {
                oid: const_oid::ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.1.6"),
                parameters: Some(Any::from_der(&params.to_der().unwrap()).unwrap()),
            },
            encrypted_content: Some(octets(ciphertext)),
        },
        mac: octets(tag),
    };
    ContentInfo {
        content_type: const_oid::ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.16.1.23"),
        content: Any::from_der(&data.to_der().unwrap()).unwrap(),
    }
    .to_der()
    .unwrap()
}

#[test]
fn auth_enveloped_aes_gcm_opens_and_a_tampered_one_does_not() {
    let raw = pkcs7_message(
        &head(),
        "authEnveloped-data",
        &auth_envelope(&bob().cert, INNER, false),
    );
    let opened = opened_by(&raw, &[bob()], &[]);
    assert_eq!(opened.encryption, SmimeEncryption::Decrypted);
    assert_eq!(text_of(&opened), "Inside the envelope.\n");
    assert_eq!(smime::recipients(&raw).unwrap().len(), 1);

    let raw = pkcs7_message(
        &head(),
        "authEnveloped-data",
        &auth_envelope(&bob().cert, INNER, true),
    );
    let opened = opened_by(&raw, &[bob()], &[]);
    let SmimeEncryption::Unreadable { why } = &opened.encryption else {
        panic!("{:?}", opened.encryption);
    };
    assert!(why.contains("integrity"), "{why}");
}

#[test]
fn an_identity_round_trips_through_pkcs12_and_a_wrong_password_is_named() {
    let me = alice();
    let file = smime::write_pkcs12(&me, "correct horse", &mut rng(3)).unwrap();
    let back = smime::read_pkcs12(&file, "correct horse").unwrap();
    assert_eq!(back, me);
    assert_eq!(
        back.chain[0],
        pki().intermediate.cert,
        "nearest issuer first"
    );
    assert!(matches!(
        smime::read_pkcs12(&file, "wrong"),
        Err(MimeError::WrongPassword)
    ));
    assert!(smime::read_pkcs12(b"not a pfx", "x").is_err());
    // What the keyring keeps reads back as the same key.
    let pem = me.key.to_pkcs8_pem().unwrap();
    assert!(pem.starts_with("-----BEGIN PRIVATE KEY-----"));
    assert_eq!(smime::PrivateKey::from_pkcs8_pem(&pem).unwrap(), me.key);
    assert!(!format!("{:?}", me.key).contains(&pem[30..60]));
}

#[test]
fn a_key_that_is_not_the_certificates_is_refused() {
    assert!(Identity::new(bob().key, alice().cert, Vec::new()).is_err());
}

#[test]
fn certificates_read_from_pem_and_der_alike() {
    let pem = format!("{}{}", alice().cert.pem(), pki().root.cert.pem());
    let read = smime::read_certs(pem.as_bytes()).unwrap();
    assert_eq!(read, vec![alice().cert, pki().root.cert.clone()]);
    assert_eq!(
        smime::read_certs(alice().cert.der()).unwrap(),
        vec![alice().cert]
    );
}

#[test]
fn a_plain_message_carries_no_smime() {
    assert!(
        smime::open(
            &message(ALICE, BOB, DATE),
            &Keys {
                identities: &[],
                certs: &[],
                anchors: &[],
                now: now(),
            }
        )
        .is_none()
    );
    assert_eq!(
        sealed(Smime::None, None, &[], &message(ALICE, BOB, DATE)),
        message(ALICE, BOB, DATE)
    );
}
