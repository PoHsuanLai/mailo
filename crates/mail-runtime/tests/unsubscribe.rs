//! One-click unsubscribe against a list server in this process.
//!
//! No network. The "list server" is a `TcpListener` behind TLS with a self-signed certificate
//! from `tests/fixtures/tls/` (for `unsubscribe.test` and `plain.test`, valid until 2126), and
//! the client is told to trust that certificate and to resolve those names to the listener.
//! Everything else about the client is what `mailo unsubscribe` uses.

use mail_domain::{Retry, Retryable};
use mail_mime::HttpsUrl;
use mail_runtime::unsubscribe::{self, UnsubscribeFailure};
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpListener;

const CERT: &[u8] = include_bytes!("fixtures/tls/cert.pem");
const KEY: &[u8] = include_bytes!("fixtures/tls/key.pem");

const OK: &str = "HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";

/// Every request a listener received, as text: request line, headers, body.
type Seen = Arc<Mutex<Vec<String>>>;

/// A TLS listener. `respond(port, n)` is the answer to the `n`th request, counting from zero.
async fn tls_server(
    respond: impl Fn(u16, usize) -> String + Send + Sync + 'static,
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
        // One connection at a time, in order, so `n` is the request's position.
        let mut n = 0;
        while let Ok((sock, _)) = listener.accept().await {
            let Ok(stream) = acceptor.accept(sock).await else {
                continue;
            };
            answer(stream, &recording, &respond(addr.port(), n)).await;
            n += 1;
        }
    });
    (addr, seen)
}

/// Always the same answer.
fn always(response: &'static str) -> impl Fn(u16, usize) -> String + Send + Sync + 'static {
    move |_, _| response.to_owned()
}

/// A redirect with `status` to `location`.
fn redirect(status: &str, location: &str) -> String {
    format!(
        "HTTP/1.1 {status}\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    )
}

/// A plain-TCP listener that records only that someone connected.
async fn plain_server() -> (SocketAddr, Arc<Mutex<usize>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let connections = Arc::new(Mutex::new(0));
    let counting = connections.clone();
    tokio::spawn(async move {
        while let Ok((sock, _)) = listener.accept().await {
            *counting.lock().unwrap() += 1;
            drop(sock);
        }
    });
    (addr, connections)
}

/// Read one request, headers and `Content-Length` body, record it, and answer.
async fn answer<S: AsyncRead + AsyncWrite + Unpin>(mut stream: S, seen: &Seen, response: &str) {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let Ok(read) = stream.read(&mut chunk).await else {
            return;
        };
        if read == 0 {
            return;
        }
        buf.extend_from_slice(&chunk[..read]);
        let text = String::from_utf8_lossy(&buf).to_string();
        let Some(end) = text.find("\r\n\r\n") else {
            continue;
        };
        let length = text[..end]
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            })
            .unwrap_or(0);
        if buf.len() >= end + 4 + length {
            seen.lock().unwrap().push(text);
            break;
        }
    }
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.flush().await;
    let _ = stream.shutdown().await;
}

/// The production client, trusting the fixture and resolving the test names locally.
fn client(tls: SocketAddr, plain: Option<SocketAddr>) -> reqwest::Client {
    let mut builder = unsubscribe::client_builder()
        .add_root_certificate(reqwest::Certificate::from_pem(CERT).unwrap())
        .resolve("unsubscribe.test", tls);
    if let Some(plain) = plain {
        builder = builder.resolve("plain.test", plain);
    }
    builder.build().unwrap()
}

fn url(addr: SocketAddr, path: &str) -> HttpsUrl {
    HttpsUrl::parse(&format!("https://unsubscribe.test:{}{path}", addr.port())).unwrap()
}

#[tokio::test]
async fn one_click_posts_the_rfc_8058_body_and_nothing_else() {
    let (addr, seen) = tls_server(always(OK)).await;
    unsubscribe::one_click(&client(addr, None), &url(addr, "/u/abc?list=7"))
        .await
        .expect("a 200 is done");

    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 1, "exactly one request");
    let (head, body) = seen[0].split_once("\r\n\r\n").unwrap();
    assert!(
        head.starts_with("POST /u/abc?list=7 HTTP/1.1\r\n"),
        "{head}"
    );
    let header = |name: &str| {
        head.lines().skip(1).find_map(|line| {
            let (n, v) = line.split_once(':')?;
            n.eq_ignore_ascii_case(name).then(|| v.trim().to_owned())
        })
    };
    assert_eq!(
        header("content-type").as_deref(),
        Some("application/x-www-form-urlencoded")
    );
    assert_eq!(body, "List-Unsubscribe=One-Click");
    assert_eq!(header("content-length").as_deref(), Some("26"));
    // RFC 8058 §3.1: no cookies and no credentials on the request.
    assert_eq!(header("cookie"), None, "{head}");
    assert_eq!(header("authorization"), None, "{head}");
}

