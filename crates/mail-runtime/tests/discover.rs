//! Account discovery against servers in this process.
//!
//! No network. Every HTTPS name is resolved by the client to a local `TcpListener` behind TLS
//! with a self-signed certificate from `tests/fixtures/discover-tls/` (for `example.test`,
//! `autoconfig.example.test`, `starttls.test`, `autoconfig.starttls.test` and `ispdb.test`,
//! valid until 2126), which the client is told to trust. DNS answers come from a table.

use mail_domain::{AuthPlan, Incoming, OAuthIssuer, Outgoing, Tls};
use mail_proto::discover::{MxRecord, Side, Source, SrvRecord, Unusable};
use mail_runtime::discover::{self, Dns, Miss, Sources};
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const CERT: &[u8] = include_bytes!("fixtures/discover-tls/cert.pem");
const KEY: &[u8] = include_bytes!("fixtures/discover-tls/key.pem");

/// Every name a test may fetch from. Each is pointed at the listener, so nothing reaches a real
/// resolver.
const NAMES: [&str; 5] = [
    "example.test",
    "autoconfig.example.test",
    "starttls.test",
    "autoconfig.starttls.test",
    "ispdb.test",
];

fn now() -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::parse_from_rfc3339("2026-09-24T12:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc)
}

/// Requests seen, as `host path?query`.
type Seen = Arc<Mutex<Vec<String>>>;

/// A TLS listener answering each request with `route(host, path)`: `Some(body)` is a `200`
/// with that body, `None` a `404`.
async fn server(
    route: impl Fn(&str, &str) -> Option<String> + Send + Sync + 'static,
) -> (SocketAddr, Seen) {
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(
        vec![CertificateDer::from_pem_slice(CERT).unwrap()],
        PrivateKeyDer::from_pem_slice(KEY).unwrap(),
    )
    .unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let seen: Seen = Arc::default();
    let recording = seen.clone();
    let route = Arc::new(route);
    tokio::spawn(async move {
        while let Ok((sock, _)) = listener.accept().await {
            let acceptor = acceptor.clone();
            let recording = recording.clone();
            let route = route.clone();
            tokio::spawn(async move {
                let Ok(mut stream) = acceptor.accept(sock).await else {
                    return;
                };
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                while !String::from_utf8_lossy(&buf).contains("\r\n\r\n") {
                    match stream.read(&mut chunk).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => buf.extend_from_slice(&chunk[..n]),
                    }
                }
                let text = String::from_utf8_lossy(&buf).to_string();
                let target = text.split(' ').nth(1).unwrap_or_default().to_owned();
                let host = text
                    .lines()
                    .find_map(|l| {
                        let (name, value) = l.split_once(':')?;
                        name.eq_ignore_ascii_case("host")
                            .then(|| value.trim().to_owned())
                    })
                    .unwrap_or_default();
                let host = host.split(':').next().unwrap_or_default().to_owned();
                recording.lock().unwrap().push(format!("{host} {target}"));
                let response = match route(&host, &target) {
                    Some(body) => format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    ),
                    None => {
                        "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                            .to_owned()
                    }
                };
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.shutdown().await;
            });
        }
    });
    (addr, seen)
}

/// The discovery client, trusting the fixture and resolving every test name to `addr`.
fn client(addr: SocketAddr) -> reqwest::Client {
    let mut builder = discover::client_builder()
        .add_root_certificate(reqwest::Certificate::from_pem(CERT).unwrap());
    for name in NAMES {
        builder = builder.resolve(name, addr);
    }
    builder.build().unwrap()
}

fn sources() -> Sources {
    Sources {
        ispdb: "https://ispdb.test/v1.1/".to_owned(),
    }
}

/// DNS from a table. A name not in it has no records.
#[derive(Default)]
struct Table {
    srv: HashMap<String, Vec<SrvRecord>>,
    mx: HashMap<String, Vec<MxRecord>>,
    /// Every query fails as if the network were down.
    down: bool,
    asked: Mutex<Vec<String>>,
}

