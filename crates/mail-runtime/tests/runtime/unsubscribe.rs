//! One-click unsubscribe against a list server in this process.
//!
//! No network. The "list server" is a `TcpListener` behind TLS with the certificate
//! `mail_net::testing::tls` serves, and the client is told to trust that certificate and to resolve those names to the listener.
//! Everything else about the client is what `mailo unsubscribe` uses.

use mail_domain::{Retry, Retryable};
use mail_mime::HttpsUrl;
use mail_net::testing::tls::{hang_up, header, read_request, reply, root, serve_tls};
use mail_runtime::unsubscribe::{self, UnsubscribeFailure};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const OK: &str = "HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";

/// Every request a listener received, as text: request line, headers, body.
type Seen = Arc<Mutex<Vec<String>>>;

/// A TLS listener. `respond(host, n)` is the answer to the `n`th request, counting from zero,
/// where `host` is the request's `Host`, port and all.
async fn tls_server(
    respond: impl Fn(&str, usize) -> String + Send + Sync + 'static,
) -> (SocketAddr, Seen) {
    let seen: Seen = Arc::default();
    let recording = seen.clone();
    let respond = Arc::new(respond);
    let count = Arc::new(AtomicUsize::new(0));
    let addr = serve_tls(move |mut stream| {
        let (recording, respond, count) = (recording.clone(), respond.clone(), count.clone());
        async move {
            let Some(text) = read_request(&mut stream).await else {
                return;
            };
            let host = header(&text, "host").unwrap_or_default();
            recording.lock().unwrap().push(text);
            // The client sends one request at a time, so arrival order is the request's position.
            let answer = respond(&host, count.fetch_add(1, Ordering::SeqCst));
            reply(&mut stream, answer.as_bytes()).await;
        }
    })
    .await;
    (addr, seen)
}

/// Always the same answer.
fn always(response: &'static str) -> impl Fn(&str, usize) -> String + Send + Sync + 'static {
    move |_, _| response.to_owned()
}

/// A redirect with `status` to `location`.
fn redirect(status: &str, location: &str) -> String {
    format!(
        "HTTP/1.1 {status}\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    )
}

/// The production client, trusting the fixture and resolving the test names locally.
fn client(tls: SocketAddr, plain: Option<SocketAddr>) -> reqwest::Client {
    let mut builder = unsubscribe::client_builder()
        .add_root_certificate(root())
        .resolve("unsubscribe.test", tls);
    if let Some(plain) = plain {
        builder = builder.resolve("plain.test", plain);
    }
    builder.build().unwrap()
}

fn url(addr: SocketAddr, path: &str) -> HttpsUrl {
    format!("https://unsubscribe.test:{}{path}", addr.port())
        .parse()
        .unwrap()
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
    let header = |name: &str| header(head, name);
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
    let (plain, connections) = hang_up().await;
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
    assert_eq!("http://plain.test/u".parse::<HttpsUrl>().ok(), None);
    let (plain, connections) = hang_up().await;
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
    let (addr, seen) =
        tls_server(|host, _| redirect("303 See Other", &format!("https://{host}/done"))).await;
    let err = unsubscribe::one_click(&client(addr, None), &url(addr, "/u"))
        .await
        .expect_err("a 303 is not an unsubscribe");
    assert_eq!(err, UnsubscribeFailure::Refused { status: 303 });
    assert_eq!(seen.lock().unwrap().len(), 1, "the redirect was followed");
}

#[tokio::test]
async fn a_307_to_https_is_followed_with_the_same_post() {
    let (addr, seen) = tls_server(|host, n| match n {
        0 => redirect("307 Temporary Redirect", &format!("https://{host}/done")),
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
    // A listener that hangs up before a word of TLS, and stays bound so the port is nobody else's.
    let (gone, connections) = hang_up().await;
    let err = unsubscribe::one_click(&client(gone, None), &url(gone, "/u"))
        .await
        .expect_err("nothing answered");
    assert!(matches!(err, UnsubscribeFailure::Unreachable(_)), "{err:?}");
    assert!(matches!(err.retry(), Retry::After(_)));
    assert_eq!(*connections.lock().unwrap(), 1);
}

/// A list address, or a redirect, inside the reader's own network is never posted to: the
/// request would be the reader's machine acting on a stranger's behalf where they cannot reach.
#[tokio::test]
async fn the_local_network_is_never_posted_to() {
    let (inside, connections) = hang_up().await;
    let direct: HttpsUrl = format!("https://127.0.0.1:{}/u", inside.port())
        .parse()
        .unwrap();
    let (addr, _) = tls_server(always(OK)).await;
    let err = unsubscribe::one_click(&client(addr, None), &direct)
        .await
        .expect_err("a loopback address is refused");
    assert!(
        matches!(err, UnsubscribeFailure::NotPublic { .. }),
        "{err:?}"
    );
    assert!(matches!(err.retry(), Retry::Fatal(_)));

    for to in [
        format!("https://127.0.0.1:{}/u", inside.port()),
        "https://169.254.169.254/latest/meta-data/".to_owned(),
        "https://printer.local/u".to_owned(),
    ] {
        let (addr, _) = tls_server(move |_, _| redirect("307 Temporary Redirect", &to)).await;
        let err = unsubscribe::one_click(&client(addr, None), &url(addr, "/u"))
            .await
            .expect_err("a redirect inside is refused");
        assert!(
            matches!(err, UnsubscribeFailure::NotPublic { .. }),
            "{err:?}"
        );
    }
    assert_eq!(*connections.lock().unwrap(), 0, "the inside was contacted");
}
