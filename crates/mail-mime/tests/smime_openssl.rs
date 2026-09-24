//! S/MIME against another implementation: OpenSSL's command line reads what this client writes,
//! and this client reads what OpenSSL writes — signatures, enveloped and AES-GCM
//! authEnveloped data (which OpenSSL streams as BER), and PKCS#12 files.
//!
//! `#[ignore]`d: it runs the `openssl` binary, which a test may not assume is installed. Run it
//! by hand with `cargo test -p mail-mime --test smime_openssl -- --ignored`; each test says so
//! and passes vacuously when there is no `openssl` on the PATH.

mod smime_support;

use mail_domain::*;
use mail_mime::smime::{self, Identity, Keys, Sealing};
use smime_support::*;
use std::path::Path;
use std::process::Command;

const ALICE: &str = "alice@example.test";
const BOB: &str = "bob@example.test";

fn alice() -> Identity {
    identity(pki(), &Person::new("Alice", &[ALICE], 30, 3001))
}

fn bob() -> Identity {
    identity(pki(), &Person::new("Bob", &[BOB], 31, 3002))
}

fn openssl_here() -> bool {
    Command::new("openssl").arg("version").output().is_ok()
}

fn openssl(dir: &Path, args: &[&str], input: Option<&[u8]>) -> (bool, Vec<u8>, String) {
    use std::io::Write;
    let mut child = Command::new("openssl")
        .args(args)
        .current_dir(dir)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    if let Some(input) = input {
        child.stdin.take().unwrap().write_all(input).unwrap();
    }
    drop(child.stdin.take());
    let out = child.wait_with_output().unwrap();
    (
        out.status.success(),
        out.stdout,
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// A directory with the root, the intermediate, and both people's certificates and keys as
/// OpenSSL reads them.
fn workspace() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let write = |name: &str, text: &str| std::fs::write(dir.path().join(name), text).unwrap();
    write("root.pem", &pki().root.cert.pem());
    write("inter.pem", &pki().intermediate.cert.pem());
    for (name, who) in [("alice", alice()), ("bob", bob())] {
        write(&format!("{name}.pem"), &who.cert.pem());
        write(&format!("{name}.key"), &who.key.to_pkcs8_pem().unwrap());
    }
    dir
}

fn keys<'a>(me: &'a [Identity], anchors: &'a [smime::Cert]) -> Keys<'a> {
    Keys {
        identities: me,
        certs: &[],
        anchors,
        now: now(),
    }
}

#[test]
#[ignore = "runs the openssl binary"]
fn openssl_verifies_and_decrypts_what_this_client_sends() {
    if !openssl_here() {
        return;
    }
    let dir = workspace();
    let sealed = smime::seal(
        &message(ALICE, BOB, DATE),
        &Sealing {
            mode: Smime::Sign,
            signer: Some(&alice()),
            recipients: &[],
            now: now(),
        },
        &mut rng(1),
    )
    .unwrap();
    let (ok, out, err) = openssl(
        dir.path(),
        &[
            "smime",
            "-verify",
            "-CAfile",
            "root.pem",
            "-attime",
            "1790000000",
        ],
        Some(&sealed),
    );
    assert!(ok, "{err}");
    assert!(String::from_utf8_lossy(&out).contains("Meet at noon.  "));

    let sealed = smime::seal(
        &message(ALICE, BOB, DATE),
        &Sealing {
            mode: Smime::SignAndEncrypt,
            signer: Some(&alice()),
            recipients: &[bob().cert],
            now: now(),
        },
        &mut rng(2),
    )
    .unwrap();
    let (ok, out, err) = openssl(
        dir.path(),
        &[
            "smime", "-decrypt", "-recip", "bob.pem", "-inkey", "bob.key",
        ],
        Some(&sealed),
    );
    assert!(ok, "{err}");
    let (ok, out, err) = openssl(
        dir.path(),
        &[
            "smime",
            "-verify",
            "-CAfile",
            "root.pem",
            "-attime",
            "1790000000",
        ],
        Some(&out),
    );
    assert!(ok, "{err}");
    assert!(String::from_utf8_lossy(&out).contains("Bring the map."));
}

#[test]
#[ignore = "runs the openssl binary"]
fn this_client_reads_what_openssl_signs_and_encrypts() {
    if !openssl_here() {
        return;
    }
    let dir = workspace();
    let body = b"Content-Type: text/plain; charset=utf-8\r\n\r\nFrom the other side.  \r\n";
    let anchors = [pki().root.cert.clone()];
    let (ok, signed, err) = openssl(
        dir.path(),
        &[
            "smime",
            "-sign",
            "-signer",
            "alice.pem",
            "-inkey",
            "alice.key",
            "-certfile",
            "inter.pem",
            "-from",
            ALICE,
            "-to",
            BOB,
            "-subject",
            "hi",
        ],
        Some(body),
    );
    assert!(ok, "{err}");
    let opened = smime::open(&signed, &keys(&[], &anchors)).unwrap();
    assert!(
        matches!(opened.verification, SmimeVerification::Good { .. }),
        "{:?}",
        opened.verification
    );
    // Opaque signed-data, `-nodetach`.
    let (ok, opaque, err) = openssl(
        dir.path(),
        &[
            "smime",
            "-sign",
            "-nodetach",
            "-signer",
            "alice.pem",
            "-inkey",
            "alice.key",
            "-certfile",
            "inter.pem",
            "-from",
            ALICE,
        ],
        Some(body),
    );
    assert!(ok, "{err}");
    let opened = smime::open(&opaque, &keys(&[], &anchors)).unwrap();
    assert!(
        matches!(opened.verification, SmimeVerification::Good { .. }),
        "{:?}",
        opened.verification
    );

    for cipher in ["-aes256", "-aes128"] {
        let (ok, encrypted, err) = openssl(
            dir.path(),
            &["smime", "-encrypt", cipher, "-from", ALICE, "bob.pem"],
            Some(&signed),
        );
        assert!(ok, "{err}");
        let opened = smime::open(&encrypted, &keys(&[bob()], &anchors)).unwrap();
        assert_eq!(opened.encryption, SmimeEncryption::Decrypted, "{cipher}");
        assert!(
            matches!(opened.verification, SmimeVerification::Good { .. }),
            "{cipher}: {:?}",
            opened.verification
        );
    }
    // AES-GCM, as AuthEnvelopedData.
    let (ok, gcm, err) = openssl(
        dir.path(),
        &[
            "cms",
            "-encrypt",
            "-aes-128-gcm",
            "-from",
            ALICE,
            "-recip",
            "bob.pem",
        ],
        Some(body),
    );
    assert!(ok, "{err}");
    let opened = smime::open(&gcm, &keys(&[bob()], &anchors)).unwrap();
    assert_eq!(opened.encryption, SmimeEncryption::Decrypted);
    // RSA-OAEP key transport.
    let (ok, oaep, err) = openssl(
        dir.path(),
        &[
            "cms",
            "-encrypt",
            "-aes256",
            "-recip",
            "bob.pem",
            "-keyopt",
            "rsa_padding_mode:oaep",
        ],
        Some(body),
    );
    assert!(ok, "{err}");
    let opened = smime::open(&oaep, &keys(&[bob()], &anchors)).unwrap();
    assert_eq!(opened.encryption, SmimeEncryption::Decrypted);
}

#[test]
#[ignore = "runs the openssl binary"]
fn pkcs12_files_go_both_ways() {
    if !openssl_here() {
        return;
    }
    let dir = workspace();
    for extra in [&[][..], &["-legacy"][..]] {
        let mut args = vec![
            "pkcs12",
            "-export",
            "-in",
            "alice.pem",
            "-inkey",
            "alice.key",
            "-certfile",
            "inter.pem",
            "-passout",
            "pass:secret",
            "-out",
            "alice.p12",
        ];
        args.extend_from_slice(extra);
        let (ok, _, err) = openssl(dir.path(), &args, None);
        if !ok && !extra.is_empty() {
            // The legacy provider is not always built in; nothing to check without it.
            continue;
        }
        assert!(ok, "{err}");
        let file = std::fs::read(dir.path().join("alice.p12")).unwrap();
        let read = smime::read_pkcs12(&file, "secret");
        if extra.is_empty() {
            let read = read.unwrap();
            assert_eq!(read.cert, alice().cert);
            assert_eq!(read.chain, vec![pki().intermediate.cert.clone()]);
        } else {
            // Legacy exports encrypt the certificates with 40-bit RC2, which is refused with a
            // way out named.
            match read {
                Ok(read) => assert_eq!(read.cert, alice().cert),
                Err(e) => assert!(e.to_string().contains("export it again"), "{e}"),
            }
        }
    }
    let ours = smime::write_pkcs12(&alice(), "secret", &mut rng(3)).unwrap();
    std::fs::write(dir.path().join("ours.p12"), ours).unwrap();
    let (ok, out, err) = openssl(
        dir.path(),
        &[
            "pkcs12",
            "-in",
            "ours.p12",
            "-passin",
            "pass:secret",
            "-nodes",
        ],
        None,
    );
    assert!(ok, "{err}");
    let text = String::from_utf8_lossy(&out);
    assert!(text.contains("BEGIN PRIVATE KEY") && text.contains("BEGIN CERTIFICATE"));
}