impl Dns for Table {
    async fn srv(&self, name: &str) -> Result<Vec<SrvRecord>, Miss> {
        self.asked.lock().unwrap().push(format!("SRV {name}"));
        if self.down {
            return Err(Miss::Unreachable("network unreachable".to_owned()));
        }
        self.srv.get(name).cloned().ok_or(Miss::Absent)
    }

    async fn mx(&self, name: &str) -> Result<Vec<MxRecord>, Miss> {
        self.asked.lock().unwrap().push(format!("MX {name}"));
        if self.down {
            return Err(Miss::Unreachable("network unreachable".to_owned()));
        }
        self.mx.get(name).cloned().ok_or(Miss::Absent)
    }
}

fn document(incoming: &str, outgoing: &str) -> String {
    format!(
        r#"<?xml version="1.0"?>
<clientConfig version="1.1"><emailProvider id="x">
  <incomingServer type="imap"><hostname>{incoming}</hostname><port>993</port>
    <socketType>SSL</socketType><username>%EMAILADDRESS%</username>
    <authentication>password-cleartext</authentication></incomingServer>
  <outgoingServer type="smtp"><hostname>{outgoing}</hostname><port>465</port>
    <socketType>SSL</socketType><username>%EMAILADDRESS%</username>
    <authentication>password-cleartext</authentication></outgoingServer>
</emailProvider></clientConfig>"#
    )
}

const STARTTLS_ONLY: &str = r#"<clientConfig version="1.1"><emailProvider id="x">
  <incomingServer type="imap"><hostname>imap.starttls.test</hostname><port>143</port>
    <socketType>STARTTLS</socketType></incomingServer>
  <outgoingServer type="smtp"><hostname>smtp.starttls.test</hostname><port>587</port>
    <socketType>STARTTLS</socketType></outgoingServer>
</emailProvider></clientConfig>"#;

fn imap_host(found: &mail_proto::discover::Found) -> &str {
    match &found.preset.plan.incoming {
        Incoming::Imap { host, .. } | Incoming::Pop3 { host, .. } => host,
        Incoming::Local => "",
    }
}

#[tokio::test]
async fn the_domains_own_document_is_asked_first_and_carries_the_address() {
    let (addr, seen) = server(|host, _| {
        (host == "autoconfig.example.test")
            .then(|| document("imap.example.test", "smtp.example.test"))
    })
    .await;
    let dns = Table::default();
    let found = discover::discover("me@example.test", &client(addr), &dns, &sources(), now())
        .await
        .unwrap();
    assert_eq!(found.source, Source::DomainAutoconfig);
    assert_eq!(imap_host(&found), "imap.example.test");
    assert_eq!(
        found.preset.plan.outgoing,
        Outgoing::Smtp {
            host: "smtp.example.test".to_owned(),
            port: 465,
            tls: Tls::Implicit
        }
    );
    let seen = seen.lock().unwrap();
    assert_eq!(
        *seen,
        vec!["autoconfig.example.test /mail/config-v1.1.xml?emailaddress=me%40example.test"],
        "stopped at the first usable answer"
    );
    assert!(dns.asked.lock().unwrap().is_empty(), "DNS was not needed");
}

#[tokio::test]
async fn the_well_known_path_is_tried_next() {
    let (addr, _) = server(|host, path| {
        (host == "example.test" && path == "/.well-known/autoconfig/mail/config-v1.1.xml")
            .then(|| document("mail.example.test", "mail.example.test"))
    })
    .await;
    let found = discover::discover(
        "me@example.test",
        &client(addr),
        &Table::default(),
        &sources(),
        now(),
    )
    .await
    .unwrap();
    assert_eq!(found.source, Source::DomainAutoconfig);
    assert_eq!(imap_host(&found), "mail.example.test");
}

