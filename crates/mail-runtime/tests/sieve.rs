//! Pushing rules to a ManageSieve server in this process.
//!
//! No network: the server is a `TcpListener` on loopback that speaks enough of RFC 5804 to keep
//! scripts, list them, and switch the active one. Plaintext, because the client here cannot be
//! told to trust a test certificate; the STARTTLS half of the protocol is covered byte for byte
//! by the transcripts in `mail-proto/tests/sieve.rs`, and the last test below checks the one
//! thing only a socket can — that a server without it never hears a credential.

use chrono::{TimeZone, Utc};
use mail_domain::{
    AccountId, AfterMatch, Credential, Filter, Rule, RuleAction, RuleId, RuleState, TextMatch, Tls,
};
use mail_proto::sieve::{Deleted, Endpoint, Places, SieveOutcome, Takeover, Unmappable};
use mail_runtime::RuntimeError;
use mail_runtime::sieve::{SieveAuth, push};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

const ACCOUNT: AccountId = AccountId::from_uuid(uuid::Uuid::from_u128(1));

/// What the fake server holds and has heard.
#[derive(Debug, Default)]
struct Server {
    /// (name, script, active)
    scripts: Vec<(String, String, bool)>,
    starttls: bool,
    /// Every command line, in order, credentials included — it is our own fake.
    heard: Vec<String>,
}

fn unquote(word: &str) -> String {
    word.trim().trim_matches('"').to_owned()
}

async fn serve(listener: TcpListener, state: Arc<Mutex<Server>>) {
    loop {
        let Ok((socket, _)) = listener.accept().await else {
            return;
        };
        let state = state.clone();
        tokio::spawn(async move {
            let (read, mut write) = socket.into_split();
            let mut read = BufReader::new(read);
            let offers_tls = state.lock().unwrap().starttls;
            let mut greeting = String::from(
                "\"IMPLEMENTATION\" \"Loopback Sieve\"\r\n\"SASL\" \"PLAIN\"\r\n\
                 \"SIEVE\" \"fileinto imap4flags vacation\"\r\n",
            );
            if offers_tls {
                greeting.push_str("\"STARTTLS\"\r\n");
            }
            greeting.push_str("OK \"ready\"\r\n");
            write.write_all(greeting.as_bytes()).await.unwrap();
            loop {
                let mut line = String::new();
                if read.read_line(&mut line).await.unwrap_or(0) == 0 {
                    return;
                }
                let line = line.trim_end().to_owned();
                state.lock().unwrap().heard.push(line.clone());
                let (verb, rest) = line.split_once(' ').unwrap_or((line.as_str(), ""));
                let reply = match verb {
                    "AUTHENTICATE" => "OK \"signed in\"\r\n".to_owned(),
                    "LISTSCRIPTS" => {
                        let mut out = String::new();
                        for (name, _, active) in &state.lock().unwrap().scripts {
                            let flag = if *active { " ACTIVE" } else { "" };
                            out.push_str(&format!("\"{name}\"{flag}\r\n"));
                        }
                        out + "OK\r\n"
                    }
                    "GETSCRIPT" => {
                        let name = unquote(rest);
                        let held = state.lock().unwrap();
                        let script = held
                            .scripts
                            .iter()
                            .find(|(n, _, _)| *n == name)
                            .map(|(_, s, _)| s.clone())
                            .unwrap_or_default();
                        format!("{{{}}}\r\n{script}\r\nOK\r\n", script.len())
                    }
                    "PUTSCRIPT" => {
                        let (name, literal) = rest.split_once(' ').unwrap();
                        let length: usize = literal
                            .trim_start_matches('{')
                            .trim_end_matches("+}")
                            .parse()
                            .unwrap();
                        let mut script = vec![0u8; length + 2];
                        read.read_exact(&mut script).await.unwrap();
                        script.truncate(length);
                        let script = String::from_utf8(script).unwrap();
                        let name = unquote(name);
                        let mut held = state.lock().unwrap();
                        held.scripts.retain(|(n, _, _)| *n != name);
                        held.scripts.push((name, script, false));
                        "OK\r\n".to_owned()
                    }
                    "SETACTIVE" => {
                        let name = unquote(rest);
                        for (n, _, active) in &mut state.lock().unwrap().scripts {
                            *active = *n == name;
                        }
                        "OK\r\n".to_owned()
                    }
                    "DELETESCRIPT" => {
                        let name = unquote(rest);
                        state.lock().unwrap().scripts.retain(|(n, _, _)| *n != name);
                        "OK\r\n".to_owned()
                    }
                    "LOGOUT" => {
                        let _ = write.write_all(b"OK \"bye\"\r\n").await;
                        return;
                    }
                    _ => "NO \"unknown command\"\r\n".to_owned(),
                };
                write.write_all(reply.as_bytes()).await.unwrap();
            }
        });
    }
}

async fn server(held: Server) -> (Endpoint, Arc<Mutex<Server>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let state = Arc::new(Mutex::new(held));
    tokio::spawn(serve(listener, state.clone()));
    (
        Endpoint {
            host: "127.0.0.1".to_owned(),
            port,
            tls: Tls::Plaintext,
        },
        state,
    )
}

