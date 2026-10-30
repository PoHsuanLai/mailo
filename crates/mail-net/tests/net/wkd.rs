//! Web Key Directory lookup against a directory in this process.
//!
//! No network. The "directory" is a `TcpListener` behind TLS with the certificate `tls_fixture`
//! serves, and the client is told to trust that certificate and to resolve the test names to the
//! listener. The URLs are the ones `mail_crypto::openpgp::wkd::urls` derives, with the
//! listener's port added — a directory on port 443 is not something a test can open.

use chrono::{TimeZone, Utc};
use mail_crypto::openpgp::{self, SecretCert, wkd};
use mail_net::testing::tls::{
    NOT_FOUND, hang_up, header, ok, read_request, reply, root, serve_tls,
};
use rand::SeedableRng;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

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
    let seen: Seen = Arc::default();
    let recording = seen.clone();
    let answer = Arc::new(answer);
    let addr = serve_tls(move |mut stream| {
        let (recording, answer) = (recording.clone(), answer.clone());
        async move {
            let Some(text) = read_request(&mut stream).await else {
                return;
            };
            let path = text.split_whitespace().nth(1).unwrap_or("").to_owned();
            let host = header(&text, "host")
                .map(|value| value.split(':').next().unwrap_or("").to_owned())
                .unwrap_or_default();
            recording.lock().unwrap().push(format!("{host}{path}"));
            let response = match answer(&host, &path) {
                Some(body) => ok(Some("application/octet-stream"), &body),
                None => NOT_FOUND.to_vec(),
            };
            reply(&mut stream, &response).await;
        }
    })
    .await;
    (addr, seen)
}

/// The client `mailo pgp lookup` uses, trusting the fixture and resolving the test names to `to`.
fn client(to: SocketAddr) -> reqwest::Client {
    mail_net::wkd::client_builder()
        .add_root_certificate(root())
        .resolve("wkd.test", to)
        .resolve("openpgpkey.wkd.test", to)
        .resolve("direct-only.test", to)
        .resolve("openpgpkey.direct-only.test", to)
        .build()
        .unwrap()
}

/// `address`'s URLs with `port` on each host.
fn at_port(address: &str, port: u16) -> wkd::WkdUrls {
    at_ports(address, port, port)
}

/// `address`'s URLs with `advanced` on the advanced host and `direct` on the direct one.
fn at_ports(address: &str, advanced: u16, direct: u16) -> wkd::WkdUrls {
    let urls = wkd::urls(address).unwrap();
    wkd::WkdUrls {
        advanced: urls.advanced.replacen(
            &format!("https://{}/", urls.advanced_host),
            &format!("https://{}:{advanced}/", urls.advanced_host),
            1,
        ),
        direct: urls.direct.replacen(
            &format!("https://{}/", urls.direct_host),
            &format!("https://{}:{direct}/", urls.direct_host),
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
    let found = mail_net::wkd::fetch(
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
    let found = mail_net::wkd::fetch(
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
    let (addr, seen) = directory(move |_, _| Some(served.clone())).await;
    // `openpgpkey.direct-only.test` hangs up before a handshake: the advanced host is not there.
    let (gone, tried) = hang_up().await;
    let found = mail_net::wkd::fetch(
        &client(addr),
        &at_ports("joe@direct-only.test", gone.port(), addr.port()),
        "joe@direct-only.test",
    )
    .await;
    assert_eq!(found.map(|c| c.fingerprint()), Some(joe.fingerprint()));
    assert_eq!(
        *tried.lock().unwrap(),
        1,
        "the advanced host was asked first"
    );
    assert_eq!(
        seen.lock().unwrap().len(),
        1,
        "and only the direct one answered"
    );
}

#[tokio::test]
async fn a_directory_without_the_addresss_key_finds_nothing() {
    let other = key_for("other@wkd.test", 4);
    let served = other.public().to_bytes();
    let (addr, seen) = directory(move |_, _| Some(served.clone())).await;
    let found = mail_net::wkd::fetch(
        &client(addr),
        &at_port("joe@wkd.test", addr.port()),
        "joe@wkd.test",
    )
    .await;
    assert!(found.is_none(), "a key for someone else is not joe's");
    assert_eq!(seen.lock().unwrap().len(), 2, "both methods were asked");

    let (addr, _) = directory(|_, _| None).await;
    assert!(
        mail_net::wkd::fetch(
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
        mail_net::wkd::fetch(&client(addr), &urls, "joe@wkd.test")
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
    let http = mail_net::wkd::client().unwrap();
    assert!(
        mail_net::wkd::lookup(&http, "not an address")
            .await
            .is_err()
    );
}
