//! Sending a message over a real socket: outbox, engine, SMTP session, transport, server.
//!
//! The companion to `end_to_end.rs`, which proved mail could come in. Until this file, nothing
//! in the workspace had ever sent one: `SmtpBackend` was written, documented as the thing the
//! runtime routes `ProtoOp::Submit` to, unit-tested against a transcript — and unreachable,
//! because no code path put a submission in the outbox and the drain handed every operation to
//! the incoming backend.
//!
//! The server here records what it was actually told, because the two things most worth
//! checking are invisible from the client's side: that every recipient was named in the
//! envelope, and that the blind ones were named *nowhere else*.

use chrono::{DateTime, TimeZone, Utc};
use mail_domain::*;
use mail_mime::posting;
use mail_proto::backend::{Authenticate, Pop3Backend};
use mail_proto::{Pop3Command, Pop3Session};
use mail_runtime::{AccountEngine, MapSecrets, Secrets};
use mail_store::{SqliteStore, Store};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::sync::watch;

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));
const IDENTITY: IdentityId =
    IdentityId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000b1"));
const PASSWORD: &str = "s3cr3t";

fn now() -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000, 0).unwrap()
}

/// What the server was told, in order.
#[derive(Debug, Default)]
struct Transcript {
    commands: Vec<String>,
    /// The `DATA` payload, after the server un-stuffed it.
    body: String,
}

impl Transcript {
    fn rcpt_to(&self) -> Vec<String> {
        self.commands
            .iter()
            .filter_map(|c| c.strip_prefix("RCPT TO:<"))
            .filter_map(|c| c.strip_suffix('>'))
            .map(str::to_owned)
            .collect()
    }

    fn mail_from(&self) -> Option<String> {
        self.commands.iter().find_map(|c| {
            c.strip_prefix("MAIL FROM:<")
                .and_then(|rest| rest.split('>').next())
                .map(str::to_owned)
        })
    }
}

type Shared = Arc<Mutex<Transcript>>;

/// A submission server on a real socket: EHLO, AUTH PLAIN, the envelope, DATA, QUIT.
async fn serve(seen: Shared) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((sock, _)) = listener.accept().await {
            tokio::spawn(session(sock, seen.clone()));
        }
    });
    port
}

async fn session(sock: tokio::net::TcpStream, seen: Shared) {
    let (read, mut write) = sock.into_split();
    let mut lines = BufReader::new(read).lines();

    if write
        .write_all(b"220 smtp.example ESMTP ready\r\n")
        .await
        .is_err()
    {
        return;
    }

    let mut in_data = false;
    let mut body = String::new();
    while let Ok(Some(line)) = lines.next_line().await {
        if in_data {
            if line == "." {
                in_data = false;
                seen.lock().unwrap().body = std::mem::take(&mut body);
                if write
                    .write_all(b"250 2.0.0 Ok: queued as ABC123\r\n")
                    .await
                    .is_err()
                {
                    return;
                }
                continue;
            }
            // Un-stuff, exactly as a real server must: the client doubles a leading dot so it
            // cannot be mistaken for the terminator.
            let unstuffed = line.strip_prefix('.').unwrap_or(&line);
            body.push_str(unstuffed);
            body.push_str("\r\n");
            continue;
        }

        let upper = line.to_uppercase();
        // Recorded before the reply, and without the AUTH argument: that argument is the
        // password, and a test fixture that logs it is a test fixture that leaks it.
        let recorded = if upper.starts_with("AUTH ") {
            upper
                .split_whitespace()
                .take(2)
                .collect::<Vec<_>>()
                .join(" ")
        } else {
            line.clone()
        };
        seen.lock().unwrap().commands.push(recorded);

        let reply = if upper.starts_with("EHLO") {
            "250-smtp.example\r\n250-SIZE 35882577\r\n250-8BITMIME\r\n250 AUTH PLAIN LOGIN\r\n"
                .to_owned()
        } else if upper.starts_with("AUTH PLAIN") {
            "235 2.7.0 Accepted\r\n".to_owned()
        } else if upper.starts_with("MAIL FROM") || upper.starts_with("RCPT TO") {
            "250 2.1.0 Ok\r\n".to_owned()
        } else if upper.starts_with("DATA") {
            in_data = true;
            "354 End data with <CR><LF>.<CR><LF>\r\n".to_owned()
        } else if upper.starts_with("QUIT") {
            let _ = write.write_all(b"221 2.0.0 Bye\r\n").await;
            return;
        } else {
            "500 5.5.2 Unrecognized command\r\n".to_owned()
        };
        if write.write_all(reply.as_bytes()).await.is_err() {
            return;
        }
    }
}

fn identity() -> Identity {
    Identity {
        id: IDENTITY,
        account: ACCOUNT,
        from: Address {
            name: Some("Me".to_owned()),
            email: "me@example.test".to_owned(),
        },
        reply_to: None,
        signature: None,
        default: IsDefault::Default,
    }
}

