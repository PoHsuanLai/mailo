//! `sync::check_sign_in`: the sign-in an account is added with, tried against its server while
//! the person is still at the form, so a wrong user name or password is said there, in the
//! server's words, and not found at the first sync as "sign in again".
//!
//! The server is a `TcpListener` in this process speaking just enough POP3 to take or refuse a
//! login. Nothing here reaches a real server or a keyring.

use crate::support::{account, now};
use mail_core::CoreError;
use mail_core::sync::check_sign_in;
use mail_domain::*;
use mail_secrets::AccountSecrets;
use mail_store::SqliteStore;
use porter_core::{Credential, SecretKey, SecretPurpose, SecretText};
use porter_secrets::MemorySecrets;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

/// A POP3 server that takes the user `ada` with the password `right` and refuses any other
/// login with the reply a server gives. The port it listens on.
async fn serve() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((sock, _)) = listener.accept().await {
            tokio::spawn(async move {
                let (read, mut write) = sock.into_split();
                let mut lines = BufReader::new(read).lines();
                let _ = write.write_all(b"+OK ready\r\n").await;
                let mut user = String::new();
                while let Ok(Some(line)) = lines.next_line().await {
                    let mut words = line.split_whitespace();
                    let verb = words.next().unwrap_or("").to_ascii_uppercase();
                    let arg = words.next().unwrap_or("").to_owned();
                    let reply: &str = match verb.as_str() {
                        "CAPA" => "+OK\r\nUSER\r\nTOP\r\nUIDL\r\n.\r\n",
                        "USER" => {
                            user = arg;
                            "+OK\r\n"
                        }
                        "PASS" if user == "ada" && arg == "right" => "+OK welcome\r\n",
                        "PASS" => "-ERR [AUTH] Authentication failed.\r\n",
                        "QUIT" => {
                            let _ = write.write_all(b"+OK bye\r\n").await;
                            return;
                        }
                        _ => "-ERR unknown\r\n",
                    };
                    if write.write_all(reply.as_bytes()).await.is_err() {
                        return;
                    }
                }
            });
        }
    });
    port
}

/// A store holding one POP3 account on `port` that signs in as `username`, and secrets holding
/// `password` for it.
fn added(
    port: u16,
    username: Username,
    password: &str,
) -> (Arc<SqliteStore>, Arc<dyn AccountSecrets>) {
    // Plaintext because this is loopback to a server in this process.
    let incoming = Incoming::Pop3 {
        host: "127.0.0.1".to_owned(),
        port,
        tls: Tls::Plaintext,
        leave: LeaveOnServer::Keep,
    };
    added_on(incoming, username, password)
}

