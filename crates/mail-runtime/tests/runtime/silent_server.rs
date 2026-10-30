//! A server that goes quiet and never says so: the 40 hours an IDLE account sat on a socket a NAT
//! had forgotten, with no error, no reconnect and no pass.
//!
//! Each connection is scripted in memory (`AccountEngine::with_dial`), so the clock can be paused and half an hour of silence costs nothing. Two promises are checked: a held
//! IDLE is ended in protocol and issued again before RFC 2177's twenty-nine minutes, and an
//! exchange that hears nothing at all ends as [`RuntimeError::Stalled`], which is answered by a
//! new connection.

use crate::fixture::now;
use mail_domain::id::account_id_from_uuid;
use mail_domain::*;
use mail_net::testing::memory_transport;
use mail_proto::backend::ImapBackend;
use mail_proto::{ImapAuth, ImapCommand, ImapSession};
use mail_runtime::engine::Dial;
use mail_runtime::testing::memory_secrets;
use mail_runtime::{AccountEngine, IDLE_REARM, RuntimeError, Woke};
use mail_store::SqliteStore;
use porter_core::stream::{ByteStream, DuplexEnd};
use porter_core::{AccountId, Credential, SecretText};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::watch;
use tokio::time::Instant;

fn acct_account() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000c7"))
}

fn inbox() -> MailboxRef {
    MailboxRef {
        account: acct_account(),
        path: "INBOX".to_owned(),
    }
}

/// How one scripted connection behaves.
#[derive(Debug, Clone, Copy)]
enum Server {
    /// Greets, then never says another word.
    Silent,
    /// Answers everything up to `IDLE`, acknowledges it, and then says nothing.
    QuietAfterIdle,
}

/// Every line a client sent, with when, across connections.
type Heard = Arc<Mutex<Vec<(Instant, String)>>>;

/// The incoming server, as far as the engine reaches it: a scripted connection per dial.
#[derive(Debug, Default)]
struct Script {
    next: Mutex<VecDeque<Server>>,
    heard: Heard,
    opened: Mutex<usize>,
}

impl Script {
    fn serving(servers: &[Server]) -> Arc<Script> {
        Arc::new(Script {
            next: Mutex::new(servers.iter().copied().collect()),
            ..Script::default()
        })
    }

    fn opened(&self) -> usize {
        *self.opened.lock().unwrap()
    }

    /// When the client sent a line starting with `verb` (after its tag), in order.
    fn sent(&self, verb: &str) -> Vec<Instant> {
        self.heard
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, line)| {
                let words: Vec<&str> = line.split_whitespace().collect();
                words.first().is_some_and(|w| w.eq_ignore_ascii_case(verb))
                    || words.get(1).is_some_and(|w| w.eq_ignore_ascii_case(verb))
            })
            .map(|(at, _)| *at)
            .collect()
    }
}

/// A dial that answers each connection with the next of `script`'s servers.
fn dial(script: Arc<Script>) -> Dial {
    Arc::new(move || {
        *script.opened.lock().unwrap() += 1;
        let server = script
            .next
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Server::QuietAfterIdle);
        let (transport, end) = memory_transport();
        tokio::spawn(serve(end, server, script.heard.clone()));
        Box::pin(async move { Ok(transport) })
    })
}

/// Play `server` on one connection, keeping every line the client sends.
async fn serve(mut end: DuplexEnd, server: Server, heard: Heard) {
    if ByteStream::write_all(&mut end, b"* OK [CAPABILITY IMAP4rev1 IDLE] ready\r\n")
        .await
        .is_err()
    {
        return;
    }
    let (mut pending, mut buf) = (Vec::new(), vec![0u8; 4096]);
    loop {
        let n = match ByteStream::read(&mut end, &mut buf).await {
            Ok(0) | Err(_) => return,
            Ok(n) => n,
        };
        pending.extend_from_slice(&buf[..n]);
        while let Some(at) = pending.windows(2).position(|w| w == b"\r\n") {
            let line: Vec<u8> = pending.drain(..at + 2).collect();
            let line = String::from_utf8_lossy(&line).trim_end().to_owned();
            heard.lock().unwrap().push((Instant::now(), line.clone()));
            let reply = match server {
                Server::Silent => continue,
                Server::QuietAfterIdle => answer(&line),
            };
            if !reply.is_empty()
                && ByteStream::write_all(&mut end, reply.as_bytes())
                    .await
                    .is_err()
            {
                return;
            }
        }
    }
}

/// What a server that is fine until IDLE says to `line`.
fn answer(line: &str) -> String {
    let mut words = line.split_whitespace();
    let tag = words.next().unwrap_or("*");
    match words.next().map(str::to_uppercase).as_deref() {
        // Acknowledged, and then nothing: the parked case.
        Some("IDLE") => "+ idling\r\n".to_owned(),
        Some("EXAMINE") | Some("SELECT") => {
            format!("* 2 EXISTS\r\n* OK [UIDVALIDITY 7] ok\r\n{tag} OK [READ-ONLY] done\r\n")
        }
        // `DONE` has no tag of its own; what answers it is never read once a wait is over.
        _ if line.eq_ignore_ascii_case("DONE") => String::new(),
        _ => format!("{tag} OK done\r\n"),
    }
}

