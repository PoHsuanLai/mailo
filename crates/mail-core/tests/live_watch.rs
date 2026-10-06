//! The reusable watch: one connection held open, and what the server says as typed events.
//!
//! The scripted server is a thread on loopback speaking just enough IMAP to log in, select the
//! inbox and park in `IDLE`. What is under test is that a `* n EXISTS` it sends while idling comes
//! out as [`Heard::Mail`], that the connection is called established only once it has been held,
//! and that the watch ends in the ways a caller has to act on.

use chrono::{DateTime, TimeZone, Utc};
use mail_core::sync::live::{self, Heard};
use mail_domain::id::account_id_from_uuid;
use mail_domain::*;
use mail_runtime::{MapSecrets, OAuthRegistry, Secrets};
use mail_store::SqliteStore;
use porter_core::SecretText;
use porter_core::{AccountId, Credential, SecretKey, SecretPurpose};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::watch;

fn acct_account() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000b1"))
}

fn now() -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000, 0).unwrap()
}

/// What the scripted server does when the client asks to idle.
#[derive(Clone, Copy)]
enum Idling {
    /// Park, then report one new message after a moment.
    Announce,
    /// Park and say nothing.
    Quiet,
}

/// What it does at the login.
#[derive(Clone, Copy)]
enum Login {
    Accept,
    Refuse,
}

/// A server on loopback; returns its port. One thread per connection, for as long as the test
/// lives.
fn server(idling: Idling, login: Login) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for sock in listener.incoming() {
            let Ok(sock) = sock else { continue };
            std::thread::spawn(move || session(sock, idling, login));
        }
    });
    port
}

fn session(sock: std::net::TcpStream, idling: Idling, login: Login) {
    let mut out = sock.try_clone().unwrap();
    let mut say = |text: &str| {
        let _ = out.write_all(text.as_bytes());
    };
    say("* OK [CAPABILITY IMAP4rev1 IDLE] ready\r\n");
    let mut idle_tag: Option<String> = None;
    for line in BufReader::new(sock).lines() {
        let Ok(line) = line else { return };
        let mut parts = line.splitn(2, ' ');
        let tag = parts.next().unwrap_or("*").to_owned();
        let upper = parts.next().unwrap_or("").to_uppercase();
        if tag.eq_ignore_ascii_case("DONE") {
            if let Some(idle) = idle_tag.take() {
                say(&format!("{idle} OK idle done\r\n"));
            }
        } else if upper.starts_with("CAPABILITY") {
            say(&format!("* CAPABILITY IMAP4rev1 IDLE\r\n{tag} OK done\r\n"));
        } else if upper.starts_with("LOGIN") || upper.starts_with("AUTHENTICATE") {
            match login {
                Login::Accept => say(&format!("{tag} OK logged in\r\n")),
                Login::Refuse => say(&format!("{tag} NO [AUTHENTICATIONFAILED] refused\r\n")),
            }
        } else if upper.starts_with("SELECT") || upper.starts_with("EXAMINE") {
            say(&format!(
                "* 0 EXISTS\r\n* OK [UIDVALIDITY 7] v\r\n* OK [UIDNEXT 1] n\r\n\
                 {tag} OK [READ-WRITE] done\r\n"
            ));
        } else if upper.starts_with("IDLE") {
            idle_tag = Some(tag);
            say("+ idling\r\n");
            if matches!(idling, Idling::Announce) {
                std::thread::sleep(Duration::from_millis(300));
                say("* 1 EXISTS\r\n");
            }
        } else if upper.starts_with("LOGOUT") {
            say(&format!("* BYE\r\n{tag} OK done\r\n"));
            return;
        } else {
            say(&format!("{tag} OK done\r\n"));
        }
    }
}

fn caps(watch: WatchMode) -> AccountCaps {
    AccountCaps {
        labels: ServerLabels::LocalOnly,
        threads: ServerThreads::Jwz,
        watch,
        archive: ArchiveMeans::LocalOnly,
        folders: FolderRoles(Vec::new()),
        condstore: Condstore::Absent,
        move_ext: MoveExt::Absent,
        expunge: ExpungeMeans::Forbidden,
        top: Supported::Absent,
        pipelining: Supported::Absent,
        connections: ConnectionBudget { max: 2 },
        observed_at: now(),
    }
}

