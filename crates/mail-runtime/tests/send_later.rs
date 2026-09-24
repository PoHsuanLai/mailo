//! Send later, against a submission server on a real socket: held until its time, stamped with
//! the moment it leaves, and woken for by a watch that would otherwise sleep through it.
//!
//! The incoming side is a stub backend over a socket that never says anything, which is what an
//! IDLE with no new mail looks like from here: the only way out of the wait is the outbox.

use chrono::{DateTime, TimeDelta, TimeZone, Utc};
use mail_domain::*;
use mail_mime::posting;
use mail_proto::{Backend, IoNeed, IoReady, Progress, ProtoOutcome};
use mail_runtime::engine::Schedule;
use mail_runtime::{AccountEngine, MapSecrets, Secrets, Woke};
use mail_store::{SqliteStore, Store};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::sync::watch;

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));
const IDENTITY: IdentityId =
    IdentityId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000b1"));

/// When the message was written and frozen.
fn written() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 24, 17, 0, 0).unwrap()
}

/// When it was asked to leave.
fn leaves() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 25, 9, 0, 0).unwrap()
}

/// What the submission server was given: the `DATA` payloads, in order.
type Delivered = Arc<Mutex<Vec<String>>>;

/// EHLO, AUTH PLAIN, the envelope, DATA, QUIT — and a record of every message accepted.
async fn smtp() -> (u16, Delivered) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let delivered: Delivered = Arc::default();
    let seen = delivered.clone();
    tokio::spawn(async move {
        while let Ok((sock, _)) = listener.accept().await {
            let seen = seen.clone();
            tokio::spawn(async move {
                let (read, mut write) = sock.into_split();
                let mut lines = BufReader::new(read).lines();
                let _ = write.write_all(b"220 smtp.example ESMTP\r\n").await;
                let (mut data, mut body) = (false, String::new());
                while let Ok(Some(line)) = lines.next_line().await {
                    if data {
                        if line == "." {
                            data = false;
                            seen.lock().unwrap().push(std::mem::take(&mut body));
                            let _ = write.write_all(b"250 2.0.0 queued\r\n").await;
                        } else {
                            body.push_str(line.strip_prefix('.').unwrap_or(&line));
                            body.push_str("\r\n");
                        }
                        continue;
                    }
                    let upper = line.to_uppercase();
                    let reply: &[u8] = if upper.starts_with("EHLO") {
                        b"250-smtp.example\r\n250 AUTH PLAIN\r\n"
                    } else if upper.starts_with("AUTH") {
                        b"235 2.7.0 ok\r\n"
                    } else if upper.starts_with("DATA") {
                        data = true;
                        b"354 go on\r\n"
                    } else if upper.starts_with("QUIT") {
                        let _ = write.write_all(b"221 bye\r\n").await;
                        return;
                    } else {
                        b"250 2.1.0 ok\r\n"
                    };
                    if write.write_all(reply).await.is_err() {
                        return;
                    }
                }
            });
        }
    });
    (port, delivered)
}

/// A port that accepts and never says a word: an IDLE with nothing to report.
fn silent() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        let mut held = Vec::new();
        for sock in listener.incoming() {
            match sock {
                Ok(sock) => held.push(sock),
                Err(_) => return,
            }
        }
    });
    port
}

/// Parks every `Watch` on a read that never completes, and records being interrupted.
struct Parked {
    caps: AccountCaps,
    interrupted: Arc<Mutex<bool>>,
}

impl Backend for Parked {
    fn begin(&mut self, op: ProtoOp) -> Progress<ProtoOutcome> {
        match op {
            ProtoOp::Watch { .. } => Progress::Need(vec![IoNeed::Read]),
            _ => Progress::Done(ProtoOutcome::Applied),
        }
    }

    fn feed(&mut self, ready: IoReady) -> Progress<ProtoOutcome> {
        if ready == IoReady::Interrupt {
            *self.interrupted.lock().unwrap() = true;
            return Progress::Done(ProtoOutcome::Applied);
        }
        Progress::Need(vec![IoNeed::Read])
    }

    fn caps(&self) -> &AccountCaps {
        &self.caps
    }
}

fn caps(watch: WatchMode) -> AccountCaps {
    AccountCaps {
        labels: ServerLabels::LocalOnly,
        threads: ServerThreads::Jwz,
        watch,
        archive: ArchiveMeans::LocalOnly,
        folders: FolderRoles::default(),
        condstore: Condstore::Absent,
        move_ext: MoveExt::Absent,
        expunge: ExpungeMeans::Forbidden,
        top: Supported::Absent,
        pipelining: Supported::Absent,
        connections: ConnectionBudget { max: 1 },
        observed_at: written(),
    }
}

