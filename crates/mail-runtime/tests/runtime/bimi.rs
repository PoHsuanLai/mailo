//! Brand logo lookup against DNS answered from a table and a web server in this process.
//!
//! No network. DNS is a [`Table`] that counts what it is asked. The web server is a
//! `TcpListener` behind TLS with the self-signed certificate `tests/runtime/wkd.rs` uses (it names
//! `wkd.test`), and the client is told to trust it and to resolve `wkd.test` to the listener. The
//! records point at `https://wkd.test:<port>/…`. The mark certificates come from a throwaway
//! authority (`mail-mime/tests/bimi_support`).

use crate::bimi_support;

use bimi_support::{LOGO, Mark, authority, now, pem};
use mail_runtime::bimi::{Cached, Lookup, NoLogo, Txt, cached, find, logo};
use mail_runtime::lookup::Miss;
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const CERT: &[u8] = include_bytes!("../fixtures/wkd-tls/cert.pem");
const KEY: &[u8] = include_bytes!("../fixtures/wkd-tls/key.pem");

/// TXT answers by name, and every name asked, in order.
#[derive(Default)]
struct Table {
    records: HashMap<String, Vec<String>>,
    asked: Mutex<Vec<String>>,
}

impl Table {
    fn with(mut self, name: &str, txt: &str) -> Self {
        self.records
            .entry(name.to_owned())
            .or_default()
            .push(txt.to_owned());
        self
    }

    fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }
}

impl Txt for Table {
    async fn txt(&self, name: &str) -> Result<Vec<String>, Miss> {
        self.asked.lock().unwrap().push(name.to_owned());
        self.records.get(name).cloned().ok_or(Miss::Absent)
    }
}

/// The paths the server was asked for, in order.
type Seen = Arc<Mutex<Vec<String>>>;