fn account(port: u16, watch: WatchMode) -> (Arc<SqliteStore>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::in_memory(dir.path()).unwrap());
    let plan = AccountPlan {
        address: "ada@example.test".to_owned(),
        incoming: Incoming::Imap {
            host: "127.0.0.1".to_owned(),
            port,
            tls: Tls::Plaintext,
        },
        outgoing: Outgoing::Smtp {
            host: "127.0.0.1".to_owned(),
            port: 1,
            tls: Tls::Plaintext,
        },
        auth: AuthPlan::Password {
            username: Username::SameAsAddress,
            sasl: vec![SaslMech::Plain],
        },
        identities: Vec::new(),
    };
    let db = store.connection();
    db.execute(
        "INSERT INTO accounts (id, address, plan, created_at)
         VALUES (?1, 'ada@example.test', ?2, datetime('now'))",
        rusqlite::params![
            acct_account().to_string(),
            serde_json::to_string(&plan).unwrap()
        ],
    )
    .unwrap();
    db.execute(
        "INSERT INTO account_caps (account, caps, observed_at) VALUES (?1, ?2, ?3)",
        rusqlite::params![
            acct_account().to_string(),
            serde_json::to_string(&caps(watch)).unwrap(),
            now().to_rfc3339()
        ],
    )
    .unwrap();
    drop(db);
    (store, dir)
}

fn secrets() -> Arc<MapSecrets> {
    let secrets = MapSecrets::default();
    secrets
        .put(
            &SecretKey {
                account: acct_account(),
                purpose: SecretPurpose::IncomingPassword,
            },
            &Credential::Password(SecretText::new("s3cr3t-pass".to_owned())),
        )
        .unwrap();
    Arc::new(secrets)
}

/// A watch running on a thread, and what it has heard so far.
type Listening = (
    std::thread::JoinHandle<Result<(), live::Lost>>,
    Arc<Mutex<Vec<Heard>>>,
);

/// Run the watch on a thread, as the window does; returns what it heard and how it ended.
fn listening(store: Arc<SqliteStore>, cancel: watch::Receiver<bool>) -> Listening {
    let heard = Arc::new(Mutex::new(Vec::new()));
    let said = heard.clone();
    let handle = std::thread::spawn(move || {
        live::listen_with(
            store,
            secrets(),
            &OAuthRegistry::default(),
            acct_account(),
            cancel,
            None,
            Duration::from_millis(100),
            &|h| said.lock().unwrap().push(h),
        )
    });
    (handle, heard)
}

/// Wait up to five seconds for `heard` to hold at least `n` events.
fn until(heard: &Arc<Mutex<Vec<Heard>>>, n: usize) -> Vec<Heard> {
    for _ in 0..100 {
        let now = heard.lock().unwrap().clone();
        if now.len() >= n {
            return now;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    heard.lock().unwrap().clone()
}

#[test]
fn a_server_announcing_mail_while_idling_is_heard_as_mail() {
    let port = server(Idling::Announce, Login::Accept);
    let (store, _dir) = account(port, WatchMode::Idle);
    let (cancel, rx) = watch::channel(false);
    let (handle, heard) = listening(store, rx);

    let all = until(&heard, 2);
    assert_eq!(
        all.get(..2),
        Some(&[Heard::Established, Heard::Mail][..]),
        "held, then told: {all:?}"
    );
    cancel.send(true).unwrap();
    assert_eq!(handle.join().unwrap(), Ok(()), "cancelling is not a loss");
}

#[test]
fn a_quiet_server_is_held_and_released_on_cancel() {
    let port = server(Idling::Quiet, Login::Accept);
    let (store, _dir) = account(port, WatchMode::Idle);
    let (cancel, rx) = watch::channel(false);
    let (handle, heard) = listening(store, rx);

    assert_eq!(until(&heard, 1), [Heard::Established]);
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(*heard.lock().unwrap(), [Heard::Established], "nothing said");
    cancel.send(true).unwrap();
    assert_eq!(handle.join().unwrap(), Ok(()));
}

#[test]
fn a_refused_sign_in_is_a_loss_that_asks_for_one() {
    let port = server(Idling::Quiet, Login::Refuse);
    let (store, _dir) = account(port, WatchMode::Idle);
    let (_cancel, rx) = watch::channel(false);
    let (handle, heard) = listening(store, rx);

    let lost = handle.join().unwrap().expect_err("the server said no");
    assert_eq!(lost.retry, Retry::NeedsReauth, "{lost:?}");
    assert!(heard.lock().unwrap().is_empty(), "never called established");
}

#[test]
fn an_account_the_server_cannot_push_to_is_not_watched() {
    // No server at all: it must be refused before anything is connected to.
    let (store, _dir) = account(
        1,
        WatchMode::Poll {
            every: Duration::from_secs(300),
        },
    );
    let (_cancel, rx) = watch::channel(false);
    let (handle, heard) = listening(store.clone(), rx);
    let lost = handle.join().unwrap().expect_err("nothing to wait on");
    assert!(matches!(lost.retry, Retry::Fatal(_)), "{lost:?}");
    assert!(heard.lock().unwrap().is_empty());
    assert!(live::pushing(&store).is_empty());
}

#[test]
fn an_idle_account_is_one_that_pushes() {
    let (store, _dir) = account(1, WatchMode::Idle);
    assert_eq!(live::pushing(&store), [acct_account()]);
}
