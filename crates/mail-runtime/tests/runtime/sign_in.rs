//! The browser sign-in against a token endpoint in this process, with the "browser" a thread
//! that answers the authorize URL by calling the loopback listener back.
//!
//! These are the cases mailo's own loopback and OAuth tests held before `porter-oauth` took the
//! flow over: the redirect URI without a trailing slash, a redirect with the wrong `state`
//! not ending the wait, a token answer without `expires_in`, and the issuer's own words in a
//! refusal.

use mail_runtime::RuntimeError;
use mail_runtime::authorize::sign_in_within;
use porter_core::{Credential, EndpointUrl, UnixSeconds};
use porter_provider::{ClientEntry, Issuer, IssuerEndpoints};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

/// The token requests the endpoint received, as their form bodies.
type Bodies = Arc<Mutex<Vec<String>>>;

/// A token endpoint that answers every request with `status` and `answer`.
async fn token_endpoint(status: &'static str, answer: &'static str) -> (ClientEntry, Bodies) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let bodies: Bodies = Arc::default();
    let recording = bodies.clone();
    tokio::spawn(async move {
        while let Ok((sock, _)) = listener.accept().await {
            let bodies = recording.clone();
            tokio::spawn(async move {
                let mut reader = BufReader::new(sock);
                let mut length = 0usize;
                let mut first = String::new();
                reader.read_line(&mut first).await.unwrap();
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).await.unwrap();
                    let line = line.trim_end().to_ascii_lowercase();
                    if line.is_empty() {
                        break;
                    }
                    if let Some(v) = line.strip_prefix("content-length:") {
                        length = v.trim().parse().unwrap();
                    }
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).await.unwrap();
                bodies
                    .lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&body).into_owned());
                let reply = format!(
                    "HTTP/1.1 {status}\r\ncontent-type: application/json\r\n\
                     content-length: {}\r\nconnection: close\r\n\r\n{answer}",
                    answer.len()
                );
                let _ = reader.get_mut().write_all(reply.as_bytes()).await;
            });
        }
    });
    let client = ClientEntry {
        endpoints: Some(IssuerEndpoints {
            authorize: EndpointUrl::parse(&format!("http://127.0.0.1:{port}/authorize")).unwrap(),
            token: EndpointUrl::parse(&format!("http://127.0.0.1:{port}/token")).unwrap(),
            revoke: None,
            device: None,
        }),
        ..mail_runtime::clients::entry(Issuer::Microsoft, "client-id", None)
    };
    (client, bodies)
}

const SCOPES: &[&str] = &[
    "https://outlook.office.com/IMAP.AccessAsUser.All",
    "offline_access",
];

fn scopes() -> Vec<String> {
    SCOPES.iter().map(|s| (*s).to_owned()).collect()
}

fn form(body: &str, name: &str) -> Option<String> {
    url::form_urlencoded::parse(body.as_bytes())
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.into_owned())
}

/// What the authorize URL asked for.
#[derive(Debug, Clone)]
struct Asked {
    redirect: String,
    state: String,
}

/// Sign in with a browser that, shown the authorize URL, makes each of `visits` (a request
/// target after the host, `{state}` standing for the real one) in order, and returns the outcome
/// with what the authorize URL named.
async fn sign_in_visiting(
    client: &ClientEntry,
    visits: &'static [&'static str],
    wait: Duration,
) -> (Result<Credential, RuntimeError>, Asked) {
    let asked: Arc<Mutex<Option<Asked>>> = Arc::default();
    let seen = asked.clone();
    let on_url = move |address: &str| {
        let url = url::Url::parse(address).unwrap();
        let pair = |name: &str| {
            url.query_pairs()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.into_owned())
                .unwrap()
        };
        let asking = Asked {
            redirect: pair("redirect_uri"),
            state: pair("state"),
        };
        *seen.lock().unwrap() = Some(asking.clone());
        let port = url::Url::parse(&asking.redirect).unwrap().port().unwrap();
        std::thread::spawn(move || {
            for visit in visits {
                let target = visit.replace("{state}", &asking.state);
                let mut sock = TcpStream::connect(("127.0.0.1", port)).unwrap();
                let _ = write!(sock, "GET {target} HTTP/1.1\r\nhost: x\r\n\r\n");
                let _ = sock.read_to_end(&mut Vec::new());
            }
        });
    };
    let outcome = sign_in_within(
        &client.clone(),
        &scopes(),
        &on_url,
        UnixSeconds(1_000),
        wait,
    )
    .await;
    let asked = asked.lock().unwrap().clone().expect("the URL was offered");
    (outcome, asked)
}

