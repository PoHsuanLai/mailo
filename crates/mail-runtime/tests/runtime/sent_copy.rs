//! The copy of what an IMAP account sends: SMTP files nothing (RFC 6409), so unless the server
//! is Gmail or Microsoft's the client uploads the message to the `\Sent` mailbox itself
//! (RFC 3501 §6.3.11, RFC 6154), flagged `\Seen`, with the blind recipients named in it
//! (RFC 5322 §3.6.3) and never anywhere on the wire to the other recipients.
//!
//! Both servers are on real sockets and record what they were told. The POP3 pairings are in
//! `submission_end_to_end.rs` (SMTP) and `graph_send.rs` (Graph).

use chrono::{DateTime, TimeZone, Utc};
use mail_domain::id::account_id_from_uuid;
use mail_domain::*;
use mail_mime::posting;
use mail_proto::backend::{Authenticate, ImapBackend};
use mail_proto::{ImapAuth, ImapCommand, ImapSession};
use mail_runtime::{AccountEngine, AccountSecrets};
use mail_store::{SqliteStore, Store};
use porter_core::SecretText;
use porter_core::{AccountId, Credential, SecretKey, SecretPurpose};
use porter_secrets::MemorySecrets;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::sync::watch;

fn acct_account() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000c1"))
}
const IDENTITY: IdentityId =
    IdentityId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000c2"));
const PASSWORD: &str = "s3cr3t";

fn now() -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000, 0).unwrap()
}

/// What the submission server was told.
#[derive(Debug, Default)]
struct Smtp {
    commands: Vec<String>,
    body: String,
}

type SmtpSeen = Arc<Mutex<Smtp>>;