#[tokio::test]
async fn the_ispdb_is_asked_for_the_domain_and_never_sent_the_address() {
    let (addr, seen) = server(|host, path| {
        (host == "ispdb.test" && path == "/v1.1/example.test")
            .then(|| document("imap.provider.test", "smtp.provider.test"))
    })
    .await;
    let found = discover::discover(
        "me@example.test",
        &client(addr),
        &Table::default(),
        &sources(),
        now(),
    )
    .await
    .unwrap();
    assert_eq!(found.source, Source::Ispdb);
    assert_eq!(imap_host(&found), "imap.provider.test");
    let seen = seen.lock().unwrap();
    let ispdb = seen.iter().find(|s| s.starts_with("ispdb.test")).unwrap();
    assert!(
        !ispdb.contains("emailaddress") && !ispdb.contains("%40"),
        "{ispdb}"
    );
}

#[tokio::test]
async fn a_starttls_only_document_is_skipped_and_the_search_goes_on() {
    let (addr, seen) =
        server(|host, _| (host == "autoconfig.starttls.test").then(|| STARTTLS_ONLY.to_owned()))
            .await;
    let dns = Table::default();
    let err = discover::discover("me@starttls.test", &client(addr), &dns, &sources(), now())
        .await
        .unwrap_err();
    assert!(!err.offline());
    assert!(
        err.unusable().any(|why| matches!(
            why,
            Unusable::NoImplicitTls {
                side: Side::Incoming,
                ..
            }
        )),
        "{err}"
    );
    assert!(err.to_string().contains("STARTTLS"), "{err}");
    // Every later source was still asked.
    assert!(
        seen.lock()
            .unwrap()
            .iter()
            .any(|s| s.starts_with("ispdb.test"))
    );
    assert!(
        dns.asked
            .lock()
            .unwrap()
            .iter()
            .any(|q| q.starts_with("MX"))
    );
}

#[tokio::test]
async fn srv_records_serve_when_no_document_exists() {
    let (addr, _) = server(|_, _| None).await;
    let mut dns = Table::default();
    dns.srv.insert(
        "_imaps._tcp.example.test".to_owned(),
        vec![SrvRecord {
            priority: 0,
            weight: 1,
            port: 993,
            target: "imap.example.test.".to_owned(),
        }],
    );
    dns.srv.insert(
        "_submissions._tcp.example.test".to_owned(),
        vec![SrvRecord {
            priority: 0,
            weight: 1,
            port: 465,
            target: "smtp.example.test.".to_owned(),
        }],
    );
    let found = discover::discover("me@example.test", &client(addr), &dns, &sources(), now())
        .await
        .unwrap();
    assert_eq!(found.source, Source::Srv);
    assert_eq!(imap_host(&found), "imap.example.test");
}

#[tokio::test]
async fn an_mx_at_google_is_the_gmail_preset_without_asking_the_ispdb_about_google() {
    let (addr, seen) = server(|_, _| None).await;
    let mut dns = Table::default();
    dns.mx.insert(
        "example.test".to_owned(),
        vec![
            MxRecord {
                preference: 5,
                exchange: "alt1.aspmx.l.google.com.".to_owned(),
            },
            MxRecord {
                preference: 1,
                exchange: "aspmx.l.google.com.".to_owned(),
            },
        ],
    );
    let found = discover::discover("me@example.test", &client(addr), &dns, &sources(), now())
        .await
        .unwrap();
    assert_eq!(
        found.source,
        Source::Mx {
            exchanger: "aspmx.l.google.com".to_owned(),
            provider: "google.com".to_owned()
        }
    );
    assert!(matches!(
        found.preset.plan.auth,
        AuthPlan::OAuth {
            issuer: OAuthIssuer::Google,
            ..
        }
    ));
    assert_eq!(found.preset.plan.address, "me@example.test");
    assert!(!seen.lock().unwrap().iter().any(|s| s.contains("google")));
}