fn caps() -> AccountCaps {
    AccountCaps {
        labels: ServerLabels::LocalOnly,
        threads: ServerThreads::Jwz,
        watch: WatchMode::Idle,
        archive: ArchiveMeans::MoveToFolder("Archive".to_owned()),
        folders: FolderRoles::default(),
        condstore: Condstore::Absent,
        move_ext: MoveExt::Absent,
        expunge: ExpungeMeans::Forbidden,
        top: Supported::Absent,
        pipelining: Supported::Absent,
        connections: ConnectionBudget { max: 1 },
        observed_at: now(),
    }
}

struct Fixture {
    engine: AccountEngine<ImapBackend>,
    _dir: tempfile::TempDir,
}

/// An IMAP account whose every connection is one of `script`'s. The connection is in memory and
/// asks for no sign-in, so the session sends none first.
fn engine(script: Arc<Script>) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::open(dir.path().join("mail.db"), dir.path()).unwrap());
    mail_store::testing::seed_account(&store, acct_account(), "me@example.test");
    let plan = AccountPlan {
        address: "me@example.test".to_owned(),
        incoming: Incoming::Imap {
            host: "imap.example.test".to_owned(),
            port: 993,
            tls: Tls::Implicit,
        },
        outgoing: Outgoing::Nowhere,
        auth: AuthPlan::Password {
            username: Username::SameAsAddress,
            sasl: vec![SaslMech::Plain],
        },
        identities: Vec::new(),
    };
    let auth = ImapAuth {
        username: "me@example.test".to_owned(),
        credential: Credential::Password(SecretText::new(String::new())),
        sasl: Vec::new(),
    };
    let backend = ImapBackend::new(
        acct_account(),
        caps(),
        Box::new(move |_authenticate, commands: Vec<ImapCommand>| {
            ImapSession::new(auth.clone(), commands)
        }),
    );
    let engine = AccountEngine::new(acct_account(), plan, backend, store, memory_secrets())
        .with_dial(dial(script));
    Fixture { engine, _dir: dir }
}

/// RFC 2177: the server may drop a client idle for thirty minutes, and the client re-issues
/// IDLE within twenty-nine. A wait that nothing interrupts ends the IDLE itself, in protocol,
/// at [`IDLE_REARM`], and the next wait issues it again on a connection of its own.
#[tokio::test(start_paused = true)]
async fn a_held_idle_is_ended_and_issued_again_before_twenty_nine_minutes() {
    assert!(IDLE_REARM < Duration::from_secs(29 * 60), "RFC 2177");
    let script = Script::serving(&[Server::QuietAfterIdle, Server::QuietAfterIdle]);
    let mut it = engine(script.clone());
    let (_tx, mut cancel) = watch::channel(false);
    let began = Instant::now();

    let woke = it
        .engine
        .wait(&inbox(), &mut cancel, Duration::from_secs(30))
        .await
        .expect("a re-arm is not a failure");
    // A look at the account follows, which is what a quiet wait has always ended in.
    assert_eq!(woke, Woke::Interval);
    // The server's task reads the `DONE` once this one lets it run; no time passes meanwhile.
    tokio::time::sleep(Duration::from_millis(1)).await;

    let idles = script.sent("IDLE");
    let dones = script.sent("DONE");
    assert_eq!((idles.len(), dones.len()), (1, 1), "{:?}", script.heard);
    let held = dones[0] - idles[0];
    assert!(
        held < Duration::from_secs(29 * 60),
        "IDLE was held for {held:?}"
    );
    assert!(
        dones[0] - began >= IDLE_REARM,
        "ended early: {:?}",
        dones[0] - began
    );

    // The next wait parks again, on a new connection.
    let again = tokio::time::timeout(
        Duration::from_secs(60),
        it.engine
            .wait(&inbox(), &mut cancel, Duration::from_secs(30)),
    )
    .await;
    assert!(again.is_err(), "the second IDLE ended by itself: {again:?}");
    assert_eq!(script.sent("IDLE").len(), 2, "IDLE was not issued again");
    assert_eq!(script.opened(), 2);
}

/// A server that greets and then never answers: within [`mail_runtime::drive::QUIET`] the pass
/// ends as [`RuntimeError::Stalled`], a failure that waits a little and connects again, and the
/// next attempt opens a connection of its own rather than waiting on the dead one.
#[tokio::test(start_paused = true)]
async fn a_dead_read_is_stalled_and_the_next_attempt_connects_again() {
    let script = Script::serving(&[Server::Silent, Server::QuietAfterIdle]);
    let mut it = engine(script.clone());
    let (_tx, mut cancel) = watch::channel(false);
    let began = Instant::now();

    let err = it
        .engine
        .sync(&inbox(), &mut cancel, now(), 200)
        .await
        .expect_err("a server that never answers does not complete a pass");
    assert!(matches!(err, RuntimeError::Stalled { .. }), "{err:?}");
    assert!(
        began.elapsed() <= mail_runtime::drive::QUIET + Duration::from_secs(1),
        "stalled only after {:?}",
        began.elapsed()
    );
    // Not a sign-in to ask for, not a fatal stop: a reconnect, soon.
    let retry = err.retry();
    assert!(matches!(retry, Retry::After(_)), "{retry:?}");
    let next = mail_runtime::schedule::reconnect(1, &retry, mail_runtime::fetch::RETRY_NOW);
    assert!(next.wait <= Duration::from_secs(60), "{next:?}");

    // And the next attempt is a new connection, which answers.
    let parked = tokio::time::timeout(
        Duration::from_secs(60),
        it.engine
            .wait(&inbox(), &mut cancel, Duration::from_secs(30)),
    )
    .await;
    assert!(parked.is_err(), "{parked:?}");
    assert_eq!(script.opened(), 2);
    assert_eq!(script.sent("IDLE").len(), 1);
}