fn identity() -> Identity {
    Identity {
        id: IDENTITY,
        account: ACCOUNT,
        from: Address {
            name: None,
            email: "me@example.test".to_owned(),
        },
        reply_to: None,
        signature: None,
        default: IsDefault::Default,
    }
}

struct Fixture {
    store: Arc<SqliteStore>,
    engine: AccountEngine<Parked>,
    interrupted: Arc<Mutex<bool>>,
    delivered: Delivered,
    _dir: tempfile::TempDir,
}

async fn fixture(watch: WatchMode) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::in_memory(dir.path()).unwrap());
    {
        let db = store.connection();
        db.execute(
            "INSERT INTO accounts (id, address, plan, created_at)
             VALUES (?1, 'me@example.test', '{}', datetime('now'))",
            [ACCOUNT.to_string()],
        )
        .unwrap();
        db.execute(
            "INSERT INTO identities (id, account, from_name, from_email, is_default)
             VALUES (?1, ?2, NULL, 'me@example.test', '\"default\"')",
            [IDENTITY.to_string(), ACCOUNT.to_string()],
        )
        .unwrap();
    }
    let secrets = MapSecrets::default();
    secrets
        .put(
            &SecretKey {
                account: ACCOUNT,
                purpose: SecretPurpose::IncomingPassword,
            },
            &Credential::Password("s3cr3t".to_owned()),
        )
        .unwrap();
    let (smtp_port, delivered) = smtp().await;
    let plan = AccountPlan {
        address: "me@example.test".to_owned(),
        incoming: Incoming::Imap {
            host: "127.0.0.1".to_owned(),
            port: silent(),
            tls: Tls::Plaintext,
        },
        outgoing: Outgoing::Smtp {
            host: "127.0.0.1".to_owned(),
            port: smtp_port,
            tls: Tls::Plaintext,
        },
        auth: AuthPlan::Password {
            username: Username::SameAsAddress,
            sasl: vec![SaslMech::Plain],
        },
        identities: vec![identity()],
    };
    let interrupted = Arc::new(Mutex::new(false));
    let engine = AccountEngine::new(
        ACCOUNT,
        plan,
        Parked {
            caps: caps(watch),
            interrupted: interrupted.clone(),
        },
        store.clone(),
        Arc::new(secrets),
    )
    .with_schedule(Schedule {
        outbox: Duration::from_millis(50),
        ..Schedule::default()
    });
    Fixture {
        store,
        engine,
        interrupted,
        delivered,
        _dir: dir,
    }
}

/// Write a draft at [`written`], freeze it, and queue it to leave at `at` — what
/// `compose::queue` does with `Leaves::At`.
fn schedule(store: &SqliteStore, at: DateTime<Utc>) -> Draft {
    let draft = Draft {
        id: DraftId::generate(),
        account: ACCOUNT,
        identity: IDENTITY,
        to: vec![Address {
            name: None,
            email: "you@example.test".to_owned(),
        }],
        cc: vec![],
        bcc: vec![],
        subject: "first thing".to_owned(),
        in_reply_to: None,
        forward_of: None,
        text: "good morning".to_owned(),
        html: None,
        attachments: vec![],
        receipt: ReceiptRequest::Unrequested,
        openpgp: OpenPgp::None,
        smime: mail_domain::Smime::None,
        state: SendState::Editing,
        updated: written(),
    };
    store
        .apply(
            ACCOUNT,
            &Patch {
                id: ChangeId::generate(),
                changes: vec![Change::DraftUpsert(Box::new(draft.clone()))],
            },
        )
        .unwrap();
    let post = posting(&draft, &identity(), None, &[]).unwrap();
    let raw = store
        .blobs()
        .put(&store.connection(), &post.message)
        .unwrap();
    store
        .enqueue(
            ACCOUNT,
            RemoteIntent::Send {
                draft: draft.id,
                raw,
                mail_from: post.mail_from,
                rcpt_to: post.rcpt_to,
            },
            &Patch {
                id: ChangeId::generate(),
                changes: Vec::new(),
            },
            at,
        )
        .unwrap()
        .unwrap();
    store
        .set_send_state(draft.id, &SendState::Scheduled { at }, written())
        .unwrap();
    draft
}

