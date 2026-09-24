//! Web Key Directory lookup against a directory in this process.
//!
//! No network. The "directory" is a `TcpListener` behind TLS with a self-signed certificate from
//! `tests/fixtures/wkd-tls/` (for `wkd.test`, `openpgpkey.wkd.test` and `direct-only.test`,
//! valid until 2126), and the client is told to trust that certificate and to resolve those names
//! to the listener. The URLs are the ones `mail_mime::openpgp::wkd::urls` derives, with the
//! listener's port added — a directory on port 443 is not something a test can open.

use chrono::{TimeZone, Utc};
use mail_mime::openpgp::{self, SecretCert, wkd};
use rand::SeedableRng;
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const CERT: &[u8] = include_bytes!("fixtures/wkd-tls/cert.pem");
const KEY: &[u8] = include_bytes!("fixtures/wkd-tls/key.pem");

/// The request lines the directory received, host and path, in order.
type Seen = Arc<Mutex<Vec<String>>>;

fn key_for(address: &str, seed: u64) -> SecretCert {
    let at = Utc.with_ymd_and_hms(2026, 9, 24, 12, 0, 0).unwrap();
    openpgp::generate(
        &format!("Someone <{address}>"),
        at,
        &mut rand::rngs::StdRng::seed_from_u64(seed),
    )
    .unwrap()
}

/// A TLS directory. `answer(host, path)` is the body to serve, or `None` for a 404.
async fn directory(
    answer: impl Fn(&str, &str) -> Option<Vec<u8>> + Send + Sync + 'static,
) -> (SocketAddr, Seen) {
    let certs = vec![CertificateDer::from_pem_slice(CERT).unwrap()];
    let key = PrivateKeyDer::from_pem_slice(KEY).unwrap();
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(certs, key)
    .unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let seen: Seen = Arc::default();
    let recording = seen.clone();
    tokio::spawn(async move {
        while let Ok((sock, _)) = listener.accept().await {
            let Ok(mut stream) = acceptor.accept(sock).await else {
                continue;
            };
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                match stream.read(&mut chunk).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => buf.extend_from_slice(&chunk[..n]),
                }
            }
            let text = String::from_utf8_lossy(&buf).to_string();
            let path = text.split_whitespace().nth(1).unwrap_or("").to_owned();
            let host = text
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("host")
                        .then(|| value.trim().split(':').next().unwrap_or("").to_owned())
                })
                .unwrap_or_default();
            recording.lock().unwrap().push(format!("{host}{path}"));
            let response = match answer(&host, &path) {
                Some(body) => {
                    let mut out = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\n\
                         Content-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    )
                    .into_bytes();
                    out.extend_from_slice(&body);
                    out
                }
                None => b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    .to_vec(),
            };
            let _ = stream.write_all(&response).await;
            let _ = stream.shutdown().await;
        }
    });
    (addr, seen)
}

/// The client `mailo pgp lookup` uses, trusting the fixture and resolving the test names here.
fn client(to: SocketAddr) -> reqwest::Client {
    let root = reqwest::Certificate::from_pem(CERT).unwrap();
    mail_runtime::wkd::client_builder()
        .add_root_certificate(root)
        .resolve("wkd.test", to)
        .resolve("openpgpkey.wkd.test", to)
        .resolve("direct-only.test", to)
        // The advanced host of `direct-only.test` resolves to a loopback address nothing listens
        // on, so the name is never sent to a real resolver and the connection is refused.
        .resolve(
            "openpgpkey.direct-only.test",
            SocketAddr::from(([127, 0, 0, 2], to.port())),
        )
        .build()
        .unwrap()
}

/// `address`'s URLs with `port` on each host.
fn at_port(address: &str, port: u16) -> wkd::WkdUrls {
    let urls = wkd::urls(address).unwrap();
    wkd::WkdUrls {
        advanced: urls.advanced.replacen(
            &format!("https://{}/", urls.advanced_host),
            &format!("https://{}:{port}/", urls.advanced_host),
            1,
        ),
        direct: urls.direct.replacen(
            &format!("https://{}/", urls.direct_host),
            &format!("https://{}:{port}/", urls.direct_host),
            1,
        ),
        ..urls
    }
}

