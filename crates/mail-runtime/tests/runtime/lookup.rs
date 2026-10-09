//! `ReqwestHttp`, the `porter_http::Http` seam account discovery fetches through, against
//! servers in this process.
//!
//! What an answer means, and the order the sources are asked in, are `porter-discover`'s and are
//! tested there. What belongs to mailo is the client under the seam: HTTPS only, certificates
//! verified, no redirect to plain HTTP, a cap on the body, and an error that says which kind of
//! failure it was.
//!
//! No network. The name `example.test` is resolved by the client to a local `TcpListener` behind
//! TLS with a self-signed certificate from `tests/fixtures/discover-tls/` (for `example.test`,
//! valid until 2126), which the client is told to trust.

use mail_runtime::lookup::{DOCUMENT_LIMIT, ReqwestHttp, client_builder};
use porter_core::WebUrl;
use porter_http::{Http, HttpError, HttpRequest, Method};
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const CERT: &[u8] = include_bytes!("../fixtures/discover-tls/cert.pem");
const KEY: &[u8] = include_bytes!("../fixtures/discover-tls/key.pem");

fn acceptor() -> tokio_rustls::TlsAcceptor {
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
    tokio_rustls::TlsAcceptor::from(Arc::new(config))
}

/// Requests seen, as the request target.
type Seen = Arc<Mutex<Vec<String>>>;

/// A TLS listener answering every request with `response(target)`, verbatim.
async fn server(response: impl Fn(&str) -> String + Send + Sync + 'static) -> (SocketAddr, Seen) {
    let acceptor = acceptor();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let seen: Seen = Arc::default();
    let recording = seen.clone();
    let response = Arc::new(response);
    tokio::spawn(async move {
        while let Ok((sock, _)) = listener.accept().await {
            let (acceptor, recording, response) =
                (acceptor.clone(), recording.clone(), response.clone());
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
                recording.lock().unwrap().push(target.clone());
                let _ = stream.write_all(response(&target).as_bytes()).await;
                let _ = stream.shutdown().await;
            });
        }
    });
    (addr, seen)
}

fn ok(body: &str) -> String {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

/// The production client with `example.test` pointed at `addr`, trusting the fixture.
fn trusting(addr: SocketAddr) -> ReqwestHttp {
    ReqwestHttp::over(
        client_builder()
            .add_root_certificate(reqwest::Certificate::from_pem(CERT).unwrap())
            .resolve("example.test", addr)
            .build()
            .unwrap(),
    )
}

fn get(url: &str) -> HttpRequest {
    HttpRequest::new(Method::Get, WebUrl::parse(url).unwrap())
}

#[tokio::test]
async fn a_document_comes_back_with_its_status_headers_and_body() {
    let (addr, seen) = server(|target| match target.starts_with("/missing") {
        true => "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),
        false => ok("<clientConfig/>"),
    })
    .await;
    let http = trusting(addr);
    let found = http
        .send(get(
            "https://example.test/mail/config-v1.1.xml?emailaddress=me%40example.test",
        ))
        .await
        .unwrap();
    assert!(found.status.is_success());
    assert_eq!(found.body, b"<clientConfig/>");
    assert_eq!(found.header("content-type"), Some("application/xml"));
    // The query reaches the server whole.
    assert_eq!(
        seen.lock().unwrap().as_slice(),
        ["/mail/config-v1.1.xml?emailaddress=me%40example.test"]
    );
    // An error status is a response, for the caller to read, and not a failure.
    let missing = http
        .send(get("https://example.test/missing"))
        .await
        .unwrap();
    assert_eq!(missing.status.0, 404);
}

#[tokio::test]
async fn a_certificate_nobody_vouches_for_is_not_trusted() {
    let (addr, seen) = server(|_| ok("<clientConfig/>")).await;
    // The production client, with the name pointed here but the fixture not trusted.
    let http = ReqwestHttp::over(
        client_builder()
            .resolve("example.test", addr)
            .build()
            .unwrap(),
    );
    let err = http.send(get("https://example.test/")).await.unwrap_err();
    assert_eq!(err, HttpError::Tls);
    assert!(seen.lock().unwrap().is_empty(), "no request completed");
}

#[tokio::test]
async fn plain_http_is_never_used() {
    let plain = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = plain.local_addr().unwrap().port();
    let connected = Arc::new(Mutex::new(0));
    let counting = connected.clone();
    tokio::spawn(async move {
        while let Ok((_sock, _)) = plain.accept().await {
            *counting.lock().unwrap() += 1;
        }
    });
    let http = ReqwestHttp::new().unwrap();
    let err = http
        .send(get(&format!("http://127.0.0.1:{port}/")))
        .await
        .unwrap_err();
    assert_eq!(err, HttpError::Unreachable);
    assert_eq!(*connected.lock().unwrap(), 0, "nothing went out");
}

#[tokio::test]
async fn a_redirect_to_plain_http_is_not_followed() {
    let plain = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let plain_port = plain.local_addr().unwrap().port();
    let connected = Arc::new(Mutex::new(0));
    let counting = connected.clone();
    tokio::spawn(async move {
        while let Ok((_sock, _)) = plain.accept().await {
            *counting.lock().unwrap() += 1;
        }
    });
    let (addr, _) = server(move |_| {
        format!(
            "HTTP/1.1 302 Found\r\nLocation: http://example.test:{plain_port}/x.xml\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        )
    })
    .await;
    let response = trusting(addr)
        .send(get("https://example.test/mail/config-v1.1.xml"))
        .await
        .unwrap();
    // The redirect is the answer, and a caller does not take that for a document.
    assert_eq!(response.status.0, 302);
    assert_eq!(
        *connected.lock().unwrap(),
        0,
        "nothing went out in the clear"
    );
}

#[tokio::test]
async fn a_document_over_the_limit_is_too_large() {
    let (addr, _) = server(|target| match target {
        "/just-fits" => ok(&"a".repeat(DOCUMENT_LIMIT)),
        _ => ok(&"a".repeat(DOCUMENT_LIMIT + 1)),
    })
    .await;
    let http = trusting(addr);
    let fits = http
        .send(get("https://example.test/just-fits"))
        .await
        .unwrap();
    assert_eq!(fits.body.len(), DOCUMENT_LIMIT);
    let err = http
        .send(get("https://example.test/over"))
        .await
        .unwrap_err();
    assert_eq!(err, HttpError::TooLarge);
}

#[tokio::test]
async fn nothing_answering_is_unreachable() {
    // A port that was open a moment ago and is not now.
    let closed = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap()
    };
    let err = trusting(closed)
        .send(get("https://example.test/"))
        .await
        .unwrap_err();
    assert_eq!(err, HttpError::Unreachable);
}