const GRANT: &str =
    r#"{"access_token":"a1","refresh_token":"r1","token_type":"Bearer","expires_in":1800}"#;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_redirect_uri_has_no_trailing_slash_and_is_the_same_in_both_requests() {
    let (client, bodies) = token_endpoint("200 OK", GRANT).await;
    let (outcome, asked) = sign_in_visiting(
        &client,
        &["/?code=the-code&state={state}"],
        Duration::from_secs(10),
    )
    .await;
    outcome.expect("the sign-in completes");
    assert!(
        asked.redirect.starts_with("http://127.0.0.1:") && !asked.redirect.ends_with('/'),
        "{}",
        asked.redirect
    );
    let bodies = bodies.lock().unwrap();
    assert_eq!(bodies.len(), 1);
    assert_eq!(form(&bodies[0], "redirect_uri"), Some(asked.redirect));
    assert_eq!(form(&bodies[0], "code").as_deref(), Some("the-code"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_redirect_with_the_wrong_state_does_not_end_the_wait() {
    // Another party's code offered to our listener, then the real redirect.
    let (client, bodies) = token_endpoint("200 OK", GRANT).await;
    let (outcome, _) = sign_in_visiting(
        &client,
        &["/?code=stolen&state=wrong", "/?code=ours&state={state}"],
        Duration::from_secs(10),
    )
    .await;
    outcome.expect("the real redirect still completes the sign-in");
    let bodies = bodies.lock().unwrap();
    assert_eq!(bodies.len(), 1, "the stolen code was exchanged");
    assert_eq!(form(&bodies[0], "code").as_deref(), Some("ours"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_wait_with_only_a_wrong_state_ends_at_the_deadline_without_an_exchange() {
    let (client, bodies) = token_endpoint("200 OK", GRANT).await;
    let (outcome, _) = sign_in_visiting(
        &client,
        &["/?code=stolen&state=wrong"],
        Duration::from_millis(500),
    )
    .await;
    let error = outcome.expect_err("no right redirect came");
    assert!(error.to_string().contains("did not complete"), "{error}");
    assert!(bodies.lock().unwrap().is_empty(), "a code was exchanged");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_token_answer_without_expires_in_lasts_an_hour() {
    let (client, _) = token_endpoint(
        "200 OK",
        r#"{"access_token":"a1","refresh_token":"r1","token_type":"Bearer"}"#,
    )
    .await;
    let (outcome, _) = sign_in_visiting(
        &client,
        &["/?code=c&state={state}"],
        Duration::from_secs(10),
    )
    .await;
    match outcome.expect("the sign-in completes") {
        Credential::OAuth { expires_at, .. } => assert_eq!(expires_at, UnixSeconds(1_000 + 3600)),
        other => panic!("not an OAuth credential: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_refused_exchange_carries_the_issuers_own_description() {
    let (client, _) = token_endpoint(
        "400 Bad Request",
        r#"{"error":"invalid_grant","error_description":"The code has expired."}"#,
    )
    .await;
    let (outcome, _) = sign_in_visiting(
        &client,
        &["/?code=c&state={state}"],
        Duration::from_secs(10),
    )
    .await;
    let error = outcome
        .expect_err("the issuer refused the code")
        .to_string();
    assert!(error.contains("the issuer refused it"), "{error}");
    assert!(error.contains("The code has expired."), "{error}");
}