async fn serve_smtp(seen: SmtpSeen) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((sock, _)) = listener.accept().await {
            let seen = seen.clone();
            tokio::spawn(async move {
                let (read, mut write) = sock.into_split();
                let mut lines = BufReader::new(read).lines();
                let _ = write.write_all(b"220 smtp.example ESMTP ready\r\n").await;
                let mut in_data = false;
                let mut body = String::new();
                while let Ok(Some(line)) = lines.next_line().await {
                    if in_data {
                        if line == "." {
                            in_data = false;
                            seen.lock().unwrap().body = std::mem::take(&mut body);
                            let _ = write.write_all(b"250 2.0.0 Ok: queued\r\n").await;
                        } else {
                            body.push_str(line.strip_prefix('.').unwrap_or(&line));
                            body.push_str("\r\n");
                        }
                        continue;
                    }
                    let upper = line.to_uppercase();
                    if !upper.starts_with("AUTH ") {
                        seen.lock().unwrap().commands.push(line.clone());
                    }
                    let reply = if upper.starts_with("EHLO") {
                        "250-smtp.example\r\n250-8BITMIME\r\n250 AUTH PLAIN\r\n"
                    } else if upper.starts_with("AUTH") {
                        "235 2.7.0 Accepted\r\n"
                    } else if upper.starts_with("MAIL FROM") || upper.starts_with("RCPT TO") {
                        "250 2.1.0 Ok\r\n"
                    } else if upper.starts_with("DATA") {
                        in_data = true;
                        "354 End data with <CR><LF>.<CR><LF>\r\n"
                    } else if upper.starts_with("QUIT") {
                        let _ = write.write_all(b"221 2.0.0 Bye\r\n").await;
                        return;
                    } else {
                        "500 5.5.2 Unrecognized command\r\n"
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

/// What the IMAP server was asked to store: the `APPEND` line and the literal behind it.
type Stored = Arc<Mutex<Vec<(String, String)>>>;

/// An IMAP server that takes `APPEND`s. With `drop_first`, the first one is answered by
/// closing the connection, which is a network that went away mid-upload.
async fn serve_imap(stored: Stored, drop_first: bool) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let appends = Arc::new(AtomicUsize::new(0));
    tokio::spawn(async move {
        while let Ok((mut sock, _)) = listener.accept().await {
            let (stored, appends) = (stored.clone(), appends.clone());
            tokio::spawn(async move {
                if sock
                    .write_all(b"* OK [CAPABILITY IMAP4rev1 UIDPLUS] ready\r\n")
                    .await
                    .is_err()
                {
                    return;
                }
                let mut buf = Vec::new();
                loop {
                    let mut chunk = [0u8; 4096];
                    match sock.read(&mut chunk).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => buf.extend_from_slice(&chunk[..n]),
                    }
                    while let Some(at) = buf.windows(2).position(|w| w == b"\r\n") {
                        let line = String::from_utf8_lossy(&buf[..at]).to_string();
                        buf.drain(..at + 2);
                        let (tag, rest) = line.split_once(' ').unwrap_or((&line, ""));
                        let upper = rest.to_uppercase();
                        let reply = if upper.starts_with("CAPABILITY") {
                            format!("* CAPABILITY IMAP4rev1 UIDPLUS\r\n{tag} OK done\r\n")
                        } else if upper.starts_with("APPEND") {
                            if drop_first && appends.fetch_add(1, Ordering::SeqCst) == 0 {
                                return;
                            }
                            let want: usize = rest
                                .rsplit_once('{')
                                .and_then(|(_, n)| n.trim_end_matches('}').parse().ok())
                                .unwrap_or(0);
                            let _ = sock.write_all(b"+ ready for literal\r\n").await;
                            while buf.len() < want + 2 {
                                let mut chunk = [0u8; 4096];
                                match sock.read(&mut chunk).await {
                                    Ok(0) | Err(_) => return,
                                    Ok(n) => buf.extend_from_slice(&chunk[..n]),
                                }
                            }
                            let body = buf.drain(..want).collect::<Vec<u8>>();
                            buf.drain(..2);
                            stored.lock().unwrap().push((
                                rest.to_owned(),
                                String::from_utf8_lossy(&body).to_string(),
                            ));
                            format!("{tag} OK [APPENDUID 42 7] done\r\n")
                        } else if upper.starts_with("LOGOUT") {
                            let _ = sock
                                .write_all(format!("* BYE\r\n{tag} OK done\r\n").as_bytes())
                                .await;
                            return;
                        } else {
                            format!("{tag} OK done\r\n")
                        };
                        if sock.write_all(reply.as_bytes()).await.is_err() {
                            return;
                        }
                    }
                }
            });
        }
    });
    port
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

fn draft() -> Draft {
    let to = |email: &str| Address {
        name: None,
        email: email.to_owned(),
    };
    Draft {
        id: DraftId::generate(),
        account: acct_account(),
        identity: IDENTITY,
        to: vec![to("bea@example.test")],
        cc: vec![to("cara@example.test")],
        bcc: vec![to("dee@example.test")],
        subject: "lunch on friday".to_owned(),
        in_reply_to: None,
        forward_of: None,
        text: "Shall we say one o'clock?\r\n".to_owned(),
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
    engine: AccountEngine<ImapBackend>,
    draft: Draft,
    _dir: tempfile::TempDir,
}

/// An IMAP account sending through the SMTP server at `smtp`, with its draft queued. The server
/// has a Sent mailbox where `sent_folder` is true, as a `SPECIAL-USE` listing would have said.
fn compose(smtp: u16, imap: u16, sent_folder: bool) -> Sending {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::in_memory(dir.path()).unwrap());
    mail_store::testing::seed_account(&store, acct_account(), "me@example.test");
    mail_store::testing::seed_identity_for(
        &store,
        IDENTITY,
        acct_account(),
        "me@example.test",
        Some("Me"),
    );
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
    store
        .apply(
            acct_account(),
            &Patch {
                id: ChangeId::generate(),
                changes: vec![Change::DraftUpsert(Box::new(draft.clone()))],
            },
        )
        .unwrap();
    let post = posting(&draft, &identity(), None, &[]).expect("the draft has recipients");
    let raw = store.blobs().put(&post.message).unwrap();
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

    let caps = AccountCaps {
        labels: ServerLabels::LocalOnly,
        threads: ServerThreads::Jwz,
        watch: WatchMode::Poll {
            every: std::time::Duration::from_secs(300),
        },
        archive: ArchiveMeans::LocalOnly,
        folders: FolderRoles(if sent_folder {
            vec![("Sent".to_owned(), MailboxRole::Sent)]
        } else {
            Vec::new()
        }),
        condstore: Condstore::Absent,
        move_ext: MoveExt::Absent,
        expunge: ExpungeMeans::Forbidden,
        top: Supported::Absent,
        pipelining: Supported::Absent,
        connections: ConnectionBudget { max: 1 },
        observed_at: now(),
    };
    let auth = ImapAuth {
        username: "me@example.test".to_owned(),
        credential: Credential::Password(SecretText::new(PASSWORD.to_owned())),
        sasl: vec![SaslMech::Plain],
    };
    let backend = ImapBackend::new(
        acct_account(),
        caps,
        Box::new(move |authenticate, commands: Vec<ImapCommand>| {
            let mut all = Vec::new();
            if authenticate == Authenticate::First {
                all.push(ImapCommand::Login);
            }
            all.extend(commands);
            ImapSession::new(auth.clone(), all)
        }),
    );
    let plan = AccountPlan {
        address: "me@example.test".to_owned(),
        incoming: Incoming::Imap {
            host: "127.0.0.1".to_owned(),
            port: imap,
            tls: Tls::Plaintext,
        },
        // A loopback server that is no Gmail, so nothing files the copy but this client.
        outgoing: Outgoing::Smtp {
            host: "127.0.0.1".to_owned(),
            port: smtp,
            tls: Tls::Plaintext,
        },
        auth: AuthPlan::Password {
            username: Username::SameAsAddress,
            sasl: vec![SaslMech::Plain],
        },
        identities: vec![identity()],
    };
    let engine = AccountEngine::new(
        acct_account(),
        plan,
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

fn mail_from_count(seen: &SmtpSeen) -> usize {
    seen.lock()
        .unwrap()
        .commands
        .iter()
        .filter(|c| c.to_uppercase().starts_with("MAIL FROM"))
        .count()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_imap_send_on_a_server_that_files_nothing_uploads_a_seen_copy_to_sent_with_the_bcc() {
    let smtp: SmtpSeen = Arc::default();
    let stored: Stored = Arc::default();
    let mut it = compose(
        serve_smtp(smtp.clone()).await,
        serve_imap(stored.clone(), false).await,
        true,
    );
    let (_tx, mut cancel) = watch::channel(false);

    let report = it.engine.drain_outbox(&mut cancel, now()).await.unwrap();

    assert_eq!(report.submitted, 1, "{report:?}");
    assert_eq!(
        report.appended, 1,
        "the copy went in the same pass: {report:?}"
    );
    assert!(report.needs_attention.is_empty(), "{report:?}");
    assert_eq!(report.still_queued, 0, "{report:?}");

    let stored = stored.lock().unwrap();
    assert_eq!(stored.len(), 1, "exactly one copy is filed: {stored:?}");
    let (line, literal) = &stored[0];
    assert!(line.contains("Sent"), "the \\Sent mailbox: {line}");
    assert!(line.contains("(\\Seen)"), "the user wrote it: {line}");
    assert!(
        literal.contains("Bcc: dee@example.test\r\n"),
        "the sender's copy records who was blind-copied:\n{literal}"
    );
    assert!(literal.contains("Subject: lunch on friday"), "{literal}");

    // The wire never had it: the blind recipient is in the envelope and nowhere else.
    let smtp = smtp.lock().unwrap();
    assert!(
        smtp.commands
            .iter()
            .any(|c| c == "RCPT TO:<dee@example.test>"),
        "{:?}",
        smtp.commands
    );
    let wire = smtp.body.to_lowercase();
    assert!(!wire.contains("bcc:") && !wire.contains("dee@"), "{wire}");
    assert_eq!(
        literal.replace("Bcc: dee@example.test\r\n", ""),
        smtp.body,
        "the copy is the message that went"
    );

    match it.store.draft(it.draft.id).unwrap().state {
        SendState::Sent { message, .. } => {
            assert_eq!(message, None, "a sync finds the uploaded copy in Sent")
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_copy_that_could_not_be_uploaded_waits_in_the_outbox_and_is_never_sent_twice() {
    let smtp: SmtpSeen = Arc::default();
    let stored: Stored = Arc::default();
    let mut it = compose(
        serve_smtp(smtp.clone()).await,
        serve_imap(stored.clone(), true).await,
        true,
    );
    let (_tx, mut cancel) = watch::channel(false);

    let first = it.engine.drain_outbox(&mut cancel, now()).await.unwrap();
    assert_eq!(first.submitted, 1, "the message went: {first:?}");
    assert!(stored.lock().unwrap().is_empty(), "the connection was cut");
    assert_eq!(first.still_queued, 1, "the copy is waiting: {first:?}");

    // Later, with the network back.
    let later = now() + chrono::TimeDelta::try_hours(1).unwrap();
    let second = it.engine.drain_outbox(&mut cancel, later).await.unwrap();
    assert_eq!(second.appended, 1, "{second:?}");
    assert_eq!(second.submitted, 0, "{second:?}");
    assert_eq!(stored.lock().unwrap().len(), 1);
    assert_eq!(mail_from_count(&smtp), 1, "the message was submitted once");
    assert!(
        it.store
            .outbox_due(acct_account(), later)
            .unwrap()
            .is_empty()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_imap_account_whose_server_names_no_sent_mailbox_keeps_its_copy_here() {
    // Uploading to a path nobody confirmed is how a message lands where no one looks.
    let smtp: SmtpSeen = Arc::default();
    let stored: Stored = Arc::default();
    let mut it = compose(
        serve_smtp(smtp.clone()).await,
        serve_imap(stored.clone(), false).await,
        false,
    );
    let (_tx, mut cancel) = watch::channel(false);

    let report = it.engine.drain_outbox(&mut cancel, now()).await.unwrap();

    assert_eq!(report.submitted, 1, "{report:?}");
    assert!(stored.lock().unwrap().is_empty(), "nothing was guessed");
    let SendState::Sent {
        message: Some(copy),
        ..
    } = it.store.draft(it.draft.id).unwrap().state
    else {
        panic!("the copy is kept here");
    };
    let kept = it.store.message(copy).unwrap();
    assert_eq!(kept.mailbox, MailboxRole::Sent);
    let Body::Present { raw, .. } = kept.body else {
        panic!("{:?}", kept.body);
    };
    let bytes = it.store.blobs().get(raw).unwrap();
    assert!(
        String::from_utf8(bytes)
            .unwrap()
            .contains("Bcc: dee@example.test\r\n")
    );
}