#[tokio::test]
async fn the_advanced_method_is_asked_first_at_the_drafts_path() {
    let joe = key_for("Joe.Doe@wkd.test", 1);
    let served = joe.public().to_bytes();
    let (addr, seen) =
        directory(move |host, _| (host == "openpgpkey.wkd.test").then(|| served.clone())).await;
    let found = mail_runtime::wkd::fetch(
        &client(addr),
        &at_port("Joe.Doe@wkd.test", addr.port()),
        "Joe.Doe@wkd.test",
    )
    .await
    .expect("the advanced method has it");
    assert_eq!(found.fingerprint(), joe.fingerprint());
    let hash = wkd::zbase32(&{
        use sha1::Digest;
        sha1::Sha1::digest(b"joe.doe")
    });
    assert_eq!(
        *seen.lock().unwrap(),
        vec![format!(
            "openpgpkey.wkd.test/.well-known/openpgpkey/wkd.test/hu/{hash}?l=Joe.Doe"
        )],
        "one request, to the advanced host, and the direct one never asked"
    );
}

#[tokio::test]
async fn the_direct_method_answers_when_the_advanced_one_has_nothing() {
    let joe = key_for("joe@wkd.test", 2);
    let served = joe.public().to_bytes();
    let (addr, seen) = directory(move |host, _| (host == "wkd.test").then(|| served.clone())).await;
    let found = mail_runtime::wkd::fetch(
        &client(addr),
        &at_port("joe@wkd.test", addr.port()),
        "joe@wkd.test",
    )
    .await
    .expect("the direct method has it");
    assert_eq!(found.fingerprint(), joe.fingerprint());
    let asked: Vec<String> = seen
        .lock()
        .unwrap()
        .iter()
        .map(|r| r.split('/').next().unwrap().to_owned())
        .collect();
    assert_eq!(asked, vec!["openpgpkey.wkd.test", "wkd.test"]);
    assert!(seen.lock().unwrap()[1].starts_with("wkd.test/.well-known/openpgpkey/hu/"));
}

#[tokio::test]
async fn an_advanced_host_that_does_not_answer_falls_back_to_the_direct_one() {
    let joe = key_for("joe@direct-only.test", 3);
    let served = joe.public().to_bytes();
    let (addr, _) = directory(move |_, _| Some(served.clone())).await;
    // `openpgpkey.direct-only.test` refuses the connection: the advanced host is not there.
    let found = mail_runtime::wkd::fetch(
        &client(addr),
        &at_port("joe@direct-only.test", addr.port()),
        "joe@direct-only.test",
    )
    .await;
    assert_eq!(found.map(|c| c.fingerprint()), Some(joe.fingerprint()));
}

#[tokio::test]
async fn a_directory_without_the_addresss_key_finds_nothing() {
    let other = key_for("other@wkd.test", 4);
    let served = other.public().to_bytes();
    let (addr, seen) = directory(move |_, _| Some(served.clone())).await;
    let found = mail_runtime::wkd::fetch(
        &client(addr),
        &at_port("joe@wkd.test", addr.port()),
        "joe@wkd.test",
    )
    .await;
    assert!(found.is_none(), "a key for someone else is not joe's");
    assert_eq!(seen.lock().unwrap().len(), 2, "both methods were asked");

    let (addr, _) = directory(|_, _| None).await;
    assert!(
        mail_runtime::wkd::fetch(
            &client(addr),
            &at_port("joe@wkd.test", addr.port()),
            "joe@wkd.test"
        )
        .await
        .is_none()
    );
}

#[tokio::test]
async fn a_plain_http_url_is_never_fetched() {
    let (addr, seen) = directory(|_, _| None).await;
    let urls = wkd::WkdUrls {
        advanced: format!("http://wkd.test:{}/key", addr.port()),
        direct: format!("http://wkd.test:{}/key", addr.port()),
        advanced_host: "wkd.test".to_owned(),
        direct_host: "wkd.test".to_owned(),
    };
    assert!(
        mail_runtime::wkd::fetch(&client(addr), &urls, "joe@wkd.test")
            .await
            .is_none()
    );
    assert!(
        seen.lock().unwrap().is_empty(),
        "nothing reached the listener"
    );
}

#[tokio::test]
async fn something_that_is_not_an_address_is_not_looked_up() {
    let http = mail_runtime::wkd::client().unwrap();
    assert!(
        mail_runtime::wkd::lookup(&http, "not an address")
            .await
            .is_err()
    );
}