fn auth() -> SieveAuth {
    SieveAuth {
        username: "me@example.test".to_owned(),
        credential: Credential::Password("s3cret".to_owned()),
    }
}

fn rule(position: u32, name: &str, filter: Filter, actions: Vec<RuleAction>) -> Rule {
    Rule {
        id: RuleId::generate(),
        account: ACCOUNT,
        name: name.to_owned(),
        position,
        state: RuleState::Enabled,
        filter,
        actions,
        after: AfterMatch::Continue,
    }
}

fn now() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap()
}

fn rules() -> Vec<Rule> {
    vec![
        rule(
            1,
            "Bills",
            Filter::From(TextMatch::Contains("bank.example".to_owned())),
            vec![RuleAction::MarkRead, RuleAction::File("Bills".to_owned())],
        ),
        rule(
            2,
            "Words",
            Filter::Text(TextMatch::Contains("invoice".to_owned())),
            vec![RuleAction::Archive],
        ),
    ]
}

#[tokio::test]
async fn a_push_installs_what_the_server_can_run_and_makes_it_active() {
    let (endpoint, state) = server(Server {
        scripts: vec![("old".to_owned(), "keep;".to_owned(), false)],
        ..Server::default()
    })
    .await;
    let (_tx, mut cancel) = tokio::sync::watch::channel(false);
    let pushed = push(
        &endpoint,
        &auth(),
        &rules(),
        None,
        &Places::default(),
        Takeover::Refuse,
        now(),
        &mut cancel,
    )
    .await
    .unwrap();

    assert!(
        matches!(
            pushed.outcome,
            SieveOutcome::Installed {
                displaced: None,
                ..
            }
        ),
        "{:?}",
        pushed.outcome
    );
    assert_eq!(pushed.compiled.mapped, vec!["Bills".to_owned()]);
    assert_eq!(
        pushed.compiled.local_only,
        vec![("Words".to_owned(), Unmappable::Clause("full text"))]
    );
    let held = state.lock().unwrap();
    let ours = held.scripts.iter().find(|(n, _, _)| n == "mailo").unwrap();
    assert_eq!(
        ours.1, pushed.compiled.script,
        "the server holds exactly what was written"
    );
    assert!(ours.2, "and runs it");
    assert!(ours.1.contains("fileinto \"Bills\";"), "{}", ours.1);
    assert!(
        held.scripts
            .iter()
            .any(|(n, _, active)| n == "old" && !active),
        "the old script is left as it was"
    );
}

#[tokio::test]
async fn another_active_script_is_left_alone_and_named() {
    let (endpoint, state) = server(Server {
        scripts: vec![("webmail".to_owned(), "keep;".to_owned(), true)],
        ..Server::default()
    })
    .await;
    let (_tx, mut cancel) = tokio::sync::watch::channel(false);
    let pushed = push(
        &endpoint,
        &auth(),
        &rules(),
        None,
        &Places::default(),
        Takeover::Refuse,
        now(),
        &mut cancel,
    )
    .await
    .unwrap();
    let SieveOutcome::Refused { active, .. } = &pushed.outcome else {
        panic!("{:?}", pushed.outcome);
    };
    assert_eq!(active, "webmail");
    let held = state.lock().unwrap();
    assert_eq!(
        held.scripts,
        vec![("webmail".to_owned(), "keep;".to_owned(), true)],
        "nothing was written"
    );
    assert!(!held.heard.iter().any(|l| l.starts_with("PUTSCRIPT")));
}

#[tokio::test]
async fn with_nothing_left_to_run_our_script_is_taken_down() {
    let (endpoint, state) = server(Server {
        scripts: vec![("mailo".to_owned(), "keep;".to_owned(), true)],
        ..Server::default()
    })
    .await;
    let only_local = vec![rules().remove(1)];
    let (_tx, mut cancel) = tokio::sync::watch::channel(false);
    let pushed = push(
        &endpoint,
        &auth(),
        &only_local,
        None,
        &Places::default(),
        Takeover::Refuse,
        now(),
        &mut cancel,
    )
    .await
    .unwrap();
    assert!(
        matches!(
            pushed.outcome,
            SieveOutcome::Removed {
                deleted: Deleted::Ours,
                ..
            }
        ),
        "{:?}",
        pushed.outcome
    );
    assert!(state.lock().unwrap().scripts.is_empty());
}

#[tokio::test]
async fn a_server_without_starttls_never_hears_a_credential() {
    let (mut endpoint, state) = server(Server::default()).await;
    endpoint.tls = Tls::StartTlsRequired;
    let (_tx, mut cancel) = tokio::sync::watch::channel(false);
    let err = push(
        &endpoint,
        &auth(),
        &rules(),
        None,
        &Places::default(),
        Takeover::Refuse,
        now(),
        &mut cancel,
    )
    .await
    .unwrap_err();
    assert!(
        matches!(
            err,
            RuntimeError::Proto(mail_proto::ProtoError::Unsupported(_))
        ),
        "{err}"
    );
    assert!(
        state.lock().unwrap().heard.is_empty(),
        "not a word was sent: {:?}",
        state.lock().unwrap().heard
    );
}
