//! The OAuth token exchange, which had never run.
//!
//! `begin` is covered by unit tests: the authorize URL carries PKCE, `state` and `access_type`,
//! and a mismatched `state` is refused. `exchange` and `refresh` had no tests at all — they are
//! the two functions that speak HTTP, and they are exactly the code that runs the first time
//! someone supplies a client id. Every other never-executed path in this project turned out to
//! be broken, so this one is executed before a user finds out.
//!
//! No network: the token endpoint is a `TcpListener` in this process, which is what
//! `Endpoints` being substitutable is for.

use chrono::{DateTime, TimeZone, Utc};
use mail_domain::{Credential, OAuthIssuer};
use mail_runtime::oauth::{self, Endpoints};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

fn now() -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000, 0).unwrap()
}

/// What the token endpoint was asked, so the request shape can be asserted.
type Seen = Arc<Mutex<Vec<String>>>;

/// A token endpoint that answers `body` to anything.
async fn serve(body: &'static str, status: &'static str) -> (Endpoints, Seen) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen: Seen = Arc::new(Mutex::new(Vec::new()));
    let recording = seen.clone();

    tokio::spawn(async move {
        while let Ok((mut sock, _)) = listener.accept().await {
            let recording = recording.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 8192];
                let read = match sock.read(&mut buf).await {
                    Ok(0) | Err(_) => return,
                    Ok(n) => n,
                };
                recording
                    .lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&buf[..read]).to_string());
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = sock.write_all(response.as_bytes()).await;
                let _ = sock.flush().await;
            });
        }
    });

    (
        Endpoints {
            // The authorize URL is never fetched — the browser goes there — but it must parse.
            auth: format!("http://127.0.0.1:{port}/authorize"),
            token: format!("http://127.0.0.1:{port}/token"),
        },
        seen,
    )
}

const GOOD: &str = r#"{"access_token":"ya29.the-access-token",
"refresh_token":"1//the-refresh-token","token_type":"Bearer","expires_in":3599}"#;

fn pending(ends: Endpoints) -> oauth::Pending {
    oauth::begin(
        OAuthIssuer::Google,
        "client-id.apps.googleusercontent.com",
        &["https://mail.google.com/".to_owned()],
        "http://127.0.0.1:8080",
    )
    .expect("a valid authorize request")
    .pending
    .with_endpoints(ends)
}

#[tokio::test]
async fn an_authorization_code_becomes_a_stored_credential() {
    let (ends, seen) = serve(GOOD, "200 OK").await;
    let http = reqwest::Client::new();

    let credential = pending(ends)
        .exchange("the-authorization-code", &http, now())
        .await
        .expect("the token endpoint answered");

    match credential {
        Credential::OAuth {
            access,
            refresh,
            expires_at,
        } => {
            assert_eq!(access, "ya29.the-access-token");
            // Without a refresh token the account stops working an hour later, which is the
            // failure `access_type=offline` exists to prevent.
            assert_eq!(refresh, "1//the-refresh-token");
            assert_eq!(
                expires_at,
                now() + chrono::TimeDelta::try_seconds(3599).unwrap()
            );
        }
        other => panic!("expected an OAuth credential, got {other:?}"),
    }

    // The request shape, which is what a real issuer validates.
    let request = seen.lock().unwrap().join("\n");
    assert!(request.contains("POST /token"), "{request}");
    assert!(
        request.contains("grant_type=authorization_code"),
        "{request}"
    );
    assert!(request.contains("code=the-authorization-code"), "{request}");
    // PKCE: the verifier proves this is the client that asked for the code.
    assert!(
        request.contains("code_verifier="),
        "no PKCE verifier: {request}"
    );
    // An installed app has no secret, and sending an empty one is how a client gets rejected.
    assert!(
        !request.contains("client_secret="),
        "an installed app must not send a secret: {request}"
    );
}

#[tokio::test]
async fn a_refresh_returns_a_credential_with_a_new_expiry() {
    let (ends, seen) = serve(GOOD, "200 OK").await;
    let http = reqwest::Client::new();

    let credential = oauth::refresh_at(&ends, "client-id", "1//old-refresh", &http, now())
        .await
        .expect("the token endpoint answered");

    match credential {
        Credential::OAuth {
            access, expires_at, ..
        } => {
            assert_eq!(access, "ya29.the-access-token");
            assert!(
                expires_at > now(),
                "a refreshed token must not be already expired"
            );
        }
        other => panic!("{other:?}"),
    }
    let request = seen.lock().unwrap().join("\n");
    assert!(request.contains("grant_type=refresh_token"), "{request}");
}

#[tokio::test]
async fn a_response_with_no_refresh_token_keeps_the_one_we_had() {
    // Google omits the refresh token on a repeat authorization. A client that overwrites the
    // stored one with an empty string loses the account an hour later.
    let (ends, _seen) = serve(
        r#"{"access_token":"ya29.new","token_type":"Bearer","expires_in":3599}"#,
        "200 OK",
    )
    .await;
    let http = reqwest::Client::new();

    let credential = oauth::refresh_at(&ends, "client-id", "1//still-good", &http, now()).await;
    match credential {
        Ok(Credential::OAuth { refresh, .. }) => assert_eq!(
            refresh, "1//still-good",
            "the refresh token we already had was discarded"
        ),
        Ok(other) => panic!("{other:?}"),
        Err(e) => panic!("a response without a refresh token should not fail: {e}"),
    }
}

#[tokio::test]
async fn a_rejected_code_is_an_error_rather_than_a_credential() {
    let (ends, _seen) = serve(
        r#"{"error":"invalid_grant","error_description":"Bad Request"}"#,
        "400 Bad Request",
    )
    .await;
    let http = reqwest::Client::new();

    let outcome = pending(ends).exchange("stale-code", &http, now()).await;
    let err = outcome.expect_err("invalid_grant is not a credential");
    let text = format!("{err}");
    assert!(
        text.contains("invalid_grant") || text.to_lowercase().contains("token exchange failed"),
        "the reason should survive: {text}"
    );
}