/// [`added`], with `incoming` as the account's incoming server.
fn added_on(
    incoming: Incoming,
    username: Username,
    password: &str,
) -> (Arc<SqliteStore>, Arc<dyn AccountSecrets>) {
    let store = Arc::new(mail_store::testing::in_memory());
    let manual = presets::ManualPop3 {
        pop3_host: "127.0.0.1".to_owned(),
        pop3_port: 110,
        smtp_host: "127.0.0.1".to_owned(),
        smtp_port: 1,
        login: None,
    };
    let mut preset = presets::manual_pop3("ada@example.test", &manual, now());
    preset.plan.incoming = incoming;
    preset.plan.auth = AuthPlan::Password {
        username,
        sasl: Vec::new(),
    };
    mail_store::testing::seed_account_plan(
        &store,
        account(),
        "ada@example.test",
        &preset.plan,
        None,
    );
    mail_store::testing::seed_caps(&store, account(), &preset.expected_caps, now()).unwrap();
    let secrets: Arc<dyn AccountSecrets> = Arc::new(MemorySecrets::default());
    mail_secrets::block_on(secrets.put(
        &SecretKey {
            account: account(),
            purpose: SecretPurpose::IncomingPassword,
        },
        &Credential::Password(SecretText::new(password.to_owned())),
    ))
    .unwrap();
    (store, secrets)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_login_the_server_takes_passes() {
    let port = serve().await;
    let (store, secrets) = added(port, Username::Literal("ada".to_owned()), "right");
    check_sign_in(&store, "ada@example.test", secrets, now())
        .await
        .expect("the server takes this login");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_wrong_password_is_refused_in_the_servers_words() {
    let port = serve().await;
    let (store, secrets) = added(port, Username::Literal("ada".to_owned()), "wrong");
    let refused = check_sign_in(&store, "ada@example.test", secrets, now())
        .await
        .expect_err("the server refuses this password");
    let said = refused.to_string();
    assert!(matches!(refused, CoreError::SignInRefused { .. }), "{said}");
    assert!(said.contains("127.0.0.1"), "names the server: {said}");
    assert!(
        said.contains("as ada:"),
        "names the user name tried: {said}"
    );
    assert!(
        said.contains("Authentication failed"),
        "the server's words: {said}"
    );
    assert!(
        !said.contains("without the part after the @"),
        "a name with no @ is not told to drop one: {said}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_full_address_as_user_name_is_told_to_try_without_the_domain() {
    let port = serve().await;
    let (store, secrets) = added(port, Username::SameAsAddress, "right");
    let refused = check_sign_in(&store, "ada@example.test", secrets, now())
        .await
        .expect_err("this server wants `ada`, not the address");
    let said = refused.to_string();
    assert!(said.contains("as ada@example.test:"), "{said}");
    assert!(
        said.contains("without the part after the @"),
        "the likeliest fix is named: {said}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_server_nobody_listens_on_is_unreachable_not_refused() {
    // Bind and drop, so the port is one nobody answers on.
    let port = {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        listener.local_addr().unwrap().port()
    };
    let (store, secrets) = added(port, Username::Literal("ada".to_owned()), "right");
    let failed = check_sign_in(&store, "ada@example.test", secrets, now())
        .await
        .expect_err("nothing answers");
    assert!(
        matches!(failed, CoreError::ServerUnreachable { .. }),
        "an unreachable server is not a refused login: {failed}"
    );
    assert!(failed.to_string().contains("127.0.0.1"), "{failed}");
}

/// A server that answers each protocol's greeting, refuses its upgrade to TLS, and keeps every
/// line it was sent. The port it listens on, and the lines.
async fn refusing_tls(greeting: &'static str) -> (u16, Arc<std::sync::Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let heard = Arc::new(std::sync::Mutex::new(Vec::new()));
    let kept = heard.clone();
    tokio::spawn(async move {
        while let Ok((sock, _)) = listener.accept().await {
            let kept = kept.clone();
            tokio::spawn(async move {
                let (read, mut write) = sock.into_split();
                let mut lines = BufReader::new(read).lines();
                let _ = write.write_all(greeting.as_bytes()).await;
                while let Ok(Some(line)) = lines.next_line().await {
                    kept.lock().unwrap().push(line.clone());
                    let reply = match line.split_whitespace().collect::<Vec<_>>().as_slice() {
                        ["STLS"] => "-ERR no TLS here\r\n".to_owned(),
                        ["CAPA"] => "+OK\r\nUSER\r\n.\r\n".to_owned(),
                        [tag, verb] if verb.eq_ignore_ascii_case("STARTTLS") => {
                            format!("{tag} NO no TLS here\r\n")
                        }
                        [tag, ..] if !tag.starts_with(char::is_uppercase) => {
                            format!("{tag} OK\r\n")
                        }
                        _ => "+OK\r\n".to_owned(),
                    };
                    if write.write_all(reply.as_bytes()).await.is_err() {
                        return;
                    }
                }
            });
        }
    });
    (port, heard)
}

/// An account whose incoming server is to be secured with STARTTLS signs in only over TLS: when
/// the server will not upgrade, the check fails and the server never hears the password.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_server_that_will_not_start_tls_never_hears_the_password() {
    for imap in [false, true] {
        let greeting = if imap {
            "* OK ready\r\n"
        } else {
            "+OK ready\r\n"
        };
        let (port, heard) = refusing_tls(greeting).await;
        let incoming = if imap {
            Incoming::Imap {
                host: "127.0.0.1".to_owned(),
                port,
                tls: Tls::StartTlsRequired,
            }
        } else {
            Incoming::Pop3 {
                host: "127.0.0.1".to_owned(),
                port,
                tls: Tls::StartTlsRequired,
                leave: LeaveOnServer::Keep,
            }
        };
        let (store, secrets) = added_on(incoming, Username::Literal("ada".to_owned()), "right");
        check_sign_in(&store, "ada@example.test", secrets, now())
            .await
            .expect_err("the server will not secure the connection");
        let heard = heard.lock().unwrap().clone();
        assert!(
            heard.iter().all(|line| !line.contains("right")),
            "imap {imap}: the password went out in the clear: {heard:?}"
        );
        let first = heard.first().map(String::as_str).unwrap_or("");
        assert!(
            first.ends_with("STLS") || first.ends_with("STARTTLS"),
            "imap {imap}: the upgrade was not the first thing asked: {heard:?}"
        );
    }
}
