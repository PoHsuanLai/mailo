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
use mail_domain::id::account_id_from_uuid;
use mail_domain::*;
use mail_mime::posting;
use mail_proto::backend::{Authenticate, Pop3Backend};
use mail_proto::{Pop3Command, Pop3Session};
use mail_runtime::{AccountEngine, AccountSecrets};
use mail_store::{SqliteStore, Store};
use porter_core::SecretText;
use porter_core::{AccountId, Credential, SecretKey, SecretPurpose};
use porter_secrets::MemorySecrets;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::sync::watch;

use crate::relay;

fn acct_account() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"))
}
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
        account: acct_account(),
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
        account: acct_account(),
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
        receipt: ReceiptRequest::Unrequested,
        openpgp: OpenPgp::None,
        smime: mail_domain::Smime::None,
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
    compose_as(smtp_port, None)
}

/// [`compose`], for an account that is the desktop's accountd's when `relays` is given: no
/// password of its own, and the submission goes through porter's relay.
fn compose_as(smtp_port: u16, relays: Option<Arc<relay::Relays>>) -> Sending {
    // The incoming port is closed: if the engine routes a Submit to POP3, the test fails by
    // connection refused rather than by accident.
    let it = open(smtp_port, 1, relays);
    queue(&it.store, &it.draft, None);
    it
}