fn plan(smtp_port: u16, pop_port: u16) -> AccountPlan {
    AccountPlan {
        address: "me@example.test".to_owned(),
        incoming: Incoming::Pop3 {
            host: "127.0.0.1".to_owned(),
            port: pop_port,
            tls: Tls::Plaintext,
            leave: LeaveOnServer::Keep,
        },
        outgoing: Outgoing::Smtp {
            host: "127.0.0.1".to_owned(),
            port: smtp_port,
            // Loopback to a server in this process. TLS has its own tests; using it here would
            // only be testing rustls.
            tls: Tls::Plaintext,
        },
        auth: AuthPlan::Password {
            username: Username::SameAsAddress,
            sasl: vec![SaslMech::Plain],
        },
        identities: vec![identity()],
    }
}

fn caps() -> AccountCaps {
    AccountCaps {
        labels: ServerLabels::LocalOnly,
        threads: ServerThreads::Jwz,
        watch: WatchMode::Poll {
            every: std::time::Duration::from_secs(300),
        },
        archive: ArchiveMeans::LocalOnly,
        folders: FolderRoles::default(),
        condstore: Condstore::Absent,
        move_ext: MoveExt::Absent,
        expunge: ExpungeMeans::Forbidden,
        top: Supported::Yes,
        pipelining: Supported::Absent,
        connections: ConnectionBudget { max: 1 },
        observed_at: now(),
    }
}

/// A draft to three people, one of them blind.
fn draft() -> Draft {
    Draft {
        id: DraftId::generate(),
        account: ACCOUNT,
        identity: IDENTITY,
        to: vec![Address {
            name: Some("Bea".to_owned()),
            email: "bea@example.test".to_owned(),
        }],
        cc: vec![Address {
            name: None,
            email: "cara@example.test".to_owned(),
        }],
        bcc: vec![Address {
            name: None,
            email: "dee@example.test".to_owned(),
        }],
        subject: "lunch on friday".to_owned(),
        in_reply_to: None,
        forward_of: None,
        text: "Shall we say one o'clock?\r\n.a line that needs stuffing\r\n".to_owned(),
        html: None,
        attachments: Vec::new(),
        state: SendState::Editing,
        updated: now(),
    }
}

struct Sending {
    store: Arc<SqliteStore>,
    engine: AccountEngine<Pop3Backend>,
    draft: Draft,
    _dir: tempfile::TempDir,
}

