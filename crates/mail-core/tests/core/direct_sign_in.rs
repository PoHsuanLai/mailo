//! Signing in straight in the browser (`account::sign_in_with`), against an issuer in this
//! process: its token endpoint and the profile it names the account by. The "browser" is a thread
//! that answers the sign-in page by calling the loopback listener back, as a browser sent back by
//! the issuer does.

use mail_core::account::direct::{Profiles, WAIT};
use mail_core::account::{Direct, Mode, SignInError, sign_in_with};
use mail_core::{Environment, SqliteStore};
use mail_domain::AuthPlan;
use porter_core::{AccountId, Credential, EndpointUrl, SecretKey, SecretPurpose};
use porter_provider::{ClientEntry, Issuer, IssuerEndpoints};
use porter_secrets::MemorySecrets;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

const GRANT: &str =
    r#"{"access_token":"a1","refresh_token":"r1","token_type":"Bearer","expires_in":1800}"#;

/// What the issuer was asked: each request's line (`POST /token`) and its body.
type Asked = Arc<Mutex<Vec<(String, String)>>>;

/// An issuer on a port of this machine: a token endpoint that grants every request, Google's
/// userinfo and Graph's `/me`, each naming `address`.
async fn issuer(address: &'static str) -> (u16, Asked) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let asked: Asked = Arc::default();
    let recording = asked.clone();
    tokio::spawn(async move {
        while let Ok((sock, _)) = listener.accept().await {
            let asked = recording.clone();
            tokio::spawn(async move {
                let mut reader = BufReader::new(sock);
                let mut first = String::new();
                reader.read_line(&mut first).await.unwrap();
                let mut length = 0usize;
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
                let request: String = first
                    .split_whitespace()
                    .take(2)
                    .collect::<Vec<_>>()
                    .join(" ");
                asked
                    .lock()
                    .unwrap()
                    .push((request.clone(), String::from_utf8_lossy(&body).into_owned()));
                let answer = if request.starts_with("POST /token") {
                    GRANT.to_owned()
                } else if request.starts_with("GET /userinfo") {
                    format!(r#"{{"email":"{address}"}}"#)
                } else {
                    format!(r#"{{"mail":null,"userPrincipalName":"{address}"}}"#)
                };
                let reply = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\n\
                     content-length: {}\r\nconnection: close\r\n\r\n{answer}",
                    answer.len()
                );
                let _ = reader.get_mut().write_all(reply.as_bytes()).await;
            });
        }
    });
    (port, asked)
}

/// The OAuth client for `issuer`, its endpoints the ones on `port`.
fn client(issuer: Issuer, port: u16) -> ClientEntry {
    ClientEntry {
        endpoints: Some(IssuerEndpoints {
            authorize: EndpointUrl::parse(&format!("http://127.0.0.1:{port}/authorize")).unwrap(),
            token: EndpointUrl::parse(&format!("http://127.0.0.1:{port}/token")).unwrap(),
            revoke: None,
            device: None,
        }),
        ..mail_runtime::clients::entry(issuer, "client-id", None)
    }
}

fn profiles(port: u16) -> Profiles {
    Profiles {
        google: format!("http://127.0.0.1:{port}/userinfo"),
        microsoft: format!("http://127.0.0.1:{port}/me"),
    }
}

fn store() -> (SqliteStore, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::in_memory(dir.path()).unwrap();
    (store, dir)
}

/// A query parameter of the sign-in page's address.
fn param(url: &str, name: &str) -> String {
    url::Url::parse(url)
        .unwrap()
        .query_pairs()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.into_owned())
        .unwrap()
}

/// A browser that records the page it is asked to open and, when `back` names one, comes back
/// to the loopback with it (`{state}` standing for the real one).
fn browser(
    opened: Arc<Mutex<Vec<String>>>,
    back: Option<&'static str>,
) -> impl Fn(&str) -> std::io::Result<()> {
    move |url: &str| {
        opened.lock().unwrap().push(url.to_owned());
        if let Some(back) = back {
            let state = param(url, "state");
            let port = url::Url::parse(&param(url, "redirect_uri"))
                .unwrap()
                .port()
                .unwrap();
            std::thread::spawn(move || {
                let target = back.replace("{state}", &state);
                let mut sock = TcpStream::connect(("127.0.0.1", port)).unwrap();
                let _ = write!(sock, "GET {target} HTTP/1.1\r\nhost: x\r\n\r\n");
                let _ = sock.read_to_end(&mut Vec::new());
            });
        }
        Ok(())
    }
}

/// A cancel that never comes.
async fn never() {
    std::future::pending::<()>().await
}

