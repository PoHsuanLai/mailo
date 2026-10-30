//! The reusable watch: one connection held open, and what the server says as typed events.
//!
//! The scripted server is a thread on loopback speaking just enough IMAP to log in, select the
//! inbox and park in `IDLE`. What is under test is IMAP's: a refused login is a loss that asks for
//! a sign-in, and a followed folder is idled on a connection of its own. How a watch hears and
//! ends is `sync::live`'s own tests, against a protocol with no server.

use crate::support::{account, now};
use mail_core::sync::live::{self, Heard};
use mail_domain::*;
use mail_secrets::{AccountSecrets, ClientRegistry};
use mail_store::SqliteStore;
use porter_core::SecretText;
use porter_core::{AccountId, Credential, SecretKey, SecretPurpose};
use porter_secrets::MemorySecrets;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::watch;

/// What the scripted server does when the client asks to idle.
#[derive(Clone, Copy)]
enum Idling {
    /// Park and say nothing.
    Quiet,
    /// Park, saying nothing in the inbox and reporting one new message after a moment in the
    /// mailbox of this name.
    AnnounceIn(&'static str),
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
    let mut selected = String::new();
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
            selected = line.split('"').nth(1).unwrap_or("").to_owned();
            say(&format!(
                "* 0 EXISTS\r\n* OK [UIDVALIDITY 7] v\r\n* OK [UIDNEXT 1] n\r\n\
                 {tag} OK [READ-WRITE] done\r\n"
            ));
        } else if upper.starts_with("IDLE") {
            idle_tag = Some(tag);
            say("+ idling\r\n");
            let announces = match idling {
                Idling::AnnounceIn(mailbox) => selected == mailbox,
                Idling::Quiet => false,
            };
            if announces {
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

fn configured(port: u16, watch: WatchMode) -> Arc<SqliteStore> {
    let store = Arc::new(mail_store::testing::in_memory());
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
    mail_store::testing::seed_account_plan(&store, account(), "ada@example.test", &plan, None);
    mail_store::testing::seed_caps(&store, account(), &caps(watch), now()).unwrap();
    store
}

fn secrets() -> Arc<MemorySecrets> {
    let secrets = MemorySecrets::default();
    mail_secrets::block_on(secrets.put(
        &SecretKey {
            account: account(),
            purpose: SecretPurpose::IncomingPassword,
        },
        &Credential::Password(SecretText::new("s3cr3t-pass".to_owned())),
    ))
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
    listening_with_grace(store, cancel, Duration::from_millis(100))
}

/// [`listening`], with the grace after which a wait that has not failed counts as established.
fn listening_with_grace(
    store: Arc<SqliteStore>,
    cancel: watch::Receiver<bool>,
    grace: Duration,
) -> Listening {
    let heard = Arc::new(Mutex::new(Vec::new()));
    let said = heard.clone();
    let handle = std::thread::spawn(move || {
        crate::blocking::listen_with(
            store,
            secrets(),
            &ClientRegistry::default(),
            account(),
            live::Watch {
                cancel,
                hold: None,
                grace,
            },
            &|h| said.lock().unwrap().push(h),
        )
    });
    (handle, heard)
}

#[test]
fn a_refused_sign_in_is_a_loss_that_asks_for_one() {
    let port = server(Idling::Quiet, Login::Refuse);
    let store = configured(port, WatchMode::Idle);
    let (_cancel, rx) = watch::channel(false);
    // "Established" is a wait that outlived its grace, not a login that succeeded. With the
    // others' 100 ms, a loaded CI runner took longer than that to be refused and heard it.
    let (handle, heard) = listening_with_grace(store, rx, Duration::from_secs(30));

    let lost = handle.join().unwrap().expect_err("the server said no");
    assert_eq!(lost.retry, Retry::NeedsReauth, "{lost:?}");
    assert!(heard.lock().unwrap().is_empty(), "never called established");
}

#[test]
fn an_account_the_server_cannot_push_to_is_not_watched() {
    // No server at all: it must be refused before anything is connected to.
    let store = configured(
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
    let store = configured(1, WatchMode::Idle);
    assert_eq!(live::pushing(&store), [account()]);
}

/// Pushed only while it is checked automatically: an interval the person chose is polled at, and
/// manually holds no connection.
#[test]
fn an_idle_account_checked_on_its_own_terms_is_not_pushed_to() {
    use mail_core::offline::{Every, Kept};
    let store = configured(1, WatchMode::Idle);
    let table: [(Every, &[AccountId]); 3] = [
        (Every::Automatic, &[account()]),
        (Every::Minutes(15), &[]),
        (Every::Manually, &[]),
    ];
    for (every, want) in table {
        let kept = Kept::default().with_every(account(), every);
        assert_eq!(live::pushing_with(&store, &kept), want, "{every:?}");
    }
}

fn caps(watch: WatchMode) -> AccountCaps {
    AccountCaps {
        watch,
        connections: ConnectionBudget { max: 2 },
        ..crate::support::caps()
    }
}

/// Without `NOTIFY`, a followed folder besides the inbox is idled on a connection of its own
/// where the server's budget allows it, and its news is heard as mail.
#[test]
fn a_followed_folder_is_idled_on_its_own_connection_where_there_is_no_notify() {
    let port = server(Idling::AnnounceIn("Lists"), Login::Accept);
    let store = configured(port, WatchMode::Idle);
    follow(&store, "Lists");
    seed_budget(&store, 4);
    let (cancel, rx) = watch::channel(false);
    let (handle, heard) = listening(store, rx);

    // The first wait finds there is no NOTIFY and ends at once; the second idles the folder.
    let heard_mail = (0..100).any(|_| {
        std::thread::sleep(Duration::from_millis(50));
        heard.lock().unwrap().contains(&Heard::Mail)
    });
    assert!(heard_mail, "heard {:?}", heard.lock().unwrap());
    cancel.send(true).unwrap();
    assert_eq!(handle.join().unwrap(), Ok(()));
}

/// The same server and the same folder, with a budget that leaves no connection to spare: the
/// inbox alone is idled and the folder is left to the interval.
#[test]
fn a_budget_with_no_connection_to_spare_watches_the_inbox_alone() {
    let port = server(Idling::AnnounceIn("Lists"), Login::Accept);
    let store = configured(port, WatchMode::Idle);
    follow(&store, "Lists");
    let (cancel, rx) = watch::channel(false);
    let (handle, heard) = listening(store, rx);

    std::thread::sleep(Duration::from_millis(1500));
    assert!(
        !heard.lock().unwrap().contains(&Heard::Mail),
        "a folder was idled past the budget: {:?}",
        heard.lock().unwrap()
    );
    cancel.send(true).unwrap();
    assert_eq!(handle.join().unwrap(), Ok(()));
}

/// How many connections a watch may spare for folders, by what the server allows.
#[test]
fn the_extra_connections_follow_the_budget_and_stop_at_the_limit() {
    let cases = [
        (0u8, 0usize),
        (1, 0),
        (2, 0),
        (3, 1),
        (4, 2),
        (5, 2),
        (15, 2),
    ];
    for (max, want) in cases {
        let caps = AccountCaps {
            connections: ConnectionBudget { max },
            ..crate::support::caps()
        };
        assert_eq!(live::extra_idle(&caps), want, "a budget of {max}");
    }
}

fn follow(store: &SqliteStore, path: &str) {
    use mail_store::Store as _;
    store
        .put_folders(
            account(),
            vec![Folder {
                account: account(),
                path: path.to_owned(),
                delimiter: Some('/'),
                special: None,
                subscription: Subscription::Subscribed,
                holds: Holds::Mail,
            }],
        )
        .unwrap();
}

fn seed_budget(store: &SqliteStore, max: u8) {
    let caps = AccountCaps {
        connections: ConnectionBudget { max },
        ..caps(WatchMode::Idle)
    };
    mail_store::testing::seed_caps(store, account(), &caps, now()).unwrap();
}
