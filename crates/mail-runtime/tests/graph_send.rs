//! Sending through Microsoft Graph: outbox, engine, one HTTPS call — here, plain HTTP to a
//! listener in this process standing in for `graph.microsoft.com`.
//!
//! What matters is only visible from the server's side: which token was presented (the Graph
//! one, never the IMAP one), what the body is (the frozen message, base64), and that a blind
//! recipient, which Graph cannot learn from an envelope, is carried as a `Bcc:` header.

use base64::Engine as _;
use chrono::{DateTime, TimeZone, Utc};
use mail_domain::*;
use mail_mime::posting;
use mail_proto::backend::{Authenticate, Pop3Backend};
use mail_proto::{Pop3Command, Pop3Session};
use mail_runtime::{AccountEngine, MapSecrets, Secrets};
use mail_store::{SqliteStore, Store};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::sync::watch;

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));
const IDENTITY: IdentityId =
    IdentityId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000b1"));

fn now() -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000, 0).unwrap()
}

/// One request, as the listener received it.
#[derive(Debug, Default, Clone)]
struct Request {
    line: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

type Seen = Arc<Mutex<Vec<Request>>>;

/// Answer every request with `status` and `body`, recording what was asked.
async fn serve(seen: Seen, status: &'static str, body: &'static str) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((sock, _)) = listener.accept().await {
            let seen = seen.clone();
            tokio::spawn(async move {
                let mut reader = BufReader::new(sock);
                let mut request = Request::default();
                reader.read_line(&mut request.line).await.unwrap();
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).await.unwrap();
                    let line = line.trim_end();
                    if line.is_empty() {
                        break;
                    }
                    if let Some((name, value)) = line.split_once(':') {
                        request
                            .headers
                            .push((name.trim().to_owned(), value.trim().to_owned()));
                    }
                }
                let length: usize = request
                    .header("content-length")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
                request.body = vec![0; length];
                reader.read_exact(&mut request.body).await.unwrap();
                seen.lock().unwrap().push(request);
                let reply = format!(
                    "HTTP/1.1 {status}\r\ncontent-type: application/json\r\n\
                     content-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                reader.get_mut().write_all(reply.as_bytes()).await.unwrap();
            });
        }
    });
    port
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

fn plan() -> AccountPlan {
    AccountPlan {
        address: "me@example.test".to_owned(),
        // Never reached: its port is closed, so routing a submission here fails the test.
        incoming: Incoming::Pop3 {
            host: "127.0.0.1".to_owned(),
            port: 1,
            tls: Tls::Plaintext,
            leave: LeaveOnServer::Keep,
        },
        outgoing: Outgoing::Graph,
        auth: AuthPlan::OAuth {
            issuer: OAuthIssuer::Microsoft,
            scopes: Vec::new(),
        },
        identities: vec![identity()],
    }
}

fn token(access: &str) -> Credential {
    Credential::OAuth {
        access: access.to_owned(),
        refresh: "refresh".to_owned(),
        expires_at: now() + chrono::TimeDelta::try_hours(1).unwrap(),
    }
}

struct Sending {
    store: Arc<SqliteStore>,
    engine: AccountEngine<Pop3Backend>,
    draft: Draft,
    _dir: tempfile::TempDir,
}

/// A draft to three people, one blind, queued for an account that sends through Graph at `port`.
fn compose(port: u16) -> Sending {
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
    for (purpose, access) in [
        (SecretPurpose::IncomingPassword, "imap-token"),
        (SecretPurpose::OutgoingPassword, "graph-token"),
    ] {
        secrets
            .put(
                &SecretKey {
                    account: ACCOUNT,
                    purpose,
                },
                &token(access),
            )
            .unwrap();
    }

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

    let backend = Pop3Backend::new(
        ACCOUNT,
        caps(),
        Box::new(|auth, commands| {
            let mut all = Vec::new();
            if auth == Authenticate::First {
                all.push(Pop3Command::AuthPlain);
            }
            all.extend(commands);
            Pop3Session::new("me@example.test", "unused", all)
        }),
    );
    let engine = AccountEngine::new(ACCOUNT, plan(), backend, store.clone(), Arc::new(secrets))
        .with_graph_url(format!("http://127.0.0.1:{port}/v1.0/me/sendMail"));
    Sending {
        store,
        engine,
        draft,
        _dir: dir,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_queued_message_goes_to_graph_with_the_graph_token() {
    let seen: Seen = Arc::default();
    let port = serve(seen.clone(), "202 Accepted", "").await;
    let mut it = compose(port);
    let (_tx, mut cancel) = watch::channel(false);

    let report = it.engine.drain_outbox(&mut cancel, now()).await.unwrap();

    assert_eq!(report.submitted, 1, "{report:?}");
    assert!(report.needs_attention.is_empty(), "{report:?}");
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    let request = &seen[0];
    assert!(
        request.line.starts_with("POST /v1.0/me/sendMail "),
        "{}",
        request.line
    );
    assert_eq!(
        request.header("authorization"),
        Some("Bearer graph-token"),
        "Graph's own token, not the one for Exchange's IMAP"
    );
    assert_eq!(request.header("content-type"), Some("text/plain"));

    let mime = base64::engine::general_purpose::STANDARD
        .decode(&request.body)
        .expect("the body is base64 MIME");
    let mime = String::from_utf8(mime).unwrap();
    assert!(mime.contains("Subject: lunch on friday"), "{mime}");
    assert!(
        mime.starts_with("Bcc: dee@example.test\r\n"),
        "Graph reads recipients from the headers, so the blind one must be there: {mime}"
    );
    match it.store.draft(it.draft.id).unwrap().state {
        SendState::Sent { message, .. } => assert_eq!(message, None),
        other => panic!("{other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_refusal_says_which_permission_is_missing() {
    let seen: Seen = Arc::default();
    let port = serve(
        seen.clone(),
        "403 Forbidden",
        r#"{"error":{"code":"ErrorAccessDenied","message":"Access is denied."}}"#,
    )
    .await;
    let mut it = compose(port);
    let (_tx, mut cancel) = watch::channel(false);

    let report = it.engine.drain_outbox(&mut cancel, now()).await.unwrap();

    assert_eq!(report.submitted, 0);
    let said = report.needs_attention.join("\n");
    assert!(said.contains("ErrorAccessDenied"), "{said}");
    match it.store.draft(it.draft.id).unwrap().state {
        SendState::Failed { retry, .. } => {
            assert!(
                matches!(retry, Retry::Fatal(ref why) if why.contains("Mail.Send")),
                "{retry:?}"
            )
        }
        other => panic!("{other:?}"),
    }
}