/// Everything up to the moment the user presses send.
fn compose(smtp_port: u16) -> Sending {
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
             VALUES (?1, ?2, 'Me', 'me@example.test', '\"default\"')",
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
            &Credential::Password(PASSWORD.to_owned()),
        )
        .unwrap();

    let draft = draft();
    store
        .apply(
            ACCOUNT,
            &Patch {
                id: ChangeId::generate(),
                changes: vec![Change::DraftUpsert(Box::new(draft.clone()))],
            },
        )
        .unwrap();

    // Pressing send: build the bytes and the envelope together, freeze the bytes in the blob
    // store, and queue the submission. Everything after this point is the outbox's problem,
    // which is what makes sending survive a restart.
    let post = posting(&draft, &identity(), None, &[]).expect("the draft has recipients");
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
                mail_from: post.mail_from.clone(),
                rcpt_to: post.rcpt_to.clone(),
            },
            &Patch {
                id: ChangeId::generate(),
                changes: Vec::new(),
            },
            now(),
        )
        .unwrap()
        .expect("a submission is always queued");
    store
        .set_send_state(draft.id, &SendState::Queued, now())
        .unwrap();

    // The incoming backend, which must never see the submission. Its port is closed: if the
    // engine routes a Submit here, the test fails by connection refused rather than by
    // accident.
    let backend = Pop3Backend::new(
        ACCOUNT,
        caps(),
        Box::new(|auth, commands| {
            let mut all = Vec::new();
            if auth == Authenticate::First {
                all.push(Pop3Command::AuthPlain);
            }
            all.extend(commands);
            Pop3Session::new("me@example.test", PASSWORD, all)
        }),
    );
    let engine = AccountEngine::new(
        ACCOUNT,
        plan(smtp_port, 1),
        backend,
        store.clone(),
        Arc::new(secrets),
    );
    Sending {
        store,
        engine,
        draft,
        _dir: dir,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_queued_message_reaches_the_submission_server() {
    let seen: Shared = Arc::new(Mutex::new(Transcript::default()));
    let port = serve(seen.clone()).await;
    let mut it = compose(port);
    let (_tx, mut cancel) = watch::channel(false);

    let report = it
        .engine
        .drain_outbox(&mut cancel, now())
        .await
        .expect("the drain reaches the submission server");

    assert_eq!(report.submitted, 1, "nothing was submitted: {report:?}");
    assert_eq!(report.outbox_settled, 1);
    assert_eq!(
        report.still_queued, 0,
        "a delivered message must not still be reported as waiting"
    );
    assert!(
        report.needs_attention.is_empty(),
        "{:?}",
        report.needs_attention
    );

    let seen = seen.lock().unwrap();
    assert_eq!(seen.mail_from().as_deref(), Some("me@example.test"));
    assert!(
        seen.commands.iter().any(|c| c.starts_with("EHLO")),
        "no EHLO: {:?}",
        seen.commands
    );
    assert!(
        seen.commands.iter().any(|c| c == "AUTH PLAIN"),
        "the session did not authenticate: {:?}",
        seen.commands
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_blind_recipient_is_delivered_to_and_named_nowhere_else() {
    // FINDINGS F37, end to end. Both halves fail silently and in opposite directions, so both
    // are asserted against what the server actually received.
    let seen: Shared = Arc::new(Mutex::new(Transcript::default()));
    let port = serve(seen.clone()).await;
    let mut it = compose(port);
    let (_tx, mut cancel) = watch::channel(false);

    it.engine.drain_outbox(&mut cancel, now()).await.unwrap();

    let seen = seen.lock().unwrap();
    let mut rcpt = seen.rcpt_to();
    rcpt.sort();
    assert_eq!(
        rcpt,
        vec![
            "bea@example.test".to_owned(),
            "cara@example.test".to_owned(),
            "dee@example.test".to_owned(),
        ],
        "the envelope must carry every recipient, blind ones included"
    );

    let body = seen.body.to_lowercase();
    assert!(
        !body.contains("bcc:"),
        "a Bcc header reached the server:\n{}",
        seen.body
    );
    assert!(
        !body.contains("dee@example.test"),
        "the blind address appears in the transmitted message:\n{}",
        seen.body
    );
    // The message really did carry its visible recipients, so the absence above is the header
    // being omitted rather than the whole envelope being empty.
    assert!(body.contains("bea@example.test"), "{}", seen.body);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_message_survives_the_wire_byte_for_byte() {
    // The body opens a line with a dot, which is the terminator. The client stuffs it and the
    // server un-stuffs it; get either wrong and the message is silently truncated there.
    let seen: Shared = Arc::new(Mutex::new(Transcript::default()));
    let port = serve(seen.clone()).await;
    let mut it = compose(port);
    let (_tx, mut cancel) = watch::channel(false);

    it.engine.drain_outbox(&mut cancel, now()).await.unwrap();

    let seen = seen.lock().unwrap();
    assert!(
        seen.body.contains(".a line that needs stuffing"),
        "the dotted line did not survive:\n{}",
        seen.body
    );
    assert!(
        seen.body.contains("Shall we say one o'clock?"),
        "the body was truncated at the dotted line:\n{}",
        seen.body
    );
    assert!(
        seen.body
            .to_lowercase()
            .contains("subject: lunch on friday"),
        "{}",
        seen.body
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_draft_ends_up_marked_sent() {
    let seen: Shared = Arc::new(Mutex::new(Transcript::default()));
    let port = serve(seen.clone()).await;
    let mut it = compose(port);
    let (_tx, mut cancel) = watch::channel(false);

    assert_eq!(
        it.store.draft(it.draft.id).unwrap().state,
        SendState::Queued,
        "precondition"
    );
    it.engine.drain_outbox(&mut cancel, now()).await.unwrap();

    match it.store.draft(it.draft.id).unwrap().state {
        SendState::Sent { at, message } => {
            assert_eq!(at, now());
            // SMTP reports acceptance, not where a copy was filed.
            assert_eq!(message, None);
        }
        other => panic!("draft did not reach Sent: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_submission_that_cannot_connect_is_retried_not_lost() {
    // Port 1 on loopback refuses. A send that failed must stay queued: the user pressed send,
    // and a message that vanishes because the laptop was on a train is the worst outcome here.
    let mut it = compose(1);
    let (_tx, mut cancel) = watch::channel(false);

    let report = it.engine.drain_outbox(&mut cancel, now()).await.unwrap();
    assert_eq!(report.submitted, 0);
    assert_eq!(report.outbox_settled, 0, "nothing was settled");
    // And the pass says so. A refused connection is a retry rather than trouble, so it never
    // reaches `needs_attention` — which meant someone who ran `send` and then `sync` read
    // "0 sent" and had no reason to think their mail was still sitting here.
    assert_eq!(
        report.still_queued, 1,
        "a message that did not go must be counted as still waiting"
    );

    let still_queued = it
        .store
        .outbox_due(ACCOUNT, now() + chrono::TimeDelta::try_hours(1).unwrap())
        .unwrap();
    assert_eq!(still_queued.len(), 1, "the submission was dropped");

    match it.store.draft(it.draft.id).unwrap().state {
        SendState::Failed { retry, .. } => assert!(
            !matches!(retry, Retry::Fatal(_)),
            "a refused connection must not be fatal: {retry:?}"
        ),
        other => panic!("expected Failed with a retry, got {other:?}"),
    }
}
