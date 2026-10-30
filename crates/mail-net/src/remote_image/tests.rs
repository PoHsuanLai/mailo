use super::*;

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";

fn image(content_type: Option<&str>, bytes: &[u8]) -> Image {
    Image {
        content_type: content_type.map(str::to_owned),
        bytes: bytes.to_vec(),
    }
}

#[test]
fn a_raster_image_of_the_kind_it_says_is_shown_as_a_data_uri() {
    let cases: [(&str, &[u8], &str); 4] = [
        ("image/png", PNG, "image/png"),
        (
            "image/jpeg",
            &[0xff, 0xd8, 0xff, 0xe0, 0, 0x10],
            "image/jpeg",
        ),
        ("image/gif", b"GIF89a\x01\0\x01\0", "image/gif"),
        ("image/webp", b"RIFF\x04\0\0\0WEBPVP8 ", "image/webp"),
    ];
    for (declared, bytes, kind) in cases {
        let uri = image(Some(declared), bytes).data_uri().expect(declared);
        assert!(uri.starts_with(&format!("data:{kind};base64,")), "{uri}");
    }
}

#[test]
fn the_type_written_is_the_one_the_bytes_are_in_the_allowlists_spelling() {
    // The server's spelling and parameters are not what is written.
    let uri = image(
        Some("IMAGE/JPEG; charset=binary"),
        &[0xff, 0xd8, 0xff, 0xe0],
    )
    .data_uri()
    .unwrap();
    assert!(uri.starts_with("data:image/jpeg;base64,"), "{uri}");
    // A declared type that the bytes are not is written as the bytes' own type.
    let uri = image(Some("image/gif"), PNG).data_uri().unwrap();
    assert!(uri.starts_with("data:image/png;base64,"), "{uri}");
}

#[test]
fn what_is_not_a_raster_image_is_never_shown() {
    let svg = b"<svg xmlns='http://www.w3.org/2000/svg'><script>alert(1)</script></svg>";
    let cases: [(&str, Option<&str>, &[u8]); 5] = [
        ("svg by its declared type", Some("image/svg+xml"), svg),
        ("svg dressed as a png", Some("image/png"), svg),
        ("no declared type", None, PNG),
        ("html", Some("text/html"), b"<html></html>"),
        ("nothing at all", Some("image/png"), b""),
    ];
    for (name, declared, bytes) in cases {
        assert_eq!(image(declared, bytes).data_uri(), None, "{name}");
    }
}

#[test]
fn an_image_past_the_largest_is_not_shown() {
    let mut bytes = PNG.to_vec();
    bytes.resize(MAX_BYTES + 1, 0);
    assert_eq!(image(Some("image/png"), &bytes).data_uri(), None);
    bytes.truncate(MAX_BYTES);
    assert!(image(Some("image/png"), &bytes).data_uri().is_some());
}

#[tokio::test]
async fn an_image_is_fetched_only_from_a_public_https_address() {
    let fetcher = ImageFetcher::new(tokio::runtime::Handle::current(), Route::Direct).unwrap();
    for url in [
        "http://127.0.0.1/a.png",
        "ftp://example.com/a.png",
        "https://127.0.0.1/a.png",
        "https://169.254.169.254/latest/meta-data/",
        "https://[::1]/a.png",
        "https://localhost/a.png",
        "https://router.local/a.png",
        "not a url",
    ] {
        assert!(fetcher.get(url).await.is_none(), "{url}");
    }
}

#[test]
fn an_http_image_is_asked_for_over_https() {
    for (url, fetched) in [
        (
            "http://example.com/a.png?w=2",
            Some("https://example.com/a.png?w=2"),
        ),
        (
            "http://example.com:80/a.png",
            Some("https://example.com/a.png"),
        ),
        (
            "http://example.com:8080/a.png",
            Some("https://example.com:8080/a.png"),
        ),
        (
            "https://example.com/a.png",
            Some("https://example.com/a.png"),
        ),
        ("http://192.168.1.1/a.png", None),
        ("data:image/png;base64,AA==", None),
    ] {
        assert_eq!(
            upgraded(url).as_ref().map(reqwest::Url::as_str),
            fetched,
            "{url}"
        );
    }
}

/// A loopback server that answers every request with `reply` (a whole HTTP response, or nothing
/// at all) and sends each request's head down the channel it returns with its address.
async fn listening(
    reply: &'static [u8],
) -> (
    std::net::SocketAddr,
    tokio::sync::mpsc::UnboundedReceiver<String>,
) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (heads, seen) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let heads = heads.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 8192];
                let n = stream.read(&mut buf).await.unwrap_or(0);
                // The test's receiver may be gone by the time a late request lands.
                heads
                    .send(String::from_utf8_lossy(&buf[..n]).into_owned())
                    .ok();
                stream.write_all(reply).await.ok();
                stream.shutdown().await.ok();
            });
        }
    });
    (addr, seen)
}