#[tokio::test]
async fn a_refusal_is_named_with_its_status() {
    for (response, status, retry_later) in [
        (
            "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            404,
            false,
        ),
        (
            "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            503,
            true,
        ),
    ] {
        let (addr, _) = tls_server(always(response)).await;
        let err = unsubscribe::one_click(&client(addr, None), &url(addr, "/u"))
            .await
            .expect_err("not 2xx");
        assert_eq!(err, UnsubscribeFailure::Refused { status });
        assert_eq!(
            matches!(err.retry(), Retry::After(_)),
            retry_later,
            "case {status}"
        );
    }
}

#[tokio::test]
async fn a_redirect_to_plain_http_is_refused_and_nothing_reaches_it() {
    let (plain, connections) = plain_server().await;
    let to = format!("http://plain.test:{}/u", plain.port());
    let (addr, _) = tls_server(move |_, _| redirect("307 Temporary Redirect", &to)).await;
    let err = unsubscribe::one_click(&client(addr, Some(plain)), &url(addr, "/u"))
        .await
        .expect_err("an http redirect is refused");
    assert!(
        matches!(err, UnsubscribeFailure::InsecureRedirect { ref to } if to.starts_with("http://plain.test")),
        "{err:?}"
    );
    assert_eq!(
        *connections.lock().unwrap(),
        0,
        "the http target was contacted"
    );
}

#[tokio::test]
async fn the_client_never_posts_to_plain_http_even_when_asked_directly() {
    // `one_click` takes an `HttpsUrl`, which an `http:` URL cannot become. This is the layer
    // under it: the client itself refuses, so a caller that skipped the type still sends nothing.
    assert_eq!(HttpsUrl::parse("http://plain.test/u"), None);
    let (plain, connections) = plain_server().await;
    let (tls, _) = tls_server(always(OK)).await;
    let sent = client(tls, Some(plain))
        .post(format!("http://plain.test:{}/u", plain.port()))
        .body(mail_mime::ONE_CLICK)
        .send()
        .await;
    assert!(sent.is_err(), "the https-only client sent over http");
    assert_eq!(*connections.lock().unwrap(), 0);
}

#[tokio::test]
async fn a_redirect_that_would_drop_the_post_is_reported_not_followed() {
    let (addr, seen) = tls_server(|port, _| {
        redirect(
            "303 See Other",
            &format!("https://unsubscribe.test:{port}/done"),
        )
    })
    .await;
    let err = unsubscribe::one_click(&client(addr, None), &url(addr, "/u"))
        .await
        .expect_err("a 303 is not an unsubscribe");
    assert_eq!(err, UnsubscribeFailure::Refused { status: 303 });
    assert_eq!(seen.lock().unwrap().len(), 1, "the redirect was followed");
}

#[tokio::test]
async fn a_307_to_https_is_followed_with_the_same_post() {
    let (addr, seen) = tls_server(|port, n| match n {
        0 => redirect(
            "307 Temporary Redirect",
            &format!("https://unsubscribe.test:{port}/done"),
        ),
        _ => OK.to_owned(),
    })
    .await;
    unsubscribe::one_click(&client(addr, None), &url(addr, "/u"))
        .await
        .expect("followed to a 200");
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    assert!(
        seen[1].starts_with("POST /done HTTP/1.1\r\n"),
        "{}",
        seen[1]
    );
    assert!(
        seen[1].ends_with("\r\n\r\nList-Unsubscribe=One-Click"),
        "{}",
        seen[1]
    );
}

#[tokio::test]
async fn a_server_that_does_not_answer_is_unreachable() {
    // A port nothing listens on: bind one, learn its number, and close it.
    let closed = TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap()
        .local_addr()
        .unwrap();
    let err = unsubscribe::one_click(&client(closed, None), &url(closed, "/u"))
        .await
        .expect_err("nothing is listening");
    assert!(matches!(err, UnsubscribeFailure::Unreachable(_)), "{err:?}");
    assert!(matches!(err.retry(), Retry::After(_)));
}