fn date_header(message: &str) -> Option<&str> {
    message
        .lines()
        .take_while(|line| !line.is_empty())
        .find_map(|line| line.strip_prefix("Date: "))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_scheduled_send_waits_for_its_time_and_then_goes() {
    let mut it = fixture(WatchMode::Idle).await;
    let draft = schedule(&it.store, leaves());
    let (_tx, mut cancel) = watch::channel(false);

    let early = it
        .engine
        .drain_outbox(&mut cancel, leaves() - TimeDelta::seconds(1))
        .await
        .unwrap();
    assert_eq!(early.submitted, 0, "it left early");
    assert_eq!(early.still_queued, 1, "and must be reported as waiting");
    assert!(it.delivered.lock().unwrap().is_empty());
    assert_eq!(
        it.store.draft(draft.id).unwrap().state,
        SendState::Scheduled { at: leaves() }
    );

    let due = it.engine.drain_outbox(&mut cancel, leaves()).await.unwrap();
    assert_eq!(due.submitted, 1, "{due:?}");
    assert_eq!(it.delivered.lock().unwrap().len(), 1);
    assert_eq!(
        it.store.draft(draft.id).unwrap().state,
        SendState::Sent {
            at: leaves(),
            message: None
        }
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_date_it_carries_is_when_it_left_not_when_it_was_written() {
    let mut it = fixture(WatchMode::Idle).await;
    schedule(&it.store, leaves());
    let (_tx, mut cancel) = watch::channel(false);
    // Later than asked, as after a night with the lid shut.
    let left = leaves() + TimeDelta::minutes(40);
    it.engine.drain_outbox(&mut cancel, left).await.unwrap();

    let delivered = it.delivered.lock().unwrap();
    let message = delivered.first().expect("delivered");
    assert_eq!(
        date_header(message),
        Some(left.to_rfc2822().as_str()),
        "{message}"
    );
    assert_eq!(
        message.matches("\r\nDate: ").count() + usize::from(message.starts_with("Date: ")),
        1,
        "one Date, not two: {message}"
    );
    // Everything else is what was frozen: the body and the subject are untouched.
    assert!(message.contains("Subject: first thing"), "{message}");
    assert!(message.contains("good morning"), "{message}");
}

/// Wait with a poll interval far longer than the test, for a send due in a moment.
async fn woken_for(mode: WatchMode) -> (Result<Woke, mail_runtime::RuntimeError>, Duration, bool) {
    let mut it = fixture(mode).await;
    let soon = Utc::now() + TimeDelta::milliseconds(300);
    schedule(&it.store, soon);
    let (_tx, mut cancel) = watch::channel(false);
    let inbox = MailboxRef {
        account: ACCOUNT,
        path: "INBOX".to_owned(),
    };
    let started = Instant::now();
    let woke = tokio::time::timeout(
        Duration::from_secs(10),
        it.engine
            .wait(&inbox, &mut cancel, Duration::from_secs(300)),
    )
    .await
    .expect("the wait slept through a send that came due");
    let interrupted = *it.interrupted.lock().unwrap();
    (woke, started.elapsed(), interrupted)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_watch_parked_in_idle_comes_back_for_a_send_that_came_due() {
    let (woke, took, interrupted) = woken_for(WatchMode::Idle).await;
    assert_eq!(woke.unwrap(), Woke::Due);
    assert!(
        took >= Duration::from_millis(250),
        "woke before the send was due: {took:?}"
    );
    assert!(took < Duration::from_secs(3), "woke late: {took:?}");
    assert!(
        interrupted,
        "the IDLE was dropped rather than ended in protocol"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_watch_that_polls_comes_back_for_it_too() {
    let (woke, took, _) = woken_for(WatchMode::Poll {
        every: Duration::from_secs(300),
    })
    .await;
    assert_eq!(woke.unwrap(), Woke::Due);
    assert!(took >= Duration::from_millis(250), "{took:?}");
    assert!(took < Duration::from_secs(3), "{took:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_send_scheduled_while_the_watch_waits_is_noticed() {
    // Another process — `mailo send --at` in a terminal — queues it after the watch has already
    // looked at the outbox and gone to sleep.
    let mut it = fixture(WatchMode::Idle).await;
    let store = it.store.clone();
    let (_tx, mut cancel) = watch::channel(false);
    let inbox = MailboxRef {
        account: ACCOUNT,
        path: "INBOX".to_owned(),
    };
    let later = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(150)).await;
        schedule(&store, Utc::now() + TimeDelta::milliseconds(150));
    });
    let woke = tokio::time::timeout(
        Duration::from_secs(10),
        it.engine
            .wait(&inbox, &mut cancel, Duration::from_secs(300)),
    )
    .await
    .expect("a send queued during the wait was never noticed");
    later.await.unwrap();
    assert_eq!(woke.unwrap(), Woke::Due);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn nothing_due_means_the_watch_waits_out_its_interval() {
    let mut it = fixture(WatchMode::Poll {
        every: Duration::from_millis(200),
    })
    .await;
    // Due long after the interval: the interval ends the wait, not the send.
    schedule(&it.store, Utc::now() + TimeDelta::hours(1));
    let (_tx, mut cancel) = watch::channel(false);
    let inbox = MailboxRef {
        account: ACCOUNT,
        path: "INBOX".to_owned(),
    };
    let woke = it
        .engine
        .wait(&inbox, &mut cancel, Duration::from_millis(200))
        .await;
    assert_eq!(woke.unwrap(), Woke::Interval);
    assert!(it.delivered.lock().unwrap().is_empty());
}