/// A TLS server answering `path` from `files`, else 404.
async fn server(files: HashMap<String, Vec<u8>>) -> (SocketAddr, Seen) {
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
            recording.lock().unwrap().push(path.clone());
            let response = match files.get(&path) {
                Some(body) => {
                    let mut out = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    )
                    .into_bytes();
                    out.extend_from_slice(body);
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

fn client(to: SocketAddr) -> reqwest::Client {
    mail_runtime::bimi::client_builder()
        .add_root_certificate(reqwest::Certificate::from_pem(CERT).unwrap())
        .resolve("wkd.test", to)
        .build()
        .unwrap()
}

/// A domain with a BIMI record naming both files on the server, a `reject` DMARC policy, and a
/// good mark certificate for `wkd.test` from `trusted`.
struct Brand {
    addr: SocketAddr,
    seen: Seen,
    dns: Table,
    anchors: Vec<mail_mime::smime::Cert>,
}

async fn brand(record: impl Fn(u16) -> String, dmarc: &str, served_logo: &str) -> Brand {
    let trusted = authority("Trusted", 10);
    let vmc = pem(&trusted, &Mark::new(&["wkd.test"], LOGO.as_bytes()));
    let files = HashMap::from([
        ("/logo.svg".to_owned(), served_logo.as_bytes().to_vec()),
        ("/vmc.pem".to_owned(), vmc),
    ]);
    let (addr, seen) = server(files).await;
    let dns = Table::default()
        .with("default._bimi.wkd.test", &record(addr.port()))
        .with("_dmarc.wkd.test", dmarc);
    Brand {
        addr,
        seen,
        dns,
        anchors: vec![trusted.root.clone()],
    }
}

fn full_record(port: u16) -> String {
    format!("v=BIMI1; l=https://wkd.test:{port}/logo.svg; a=https://wkd.test:{port}/vmc.pem")
}

#[tokio::test]
async fn a_certified_logo_is_fetched_drawn_and_cached() {
    let brand = brand(full_record, "v=DMARC1; p=reject", LOGO).await;
    let http = client(brand.addr);
    let cache = tempfile::tempdir().unwrap();
    let lookup = Lookup {
        dns: &brand.dns,
        http: &http,
        anchors: &brand.anchors,
        now: now(),
    };
    let png = logo(&lookup, cache.path(), "wkd.test")
        .await
        .expect("a logo");
    assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert_eq!(
        brand.dns.asked(),
        vec!["default._bimi.wkd.test", "_dmarc.wkd.test"]
    );
    assert_eq!(
        *brand.seen.lock().unwrap(),
        vec!["/vmc.pem", "/logo.svg"],
        "the certificate first, then the logo"
    );
    assert_eq!(
        cached(cache.path(), "wkd.test", now()),
        Cached::Logo(png.clone())
    );

    // Asked again: the cache answers, and nothing is looked up or fetched.
    let again = logo(&lookup, cache.path(), "wkd.test").await;
    assert_eq!(again, Some(png));
    assert_eq!(brand.dns.asked().len(), 2, "no second lookup");
    assert_eq!(brand.seen.lock().unwrap().len(), 2, "no second fetch");
}

#[tokio::test]
async fn a_record_without_evidence_shows_no_logo_and_fetches_nothing() {
    let brand = brand(
        |port| format!("v=BIMI1; l=https://wkd.test:{port}/logo.svg"),
        "v=DMARC1; p=reject",
        LOGO,
    )
    .await;
    let http = client(brand.addr);
    let cache = tempfile::tempdir().unwrap();
    let lookup = Lookup {
        dns: &brand.dns,
        http: &http,
        anchors: &brand.anchors,
        now: now(),
    };
    assert_eq!(find(&lookup, "wkd.test").await, Err(NoLogo::NoEvidence));
    assert_eq!(logo(&lookup, cache.path(), "wkd.test").await, None);
    assert!(
        brand.seen.lock().unwrap().is_empty(),
        "the self-asserted SVG was fetched: {:?}",
        brand.seen.lock().unwrap()
    );
    assert_eq!(
        cached(cache.path(), "wkd.test", now()),
        Cached::None,
        "remembered as none"
    );
}

#[tokio::test]
async fn what_rules_a_logo_out() {
    let stranger = authority("Stranger", 20);
    let cases: Vec<(&str, Brand, NoLogo, usize)> = vec![
        (
            "a policy that is not at enforcement",
            brand(full_record, "v=DMARC1; p=none", LOGO).await,
            NoLogo::NotEnforced,
            0,
        ),
        (
            "quarantine for half the mail",
            brand(full_record, "v=DMARC1; p=quarantine; pct=50", LOGO).await,
            NoLogo::NotEnforced,
            0,
        ),
        (
            "a logo that is not the certified one",
            brand(
                full_record,
                "v=DMARC1; p=reject",
                &LOGO.replace("#1a73e8", "#e81a1a"),
            )
            .await,
            NoLogo::NotTheCertified,
            2,
        ),
        (
            "a declined record",
            brand(
                |_| "v=BIMI1; l=; a=;".to_owned(),
                "v=DMARC1; p=reject",
                LOGO,
            )
            .await,
            NoLogo::NoEvidence,
            0,
        ),
        (
            "an authority nobody trusts",
            Brand {
                anchors: vec![stranger.root.clone()],
                ..brand(full_record, "v=DMARC1; p=reject", LOGO).await
            },
            NoLogo::Evidence(mail_mime::bimi::MarkProblem::Untrusted),
            1,
        ),
    ];
    for (name, brand, want, fetches) in cases {
        let http = client(brand.addr);
        let lookup = Lookup {
            dns: &brand.dns,
            http: &http,
            anchors: &brand.anchors,
            now: now(),
        };
        assert_eq!(find(&lookup, "wkd.test").await, Err(want), "case: {name}");
        assert_eq!(
            brand.seen.lock().unwrap().len(),
            fetches,
            "case: {name}: {:?}",
            brand.seen.lock().unwrap()
        );
    }
}

#[tokio::test]
async fn a_subdomain_falls_back_to_the_organisational_domains_record() {
    let brand = brand(full_record, "v=DMARC1; p=reject", LOGO).await;
    let http = client(brand.addr);
    let lookup = Lookup {
        dns: &brand.dns,
        http: &http,
        anchors: &brand.anchors,
        now: now(),
    };
    let found = find(&lookup, "mail.wkd.test").await;
    assert_eq!(found, Ok(LOGO.as_bytes().to_vec()));
    assert_eq!(
        brand.dns.asked(),
        vec![
            "default._bimi.mail.wkd.test",
            "default._bimi.wkd.test",
            "_dmarc.mail.wkd.test",
            "_dmarc.wkd.test",
        ]
    );
}

#[tokio::test]
async fn no_record_is_remembered_and_an_unreachable_resolver_is_not() {
    struct Down;
    impl Txt for Down {
        async fn txt(&self, _: &str) -> Result<Vec<String>, Miss> {
            Err(Miss::Unreachable("no resolver".to_owned()))
        }
    }
    let http = client(SocketAddr::from(([127, 0, 0, 1], 9)));
    let cache = tempfile::tempdir().unwrap();
    let down = Lookup {
        dns: &Down,
        http: &http,
        anchors: &[],
        now: now(),
    };
    assert_eq!(logo(&down, cache.path(), "brand.example").await, None);
    assert_eq!(
        cached(cache.path(), "brand.example", now()),
        Cached::Unknown,
        "a failure to ask is asked again"
    );

    let empty = Table::default();
    let nothing = Lookup {
        dns: &empty,
        http: &http,
        anchors: &[],
        now: now(),
    };
    assert_eq!(logo(&nothing, cache.path(), "brand.example").await, None);
    assert_eq!(cached(cache.path(), "brand.example", now()), Cached::None);
    assert_eq!(empty.asked(), vec!["default._bimi.brand.example"]);
}