#[tokio::test]
async fn signing_in_with_google_opens_the_page_once_and_adds_the_account_it_names() {
    let (port, asked) = issuer("you@example.test").await;
    let (store, _dir) = store();
    let secrets = MemorySecrets::default();
    let saved = mail_runtime::clients::registry_of(vec![client(Issuer::Google, port)]);
    let opened = Arc::new(Mutex::new(Vec::new()));
    let browse = browser(opened.clone(), Some("/?code=the-code&state={state}"));
    let added = sign_in_with(
        Issuer::Google,
        Mode::Add,
        Direct {
            store: &store,
            secrets: &secrets,
            saved: &saved,
            environment: &Environment::default(),
            browse: &browse,
            profiles: &profiles(port),
            wait: WAIT,
            now: chrono::Utc::now(),
        },
        never(),
    )
    .await
    .expect("the account is added");

    assert_eq!(added.address, "you@example.test");
    let opened = opened.lock().unwrap().clone();
    assert_eq!(opened.len(), 1, "the page is opened once: {opened:?}");
    assert!(
        opened[0].starts_with(&format!("http://127.0.0.1:{port}/authorize?")),
        "{}",
        opened[0]
    );
    assert!(param(&opened[0], "scope").contains("https://mail.google.com/"));

    // Kept as a Google account with its sign-in in the store of secrets, under its own id.
    let stored = store
        .account_by_address("you@example.test")
        .unwrap()
        .expect("the account is stored");
    assert!(matches!(
        stored.plan.unwrap().auth,
        AuthPlan::OAuth {
            issuer: Issuer::Google,
            ..
        }
    ));
    assert_eq!(stored.id, added.account);
    assert!(signed_in(&secrets, added.account).await);

    // The code was exchanged once, and the profile read with the sign-in's own token.
    let asked = asked.lock().unwrap();
    let requests: Vec<&str> = asked.iter().map(|(request, _)| request.as_str()).collect();
    assert_eq!(requests, ["POST /token", "GET /userinfo"]);
    assert!(asked[0].1.contains("code=the-code"), "{}", asked[0].1);
}

#[tokio::test]
async fn a_microsoft_sign_in_reads_the_address_from_graph_with_a_profile_token() {
    let (port, asked) = issuer("you@example.test").await;
    let (store, _dir) = store();
    let secrets = MemorySecrets::default();
    let saved = mail_runtime::clients::registry_of(vec![client(Issuer::Microsoft, port)]);
    let opened = Arc::new(Mutex::new(Vec::new()));
    let browse = browser(opened.clone(), Some("/?code=c&state={state}"));
    let added = sign_in_with(
        Issuer::Microsoft,
        Mode::Add,
        Direct {
            store: &store,
            secrets: &secrets,
            saved: &saved,
            environment: &Environment::default(),
            browse: &browse,
            profiles: &profiles(port),
            wait: WAIT,
            now: chrono::Utc::now(),
        },
        never(),
    )
    .await
    .expect("the account is added");
    assert_eq!(added.address, "you@example.test");
    // The page asks for the profile beside the mail scopes, and the code is redeemed for mail.
    let scope = param(&opened.lock().unwrap()[0], "scope");
    assert!(scope.contains("User.Read"), "{scope}");
    // An address no person's own Microsoft domain covers, and a token that names no tenant:
    // a work or school account, which reads and sends through Graph (Thunderbird's rule).
    let stored = store
        .account_by_address("you@example.test")
        .unwrap()
        .expect("the account is stored");
    assert_eq!(
        stored.plan.unwrap().incoming,
        mail_domain::Incoming::Graph,
        "a work or school account reads through Graph"
    );
    let asked = asked.lock().unwrap();
    let requests: Vec<&str> = asked.iter().map(|(request, _)| request.as_str()).collect();
    // The code, the profile token and the profile, then the Graph token the account reads with.
    assert_eq!(
        requests,
        ["POST /token", "POST /token", "GET /me", "POST /token"]
    );
    assert!(
        asked[0].1.contains("IMAP.AccessAsUser.All"),
        "{}",
        asked[0].1
    );
    assert!(
        asked[1].1.contains("grant_type=refresh_token"),
        "{}",
        asked[1].1
    );
}

#[tokio::test]
async fn closing_the_page_in_the_browser_is_a_cancelled_sign_in_and_adds_nothing() {
    let (port, asked) = issuer("you@example.test").await;
    let (store, _dir) = store();
    let secrets = MemorySecrets::default();
    let saved = mail_runtime::clients::registry_of(vec![client(Issuer::Google, port)]);
    let browse = browser(Arc::default(), Some("/?error=access_denied&state={state}"));
    let ended = sign_in_with(
        Issuer::Google,
        Mode::Add,
        Direct {
            store: &store,
            secrets: &secrets,
            saved: &saved,
            environment: &Environment::default(),
            browse: &browse,
            profiles: &profiles(port),
            wait: WAIT,
            now: chrono::Utc::now(),
        },
        never(),
    )
    .await;
    assert_eq!(ended, Err(SignInError::Declined));
    assert_eq!(ended.unwrap_err().to_string(), "Sign-in cancelled");
    assert!(store.list_accounts().unwrap().is_empty());
    assert!(asked.lock().unwrap().is_empty(), "nothing was exchanged");
}