/// Pressing send on `draft`, answering `parent` when it is a reply: build the bytes and the
/// envelope together, freeze the bytes in the blob store, and queue the submission. Everything
/// after this point is the outbox's problem, which is what makes sending survive a restart.
fn queue(store: &SqliteStore, draft: &Draft, parent: Option<&Message>) {
    store
        .apply(
            acct_account(),
            &Patch {
                id: ChangeId::generate(),
                changes: vec![Change::DraftUpsert(Box::new(draft.clone()))],
            },
        )
        .unwrap();
    let post = posting(draft, &identity(), parent, &[]).expect("the draft has recipients");
    let raw = store
        .blobs()
        .put(&store.connection(), &post.message)
        .unwrap();
    store
        .enqueue(
            acct_account(),
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
}

/// The account, its store and its engine, with a draft written and nothing queued yet.
fn open(smtp_port: u16, pop_port: u16, relays: Option<Arc<relay::Relays>>) -> Sending {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::in_memory(dir.path()).unwrap());
    {
        let db = store.connection();
        db.execute(
            "INSERT INTO accounts (id, address, plan, created_at)
             VALUES (?1, 'me@example.test', '{}', datetime('now'))",
            [acct_account().to_string()],
        )
        .unwrap();
        db.execute(
            "INSERT INTO identities (id, account, from_name, from_email, is_default)
             VALUES (?1, ?2, 'Me', 'me@example.test', '\"default\"')",
            [IDENTITY.to_string(), acct_account().to_string()],
        )
        .unwrap();
    }

    let secrets = MemorySecrets::default();
    mail_runtime::block_on(secrets.put(
        &SecretKey {
            account: acct_account(),
            purpose: SecretPurpose::IncomingPassword,
        },
        &Credential::Password(SecretText::new(PASSWORD.to_owned())),
    ))
    .unwrap();

    let draft = draft();

    // The incoming backend, which must never see the submission.
    let backend = Pop3Backend::new(
        acct_account(),
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
    let mut account_plan = plan(smtp_port, pop_port);
    let secrets: Arc<dyn AccountSecrets> = match relays {
        None => Arc::new(secrets),
        Some(relays) => {
            use porter_core::{EndpointUrl, Family, GrantId, LoginName, ServiceEndpoint};
            account_plan.auth = AuthPlan::Granted {
                account: AccountId::parse("fastmail-me").unwrap(),
                grant: GrantId::parse("grant-1").unwrap(),
                endpoints: vec![ServiceEndpoint {
                    family: Family::Smtp,
                    url: EndpointUrl::parse(&format!("smtp://127.0.0.1:{smtp_port}")).unwrap(),
                    tls: porter_core::Tls::Plain,
                    login: LoginName("me@example.test".to_owned()),
                }],
            };
            Arc::new(mail_runtime::link::LinkedSecrets::new(relays))
        }
    };
    let engine = AccountEngine::new(
        acct_account(),
        account_plan,
        backend,
        store.clone(),
        secrets,
    );
    Sending {
        store,
        engine,
        draft,
        _dir: dir,
    }
}

/// One queued message, one drain, and each thing worth checking about what the server got:
/// the report, the envelope (F37), the bytes past a dotted line, and the draft marked sent.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_queued_message_is_submitted_whole() {
    let seen: Shared = Arc::new(Mutex::new(Transcript::default()));
    let port = serve(seen.clone()).await;
    let mut it = compose(port);
    let (_tx, mut cancel) = watch::channel(false);

    assert_eq!(
        it.store.draft(it.draft.id).unwrap().state,
        SendState::Queued,
        "precondition: the draft is queued"
    );
    let report = it
        .engine
        .drain_outbox(&mut cancel, now())
        .await
        .expect("the drain reaches the submission server");

    // The report.
    assert_eq!(
        report.submitted, 1,
        "report: nothing was submitted: {report:?}"
    );
    assert_eq!(report.outbox_settled, 1, "report: settled");
    assert_eq!(
        report.still_queued, 0,
        "report: a delivered message must not still be reported as waiting"
    );
    assert!(
        report.needs_attention.is_empty(),
        "report: {:?}",
        report.needs_attention
    );

    {
        let seen = seen.lock().unwrap();
        // The session.
        assert_eq!(
            seen.mail_from().as_deref(),
            Some("me@example.test"),
            "session: MAIL FROM"
        );
        assert!(
            seen.commands.iter().any(|c| c.starts_with("EHLO")),
            "session: no EHLO: {:?}",
            seen.commands
        );
        assert!(
            seen.commands.iter().any(|c| c == "AUTH PLAIN"),
            "session: the session did not authenticate: {:?}",
            seen.commands
        );

        // The envelope. FINDINGS F37, end to end. Both halves fail silently and in opposite
        // directions, so both are asserted against what the server actually received.
        let mut rcpt = seen.rcpt_to();
        rcpt.sort();
        assert_eq!(
            rcpt,
            vec![
                "bea@example.test".to_owned(),
                "cara@example.test".to_owned(),
                "dee@example.test".to_owned(),
            ],
            "envelope: it must carry every recipient, blind ones included"
        );
        let body = seen.body.to_lowercase();
        assert!(
            !body.contains("bcc:"),
            "envelope: a Bcc header reached the server:\n{}",
            seen.body
        );
        assert!(
            !body.contains("dee@example.test"),
            "envelope: the blind address appears in the transmitted message:\n{}",
            seen.body
        );
        // The message really did carry its visible recipients, so the absence above is the
        // header being omitted rather than the whole envelope being empty.
        assert!(
            body.contains("bea@example.test"),
            "envelope: visible recipient missing:\n{}",
            seen.body
        );

        // Byte for byte. The body opens a line with a dot, which is the terminator. The client
        // stuffs it and the server un-stuffs it; get either wrong and the message is silently
        // truncated there.
        assert!(
            seen.body.contains(".a line that needs stuffing"),
            "bytes: the dotted line did not survive:\n{}",
            seen.body
        );
        assert!(
            seen.body.contains("Shall we say one o'clock?"),
            "bytes: the body was truncated at the dotted line:\n{}",
            seen.body
        );
        assert!(
            body.contains("subject: lunch on friday"),
            "bytes: subject missing:\n{}",
            seen.body
        );
    }

    // The draft.
    match it.store.draft(it.draft.id).unwrap().state {
        SendState::Sent { at, message } => {
            assert_eq!(at, now(), "draft: sent at");
            // SMTP reports acceptance, not where a copy was filed. A POP3 account has nowhere
            // for one to be filed, so the copy is the one kept here.
            let kept = it
                .store
                .message(message.expect("draft: a POP3 send keeps its copy"))
                .unwrap();
            assert_eq!(
                kept.mailbox,
                MailboxRole::Sent,
                "draft: the copy is in Sent"
            );
        }
        other => panic!("draft: did not reach Sent: {other:?}"),
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
        .outbox_due(
            acct_account(),
            now() + chrono::TimeDelta::try_hours(1).unwrap(),
        )
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_granted_account_submits_through_porters_relay_without_authenticating_itself() {
    // porter's SMTP relay does `EHLO` and `AUTH` to the server, and gives the app `220 porter
    // ESMTP ready` and an `EHLO` reply with no `AUTH` and no `STARTTLS`: the session goes straight
    // to `MAIL FROM`, though the plan says the real server wants STARTTLS and a password.
    let seen: Shared = Arc::new(Mutex::new(Transcript::default()));
    let port = serve(seen.clone()).await;
    let relays = relay::Relays::signing_in_with(PASSWORD);
    let mut it = compose_as(port, Some(relays.clone()));
    let (_tx, mut cancel) = watch::channel(false);

    let report = it
        .engine
        .drain_outbox(&mut cancel, now())
        .await
        .expect("the drain reaches the submission server through the relay");
    assert_eq!(report.submitted, 1, "{report:?}");
    assert!(
        report.needs_attention.is_empty(),
        "{:?}",
        report.needs_attention
    );

    let seen = seen.lock().unwrap();
    assert_eq!(seen.mail_from().as_deref(), Some("me@example.test"));
    let mut rcpt = seen.rcpt_to();
    rcpt.sort();
    assert_eq!(rcpt.len(), 3, "{rcpt:?}");
    assert!(
        seen.body.contains("Shall we say one o'clock?"),
        "{}",
        seen.body
    );
    // The relay is what authenticated to the server.
    assert!(
        seen.commands.iter().any(|c| c == "AUTH PLAIN"),
        "{:?}",
        seen.commands
    );
    // The app sent no AUTH to the relay, and never held the password.
    let app = relays.app_sent().to_uppercase();
    assert!(
        !app.contains("AUTH"),
        "the app authenticated itself:\n{app}"
    );
    assert!(!app.contains("STARTTLS"), "{app}");
    assert!(!app.contains(&PASSWORD.to_uppercase()));
    assert!(app.contains("MAIL FROM:<ME@EXAMPLE.TEST>"), "{app}");
    assert_eq!(relays.opened(), 1);
}

/// A POP3 server holding one message, `held`, for as long as the test runs: enough of RFC 1939
/// for a sync, and nothing it does could take away mail this client keeps for itself.
async fn serve_pop(held: String) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((sock, _)) = listener.accept().await {
            let held = held.clone();
            tokio::spawn(async move {
                let (read, mut write) = sock.into_split();
                let mut lines = BufReader::new(read).lines();
                let _ = write.write_all(b"+OK POP3 ready\r\n").await;
                while let Ok(Some(line)) = lines.next_line().await {
                    let upper = line.trim_end().to_ascii_uppercase();
                    let verb = upper.split_whitespace().next().unwrap_or("").to_owned();
                    let reply = match verb.as_str() {
                        "CAPA" => "+OK\r\nTOP\r\nUIDL\r\nUSER\r\nSASL PLAIN\r\n.\r\n".to_owned(),
                        "AUTH" | "USER" | "PASS" => "+OK\r\n".to_owned(),
                        "STAT" => format!("+OK 1 {}\r\n", held.len()),
                        "UIDL" => "+OK\r\n1 ask-0001\r\n.\r\n".to_owned(),
                        "LIST" => format!("+OK\r\n1 {}\r\n.\r\n", held.len()),
                        "TOP" => {
                            let head = held.split("\r\n\r\n").next().unwrap_or("");
                            format!("+OK\r\n{head}\r\n\r\n.\r\n")
                        }
                        "RETR" => format!("+OK\r\n{held}.\r\n"),
                        "QUIT" => {
                            let _ = write.write_all(b"+OK bye\r\n").await;
                            return;
                        }
                        _ => "-ERR unknown command\r\n".to_owned(),
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pop3_send_is_kept_in_sent_on_its_conversation_and_outlives_the_next_sync() {
    // POP3 has no Sent folder. A client that keeps no copy of what it sends leaves the user with
    // none anywhere: the server accepted it and nobody can read it back.
    let seen: Shared = Arc::new(Mutex::new(Transcript::default()));
    let smtp = serve(seen.clone()).await;
    let pop = serve_pop(
        "From: Bea <bea@example.test>\r\n\
         To: me@example.test\r\n\
         Subject: lunch on friday?\r\n\
         Date: Tue, 14 Nov 2023 21:00:00 +0000\r\n\
         Message-ID: <ask@example.test>\r\n\
         \r\n\
         Are you free?\r\n"
            .to_owned(),
    )
    .await;
    let mut it = open(smtp, pop, None);
    let inbox = MailboxRef {
        account: acct_account(),
        path: "INBOX".to_owned(),
    };
    let (_tx, mut cancel) = watch::channel(false);

    it.engine
        .sync(&inbox, &mut cancel, now(), 50)
        .await
        .expect("the first sync fetches Bea's question");
    let in_inbox = it
        .store
        .count(&Filter::InMailbox(MailboxRole::Inbox), now())
        .unwrap();
    assert_eq!(in_inbox, 1, "precondition: her message arrived");
    let asked: String = it
        .store
        .connection()
        .query_row(
            "SELECT id FROM messages WHERE rfc_message_id = 'ask@example.test'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let parent = it
        .store
        .message(MessageId::from_uuid(asked.parse().unwrap()))
        .unwrap();

    // The answer.
    let answer = Draft {
        in_reply_to: Some(parent.id),
        ..it.draft.clone()
    };
    queue(&it.store, &answer, Some(&parent));
    let report = it.engine.drain_outbox(&mut cancel, now()).await.unwrap();
    assert_eq!(report.submitted, 1, "{report:?}");
    assert!(
        report.needs_attention.is_empty(),
        "{:?}",
        report.needs_attention
    );

    let SendState::Sent {
        message: Some(copy),
        ..
    } = it.store.draft(answer.id).unwrap().state
    else {
        panic!("a POP3 send must point at the copy it kept");
    };
    let kept = it.store.message(copy).unwrap();
    assert_eq!(kept.mailbox, MailboxRole::Sent);
    assert_eq!(kept.read, ReadState::Read, "the user wrote it");
    assert_eq!(kept.thread, parent.thread, "the answer joins her question");
    assert_eq!(kept.from.email, "me@example.test");
    // The copy is the message that went, byte for byte, as the server was handed it, plus the
    // `Bcc` that the transmitted bytes leave out (RFC 5322 §3.6.3).
    let Body::Present { raw, .. } = kept.body else {
        panic!("the copy holds its body: {:?}", kept.body);
    };
    let bytes = it.store.blobs().get(&it.store.connection(), raw).unwrap();
    let bytes = String::from_utf8(bytes).unwrap();
    assert!(
        bytes.contains("Bcc: dee@example.test\r\n"),
        "the sender's copy names who was blind-copied:\n{bytes}"
    );
    assert_eq!(
        bytes.replace("Bcc: dee@example.test\r\n", ""),
        seen.lock().unwrap().body,
        "the kept copy differs from what was submitted"
    );
    assert!(
        !seen.lock().unwrap().body.to_lowercase().contains("bcc:"),
        "the transmitted message never carries the Bcc"
    );
    let in_sent = it
        .store
        .count(&Filter::InMailbox(MailboxRole::Sent), now())
        .unwrap();
    assert_eq!(in_sent, 1, "the Sent place lists it");

    // The next sync knows nothing of it, and must not take it away.
    it.engine
        .sync(&inbox, &mut cancel, now(), 50)
        .await
        .expect("the next sync");
    let after = it.store.message(copy).expect("the copy outlives the sync");
    assert_eq!(after.mailbox, MailboxRole::Sent);
    assert_eq!(after.thread, parent.thread);
    assert_eq!(
        it.store
            .count(&Filter::InMailbox(MailboxRole::Sent), now())
            .unwrap(),
        1
    );
}
