//! The HTTPS servers the tests here stand up, all on one certificate.
//!
//! `tests/fixtures/tls/` holds a self-signed certificate, valid until 2126, for every name a test
//! points at a local listener: `example.test`, `wkd.test`, `openpgpkey.wkd.test`,
//! `direct-only.test`, `openpgpkey.direct-only.test`, `unsubscribe.test`, `plain.test` and
//! `dav.test`. A client trusts it through [`root`] and resolves those names to the listener.

use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::server::TlsStream;

const CERT: &[u8] = include_bytes!("../fixtures/tls/cert.pem");
const KEY: &[u8] = include_bytes!("../fixtures/tls/key.pem");

/// The certificate, for a client to trust.
pub fn root() -> reqwest::Certificate {
    reqwest::Certificate::from_pem(CERT).unwrap()
}

/// A TLS listener on loopback that hands each connection, once through the handshake, to `each`.
pub async fn serve_tls<F, Fut>(each: F) -> SocketAddr
where
    F: Fn(TlsStream<TcpStream>) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
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
    let each = Arc::new(each);
    tokio::spawn(async move {
        while let Ok((sock, _)) = listener.accept().await {
            let (acceptor, each) = (acceptor.clone(), each.clone());
            tokio::spawn(async move {
                // A client that refuses the certificate never gets as far as a request.
                if let Ok(stream) = acceptor.accept(sock).await {
                    each(stream).await;
                }
            });
        }
    });
    addr
}

/// One request as text, its head and as much body as `Content-Length` says, or `None` if the
/// client went away first.
pub async fn read_request<S: AsyncRead + Unpin>(stream: &mut S) -> Option<String> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let read = stream.read(&mut chunk).await.ok()?;
        if read == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..read]);
        let text = String::from_utf8_lossy(&buf).to_string();
        let Some(end) = text.find("\r\n\r\n") else {
            continue;
        };
        let length: usize = header(&text[..end], "content-length")
            .and_then(|l| l.parse().ok())
            .unwrap_or(0);
        if buf.len() >= end + 4 + length {
            return Some(text);
        }
    }
}

/// The value of header `name` in a request's head, the first if there are several.
pub fn header(head: &str, name: &str) -> Option<String> {
    head.lines().skip(1).find_map(|line| {
        let (n, v) = line.split_once(':')?;
        n.eq_ignore_ascii_case(name).then(|| v.trim().to_owned())
    })
}

/// The answer to a request for something there isn't.
pub const NOT_FOUND: &[u8] =
    b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";

/// A `200` carrying `body`, with a `Content-Type` if there is one to give.
pub fn ok(content_type: Option<&str>, body: &[u8]) -> Vec<u8> {
    let content_type = content_type.map_or(String::new(), |t| format!("Content-Type: {t}\r\n"));
    let mut out = format!(
        "HTTP/1.1 200 OK\r\n{content_type}Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    out.extend_from_slice(body);
    out
}

/// Write `response` and close.
pub async fn reply<S: AsyncWrite + Unpin>(stream: &mut S, response: &[u8]) {
    // The client may already have hung up; what it saw is the test's to assert.
    if stream.write_all(response).await.is_ok() && stream.flush().await.is_ok() {
        // Closing ends the answer, and a client that left first has had all of it.
        let _ = stream.shutdown().await;
    }
}

/// A plain-TCP listener that counts who connects and hangs up on each without a word: no
/// handshake, no answer. It stays bound while the test runs, so the port is nobody else's.
pub async fn hang_up() -> (SocketAddr, Arc<Mutex<usize>>) {
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