#[tokio::test]
async fn no_client_for_the_issuer_opens_no_page() {
    let (store, _dir) = store();
    let opened = Arc::new(Mutex::new(Vec::new()));
    let browse = browser(opened.clone(), None);
    let ended = sign_in_with(
        Issuer::Google,
        Mode::Add,
        Direct {
            store: &store,
            secrets: &MemorySecrets::default(),
            saved: &mail_runtime::ClientRegistry::default(),
            environment: &Environment::default(),
            browse: &browse,
            profiles: &Profiles::default(),
            wait: WAIT,
            now: chrono::Utc::now(),
        },
        never(),
    )
    .await;
    assert_eq!(ended, Err(SignInError::NoClient(Issuer::Google)));
    assert!(opened.lock().unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn nobody_coming_back_ends_at_the_deadline() {
    let (store, _dir) = store();
    let saved = mail_runtime::clients::registry_of(vec![client(Issuer::Google, 9)]);
    let opened = Arc::new(Mutex::new(Vec::new()));
    let browse = browser(opened.clone(), None);
    let started = tokio::time::Instant::now();
    let ended = sign_in_with(
        Issuer::Google,
        Mode::Add,
        Direct {
            store: &store,
            secrets: &MemorySecrets::default(),
            saved: &saved,
            environment: &Environment::default(),
            browse: &browse,
            profiles: &Profiles::default(),
            wait: WAIT,
            now: chrono::Utc::now(),
        },
        never(),
    )
    .await;
    assert_eq!(ended, Err(SignInError::TimedOut));
    assert_eq!(opened.lock().unwrap().len(), 1);
    assert!(started.elapsed() >= WAIT, "{:?}", started.elapsed());
}

#[tokio::test(start_paused = true)]
async fn cancelling_ends_the_wait_and_frees_the_listener() {
    let (store, _dir) = store();
    let saved = mail_runtime::clients::registry_of(vec![client(Issuer::Google, 9)]);
    let opened = Arc::new(Mutex::new(Vec::new()));
    let browse = browser(opened.clone(), None);
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    // The person presses Cancel a minute into the wait.
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(60)).await;
        drop(stop);
    });
    let started = tokio::time::Instant::now();
    let ended = sign_in_with(
        Issuer::Google,
        Mode::Add,
        Direct {
            store: &store,
            secrets: &MemorySecrets::default(),
            saved: &saved,
            environment: &Environment::default(),
            browse: &browse,
            profiles: &Profiles::default(),
            wait: WAIT,
            now: chrono::Utc::now(),
        },
        async move {
            let _ = stopped.await;
        },
    )
    .await;
    assert_eq!(ended, Err(SignInError::Cancelled));
    assert!(started.elapsed() < WAIT, "{:?}", started.elapsed());
    // The browser coming back now finds nobody listening.
    let page = opened.lock().unwrap()[0].clone();
    let port = url::Url::parse(&param(&page, "redirect_uri"))
        .unwrap()
        .port()
        .unwrap();
    assert!(TcpStream::connect(("127.0.0.1", port)).is_err());
}

#[tokio::test]
async fn signing_in_again_as_someone_else_keeps_nothing() {
    let (port, _) = issuer("someone-else@example.test").await;
    let (store, _dir) = store();
    let secrets = MemorySecrets::default();
    let saved = mail_runtime::clients::registry_of(vec![client(Issuer::Google, port)]);
    // The account being signed in again, added the ordinary way first.
    let browse = browser(Arc::default(), Some("/?code=c&state={state}"));
    let (first_port, _) = issuer("you@example.test").await;
    let first = sign_in_with(
        Issuer::Google,
        Mode::Add,
        Direct {
            store: &store,
            secrets: &secrets,
            saved: &mail_runtime::clients::registry_of(vec![client(Issuer::Google, first_port)]),
            environment: &Environment::default(),
            browse: &browse,
            profiles: &profiles(first_port),
            wait: WAIT,
            now: chrono::Utc::now(),
        },
        never(),
    )
    .await
    .expect("the account is added");

    let again = sign_in_with(
        Issuer::Google,
        Mode::Again {
            address: "You@example.test".to_owned(),
        },
        Direct {
            store: &store,
            secrets: &secrets,
            saved: &saved,
            environment: &Environment::default(),
            browse: &browse,
            profiles: &profiles(port),
            wait: WAIT,
            now: chrono::Utc::now(),
        },
        never(),
    )
    .await;
    assert_eq!(
        again,
        Err(SignInError::OtherAccount {
            asked: "you@example.test".to_owned(),
            signed: "someone-else@example.test".to_owned(),
        })
    );
    assert_eq!(store.list_accounts().unwrap().len(), 1);
    assert!(signed_in(&secrets, first.account).await);
}

/// Whether `account` has a browser sign-in kept.
async fn signed_in(secrets: &MemorySecrets, account: AccountId) -> bool {
    use mail_runtime::AccountSecrets;
    matches!(
        secrets
            .get(&SecretKey {
                account,
                purpose: SecretPurpose::OAuthRefresh,
            })
            .await,
        Ok(Credential::OAuth { .. })
    )
}