const REPLIED: &[u8] = b"HTTP/1.1 200 OK\r\ncontent-type: image/png\r\ncontent-length: 16\r\nconnection: close\r\n\r\n\x89PNG\r\n\x1a\n\0\0\0\rIHDR";

/// The origin is a public name nothing here can reach: a fetch that went to it would fail, and
/// the only server that sees a request is the relay's.
const ORIGIN: &str = "https://images.example.test/a.png?w=2";

#[tokio::test]
async fn a_camo_relay_gets_the_request_and_the_origin_never_does() {
    let (addr, mut seen) = listening(REPLIED).await;
    let route = Route::Camo {
        base: format!("http://{addr}/").parse().unwrap(),
        key: "k".to_owned(),
    };
    let fetcher = ImageFetcher::new(tokio::runtime::Handle::current(), route.clone()).unwrap();
    let got = fetcher.get(ORIGIN).await.expect("the relay's image");
    assert_eq!(got.content_type.as_deref(), Some("image/png"));
    let head = seen.recv().await.unwrap();
    let want = route.asked(&ORIGIN.parse().unwrap());
    assert!(head.starts_with(&format!("GET {} ", want.path())), "{head}");
    assert!(!head.to_lowercase().contains("authorization"), "{head}");
    assert!(!head.to_lowercase().contains("referer"), "{head}");
}

#[tokio::test]
async fn a_token_relay_is_asked_with_the_token_and_the_origin_never_is() {
    let (addr, mut seen) = listening(REPLIED).await;
    let route = Route::Token {
        base: format!("http://{addr}/").parse().unwrap(),
        token: "secret-token".to_owned(),
    };
    let fetcher = ImageFetcher::new(tokio::runtime::Handle::current(), route).unwrap();
    assert!(fetcher.get(ORIGIN).await.is_some());
    let head = seen.recv().await.unwrap();
    assert!(
        head.starts_with("GET /i?u=https%3A%2F%2Fimages.example.test%2Fa.png%3Fw%3D2 "),
        "{head}"
    );
    assert!(
        head.contains("Bearer secret-token") || head.contains("bearer secret-token"),
        "{head}"
    );
}

#[tokio::test]
async fn a_relay_is_never_sent_a_private_address_to_fetch() {
    let (addr, mut seen) = listening(REPLIED).await;
    let route = Route::Token {
        base: format!("http://{addr}/").parse().unwrap(),
        token: "t".to_owned(),
    };
    let fetcher = ImageFetcher::new(tokio::runtime::Handle::current(), route).unwrap();
    assert!(fetcher.get("https://192.168.1.1/a.png").await.is_none());
    assert!(fetcher.get("https://localhost/a.png").await.is_none());
    assert!(seen.try_recv().is_err());
}

#[tokio::test]
async fn a_relay_that_redirects_is_not_followed_to_the_origin() {
    let (addr, mut seen) = listening(
        b"HTTP/1.1 302 Found\r\nlocation: https://images.example.test/a.png\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
    )
    .await;
    let route = Route::Token {
        base: format!("http://{addr}/").parse().unwrap(),
        token: "t".to_owned(),
    };
    let fetcher = ImageFetcher::new(tokio::runtime::Handle::current(), route).unwrap();
    assert!(fetcher.get(ORIGIN).await.is_none());
    assert!(seen.recv().await.is_some());
}

#[tokio::test]
async fn a_proxy_is_asked_to_connect_to_the_origin_and_nothing_goes_round_it() {
    // A proxy that refuses every tunnel: the fetch fails, and the CONNECT it asked for names
    // the origin, which no one reached any other way.
    let (addr, mut seen) =
        listening(b"HTTP/1.1 502 Bad Gateway\r\ncontent-length: 0\r\n\r\n").await;
    let route = Route::Proxy {
        url: format!("http://{addr}"),
    };
    let fetcher = ImageFetcher::new(tokio::runtime::Handle::current(), route).unwrap();
    assert!(fetcher.get(ORIGIN).await.is_none());
    let head = seen.recv().await.unwrap();
    assert!(
        head.starts_with("CONNECT images.example.test:443 "),
        "{head}"
    );
}

/// A relay is the person's own server, so the system's authorities are believed for it; what a
/// sender chose, and the proxy that reaches it, keep to the bundled roots.
#[test]
fn only_a_relay_is_checked_against_the_systems_authorities() {
    let base: reqwest::Url = "https://relay.example.test/".parse().unwrap();
    for (route, system) in [
        (Route::Direct, false),
        (
            Route::Proxy {
                url: "socks5h://127.0.0.1:9050".to_owned(),
            },
            false,
        ),
        (
            Route::Camo {
                base: base.clone(),
                key: "k".to_owned(),
            },
            true,
        ),
        (
            Route::Token {
                base,
                token: "t".to_owned(),
            },
            true,
        ),
    ] {
        assert_eq!(trusts_system(&route), system, "{route:?}");
    }
}