#[tokio::test]
async fn an_mx_at_an_unknown_host_leads_to_that_hosts_ispdb_document() {
    let (addr, _) = server(|host, path| {
        (host == "ispdb.test" && path == "/v1.1/hosting.test")
            .then(|| document("imap.hosting.test", "smtp.hosting.test"))
    })
    .await;
    let mut dns = Table::default();
    dns.mx.insert(
        "example.test".to_owned(),
        vec![MxRecord {
            preference: 10,
            exchange: "mx3.hosting.test.".to_owned(),
        }],
    );
    let found = discover::discover("me@example.test", &client(addr), &dns, &sources(), now())
        .await
        .unwrap();
    assert_eq!(
        found.source,
        Source::Mx {
            exchanger: "mx3.hosting.test".to_owned(),
            provider: "hosting.test".to_owned()
        }
    );
    assert_eq!(imap_host(&found), "imap.hosting.test");
}

#[tokio::test]
async fn a_certificate_nobody_vouches_for_is_not_trusted() {
    let (addr, seen) =
        server(|_, _| Some(document("imap.example.test", "smtp.example.test"))).await;
    // The production client, with the names pointed here but the fixture not trusted.
    let mut builder = discover::client_builder();
    for name in NAMES {
        builder = builder.resolve(name, addr);
    }
    let err = discover::discover(
        "me@example.test",
        &builder.build().unwrap(),
        &Table::default(),
        &sources(),
        now(),
    )
    .await
    .unwrap_err();
    assert!(
        err.tried
            .iter()
            .filter(|t| t.what.starts_with("https://"))
            .all(|t| matches!(t.miss, Miss::Unreachable(_))),
        "{err}"
    );
    assert!(seen.lock().unwrap().is_empty(), "no request completed");
}

#[tokio::test]
async fn a_redirect_to_plain_http_is_not_followed() {
    let plain = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let plain_addr = plain.local_addr().unwrap();
    let connected = Arc::new(Mutex::new(0));
    let counting = connected.clone();
    tokio::spawn(async move {
        while let Ok((_sock, _)) = plain.accept().await {
            *counting.lock().unwrap() += 1;
        }
    });
    let (addr, _) = server(|_, _| None).await;
    // A server that redirects every request to http://.
    let redirecting = {
        let config = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![CertificateDer::from_pem_slice(CERT).unwrap()],
            PrivateKeyDer::from_pem_slice(KEY).unwrap(),
        )
        .unwrap();
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let at = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((sock, _)) = listener.accept().await {
                let Ok(mut stream) = acceptor.accept(sock).await else {
                    continue;
                };
                let mut chunk = [0u8; 4096];
                let _ = stream.read(&mut chunk).await;
                let response = format!(
                    "HTTP/1.1 302 Found\r\nLocation: http://example.test:{}/x.xml\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    plain_addr.port()
                );
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.shutdown().await;
            }
        });
        at
    };
    let mut builder = discover::client_builder()
        .add_root_certificate(reqwest::Certificate::from_pem(CERT).unwrap())
        .resolve("autoconfig.example.test", redirecting);
    for name in NAMES.iter().filter(|n| **n != "autoconfig.example.test") {
        builder = builder.resolve(name, addr);
    }
    let err = discover::discover(
        "me@example.test",
        &builder.build().unwrap(),
        &Table::default(),
        &sources(),
        now(),
    )
    .await
    .unwrap_err();
    assert!(
        err.tried[0]
            .what
            .starts_with("https://autoconfig.example.test")
    );
    assert!(!matches!(err.tried[0].miss, Miss::Absent), "{err}");
    assert_eq!(
        *connected.lock().unwrap(),
        0,
        "nothing went out in the clear"
    );
}

#[tokio::test]
async fn nothing_answering_reads_as_offline() {
    // A port that was open a moment ago and is not now.
    let closed = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap()
    };
    let dns = Table {
        down: true,
        ..Table::default()
    };
    let err = discover::discover("me@example.test", &client(closed), &dns, &sources(), now())
        .await
        .unwrap_err();
    assert!(err.offline(), "{err}");
    assert!(err.to_string().contains("offline"), "{err}");
}
