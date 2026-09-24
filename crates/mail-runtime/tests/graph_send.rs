//! Sending through Microsoft Graph: outbox, engine, one HTTPS call — here, plain HTTP to a
//! listener in this process standing in for `graph.microsoft.com`.
//!
//! What matters is only visible from the server's side: which token was presented (the Graph
//! one, never the IMAP one), what the body is (the frozen message, base64), and that a blind
//! recipient, which Graph cannot learn from an envelope, is carried as a `Bcc:` header.
//!
//! A message too large for that one call is built as a draft, its large attachments uploaded in
//! ranges to an address that is never shown the token, and sent; the second half of this file
//! plays Graph's side of that, mostly with small [`Limits`] so crossing them takes kilobytes.

use base64::Engine as _;
use chrono::{DateTime, TimeZone, Utc};
use mail_domain::*;
use mail_mime::posting;
use mail_proto::backend::{Authenticate, Pop3Backend};
use mail_proto::{Pop3Command, Pop3Session};
use mail_runtime::graph::{Limits, send_mime_within};
use mail_runtime::{AccountEngine, MapSecrets, Secrets};
use mail_store::{SqliteStore, Store};
use std::sync::atomic::{AtomicUsize, Ordering};
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

/// An answer: the status line's tail, extra header lines, and the body.
struct Answer {
    status: &'static str,
    headers: String,
    body: String,
}

fn answer(status: &'static str, body: impl Into<String>) -> Answer {
    Answer {
        status,
        headers: String::new(),
        body: body.into(),
    }
}

/// What the listener says to a request, given its own port for any address it hands out.
type Script = Arc<dyn Fn(&Request, u16) -> Answer + Send + Sync>;

/// Answer every request with `status` and `body`, recording what was asked.
async fn serve(seen: Seen, status: &'static str, body: &'static str) -> u16 {
    serve_script(seen, Arc::new(move |_, _| answer(status, body))).await
}

/// Answer each request as `script` says, recording what was asked.
async fn serve_script(seen: Seen, script: Script) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((sock, _)) = listener.accept().await {
            let seen = seen.clone();
            let script = script.clone();
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
                let Answer {
                    status,
                    headers,
                    body,
                } = script(&request, port);
                seen.lock().unwrap().push(request);
                let reply = format!(
                    "HTTP/1.1 {status}\r\ncontent-type: application/json\r\n{headers}\
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
        receipt: ReceiptRequest::Unrequested,
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
    compose_as(port, None)
}

/// [`compose`], with `message` frozen in the outbox in place of the draft's own bytes.
fn compose_as(port: u16, message: Option<Vec<u8>>) -> Sending {
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
        .put(
            &store.connection(),
            message.as_deref().unwrap_or(&post.message),
        )
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

// ---------------------------------------------------------------------------------------
// Past sendMail's limit: a draft, an upload session, a send.
// ---------------------------------------------------------------------------------------

/// The id the fake Graph gives every draft: base64, with the `/` and `+` that must be escaped.
const DRAFT_ID: &str = "AAMk/draft+1=";
const DRAFT_PATH: &str = "/v1.0/me/messages/AAMk%2Fdraft%2B1=";

/// Small enough that a few kilobytes cross every one of them.
const SMALL: Limits = Limits {
    request: 4096,
    small_attachment: 1024,
    message: 64 * 1024,
    chunk: 4000,
};

fn bytes(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i % 251) as u8).collect()
}

fn base64_lines(bytes: &[u8]) -> String {
    let flat = base64::engine::general_purpose::STANDARD.encode(bytes);
    flat.as_bytes()
        .chunks(76)
        .map(|line| std::str::from_utf8(line).unwrap())
        .collect::<Vec<_>>()
        .join("\r\n")
}

/// A message to Bea and Cara with an image its HTML shows by `cid:`, and `attachments`, each
/// named and as long as given. Dee, the blind recipient, is only in the envelope.
fn message_with(attachments: &[(&str, usize)]) -> Vec<u8> {
    let mut out = format!(
        "From: Me <me@example.test>\r\nTo: Bea <bea@example.test>\r\n\
         Cc: cara@example.test\r\nSubject: the photos\r\n\
         Date: Mon, 1 Jan 2001 00:00:00 +0000\r\nMessage-ID: <Big.One@example.test>\r\n\
         X-Mailer: mailo\r\nMIME-Version: 1.0\r\n\
         Content-Type: multipart/mixed; boundary=\"outer\"\r\n\r\n\
         --outer\r\nContent-Type: multipart/related; boundary=\"inner\"\r\n\r\n\
         --inner\r\nContent-Type: text/html; charset=utf-8\r\n\r\n\
         <p>here <img src=\"cid:logo@example.test\"></p>\r\n\
         --inner\r\nContent-Type: image/png\r\nContent-ID: <logo@example.test>\r\n\
         Content-Disposition: inline; filename=\"logo.png\"\r\n\
         Content-Transfer-Encoding: base64\r\n\r\n{}\r\n--inner--\r\n",
        base64_lines(b"\x89PNG\r\n\x1a\n")
    );
    for (name, len) in attachments {
        out.push_str(&format!(
            "--outer\r\nContent-Type: application/octet-stream\r\n\
             Content-Disposition: attachment; filename=\"{name}\"\r\n\
             Content-Transfer-Encoding: base64\r\n\r\n{}\r\n",
            base64_lines(&bytes(*len))
        ));
    }
    out.push_str("--outer--\r\n");
    out.into_bytes()
}

fn rcpt_to() -> Vec<String> {
    ["bea@example.test", "cara@example.test", "dee@example.test"]
        .map(str::to_owned)
        .to_vec()
}

/// `(method, path)` of a request line.
fn route(request: &Request) -> (&str, &str) {
    let mut words = request.line.split(' ');
    (words.next().unwrap_or(""), words.next().unwrap_or(""))
}

/// `Content-Range: bytes a-b/total`, as numbers.
fn range(request: &Request) -> (usize, usize, usize) {
    let value = request
        .header("content-range")
        .expect("a range on every PUT");
    let value = value.strip_prefix("bytes ").unwrap();
    let (span, total) = value.split_once('/').unwrap();
    let (start, end) = span.split_once('-').unwrap();
    (
        start.parse().unwrap(),
        end.parse().unwrap(),
        total.parse().unwrap(),
    )
}

/// An upload session that takes each range and asks for the one after it.
fn take_the_range(request: &Request, _: usize) -> Answer {
    let (_, end, total) = range(request);
    if end + 1 == total {
        answer("201 Created", "{}")
    } else {
        answer(
            "202 Accepted",
            format!(r#"{{"nextExpectedRanges":["{}-"]}}"#, end + 1),
        )
    }
}

/// Graph, as far as a draft goes: `put` answers the `n`th upload `PUT`, `session` the request
/// that opens an upload session (by default, one handing out an address on this listener).
fn drafting(
    put: impl Fn(&Request, usize) -> Answer + Send + Sync + 'static,
    session: Option<fn() -> Answer>,
) -> Script {
    let puts = Arc::new(AtomicUsize::new(0));
    Arc::new(move |request, port| match route(request) {
        ("POST", "/v1.0/me/messages") => answer("201 Created", format!(r#"{{"id":"{DRAFT_ID}"}}"#)),
        ("POST", path) if path.ends_with("/createUploadSession") => match session {
            Some(session) => session(),
            None => answer(
                "201 Created",
                format!(
                    r#"{{"uploadUrl":"http://127.0.0.1:{port}/upload/s1?sig=x",
                        "nextExpectedRanges":["0-"]}}"#
                ),
            ),
        },
        ("PUT", _) => put(request, puts.fetch_add(1, Ordering::SeqCst)),
        ("POST", path) if path.ends_with("/attachments") => answer("201 Created", "{}"),
        ("POST", path) if path.ends_with("/send") => answer("202 Accepted", ""),
        ("DELETE", _) => answer("204 No Content", ""),
        _ => answer("404 Not Found", ""),
    })
}

fn lines(seen: &Seen) -> Vec<String> {
    seen.lock()
        .unwrap()
        .iter()
        .map(|r| {
            let (method, path) = route(r);
            format!("{method} {path}")
        })
        .collect()
}

fn json(request: &Request) -> serde_json::Value {
    serde_json::from_slice(&request.body).expect("a JSON body")
}

async fn send_small(port: u16, mime: &[u8]) -> Result<(), mail_runtime::RuntimeError> {
    send_mime_within(
        &reqwest::Client::new(),
        &format!("http://127.0.0.1:{port}/v1.0/me/sendMail"),
        "graph-token",
        mime,
        &rcpt_to(),
        &SMALL,
    )
    .await
}

fn retry_of(result: Result<(), mail_runtime::RuntimeError>) -> (String, Retry) {
    match result {
        Err(mail_runtime::RuntimeError::Graph { why, retry }) => (why, retry),
        other => panic!("{other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_message_past_four_megabytes_goes_as_a_draft_with_its_attachment_uploaded_in_ranges() {
    const TEN_MB: usize = 10 * 1024 * 1024;
    let seen: Seen = Arc::default();
    let port = serve_script(seen.clone(), drafting(take_the_range, None)).await;
    let mut it = compose_as(port, Some(message_with(&[("photos.zip", TEN_MB)])));
    let (_tx, mut cancel) = watch::channel(false);

    let report = it.engine.drain_outbox(&mut cancel, now()).await.unwrap();

    assert_eq!(report.submitted, 1, "{report:?}");
    let chunk = mail_runtime::graph::CHUNK;
    assert_eq!(chunk % (320 * 1024), 0, "Graph wants multiples of 320 KiB");
    let draft = format!("POST {DRAFT_PATH}");
    assert_eq!(
        lines(&seen),
        [
            "POST /v1.0/me/messages".to_owned(),
            format!("{draft}/attachments/createUploadSession"),
            "PUT /upload/s1?sig=x".to_owned(),
            "PUT /upload/s1?sig=x".to_owned(),
            "PUT /upload/s1?sig=x".to_owned(),
            format!("{draft}/send"),
        ]
    );
    let seen = seen.lock().unwrap();

    for request in seen.iter().filter(|r| !r.line.starts_with("PUT")) {
        assert_eq!(request.header("authorization"), Some("Bearer graph-token"));
    }
    let puts: Vec<&Request> = seen.iter().filter(|r| r.line.starts_with("PUT")).collect();
    for put in &puts {
        assert_eq!(
            put.header("authorization"),
            None,
            "the upload address is pre-authenticated and must never see the token"
        );
    }
    assert_eq!(
        puts.iter().map(|p| range(p)).collect::<Vec<_>>(),
        [
            (0, chunk - 1, TEN_MB),
            (chunk, 2 * chunk - 1, TEN_MB),
            (2 * chunk, TEN_MB - 1, TEN_MB),
        ]
    );
    let uploaded: Vec<u8> = puts.iter().flat_map(|p| p.body.clone()).collect();
    assert!(
        uploaded == bytes(TEN_MB),
        "the attachment's bytes, in order"
    );

    let created = json(&seen[0]);
    assert_eq!(created["subject"], "the photos");
    assert_eq!(created["body"]["contentType"], "html");
    assert_eq!(
        created["toRecipients"][0]["emailAddress"],
        serde_json::json!({ "name": "Bea", "address": "bea@example.test" })
    );
    assert_eq!(
        created["ccRecipients"][0]["emailAddress"]["address"],
        "cara@example.test"
    );
    assert_eq!(
        created["bccRecipients"],
        serde_json::json!([{ "emailAddress": { "address": "dee@example.test" } }]),
        "the blind recipient is only in the envelope, and Graph reads none"
    );
    assert_eq!(created["internetMessageId"], "<Big.One@example.test>");
    assert_eq!(
        created["internetMessageHeaders"],
        serde_json::json!([{ "name": "X-Mailer", "value": "mailo" }])
    );
    let logo = &created["attachments"][0];
    assert_eq!(logo["@odata.type"], "#microsoft.graph.fileAttachment");
    assert_eq!(logo["isInline"], true);
    assert_eq!(logo["contentId"], "logo@example.test");
    assert_eq!(logo["contentType"], "image/png");
    assert_eq!(created["attachments"].as_array().unwrap().len(), 1);
    // The restamped `Date` goes nowhere: Graph stamps its own `sentDateTime` when it sends.
    let text = String::from_utf8(seen[0].body.clone()).unwrap();
    for stamp in ["2001", "14 Nov 2023", "sentDateTime", "Date"] {
        assert!(!text.contains(stamp), "{stamp} in {text}");
    }

    let item = &json(&seen[1])["AttachmentItem"];
    assert_eq!(
        *item,
        serde_json::json!({
            "attachmentType": "file",
            "name": "photos.zip",
            "size": TEN_MB,
            "contentType": "application/octet-stream",
            "isInline": false,
        })
    );
    match it.store.draft(it.draft.id).unwrap().state {
        SendState::Sent { .. } => {}
        other => panic!("{other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn small_attachments_the_draft_has_no_room_for_are_posted_to_it_one_by_one() {
    let seen: Seen = Arc::default();
    let port = serve_script(seen.clone(), drafting(take_the_range, None)).await;
    let mime = message_with(&[("a.txt", 1000), ("b.txt", 1000), ("c.txt", 1000)]);

    send_small(port, &mime).await.unwrap();

    let draft = format!("POST {DRAFT_PATH}");
    assert_eq!(
        lines(&seen),
        [
            "POST /v1.0/me/messages".to_owned(),
            format!("{draft}/attachments"),
            format!("{draft}/send"),
        ]
    );
    let seen = seen.lock().unwrap();
    let packed = json(&seen[0])["attachments"].as_array().unwrap().len();
    assert_eq!(packed, 3, "the image and two of the three files fit");
    let posted = json(&seen[1]);
    assert_eq!(posted["name"], "c.txt");
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(posted["contentBytes"].as_str().unwrap())
            .unwrap(),
        bytes(1000)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_upload_resumes_from_where_graph_says_it_got_to() {
    let seen: Seen = Arc::default();
    let put = |request: &Request, n: usize| {
        if n == 0 {
            // Took only part of the first range.
            answer("202 Accepted", r#"{"nextExpectedRanges":["1500-9999"]}"#)
        } else {
            take_the_range(request, n)
        }
    };
    let port = serve_script(seen.clone(), drafting(put, None)).await;

    send_small(port, &message_with(&[("big.bin", 10_000)]))
        .await
        .unwrap();

    let seen = seen.lock().unwrap();
    let ranges: Vec<_> = seen
        .iter()
        .filter(|r| r.line.starts_with("PUT"))
        .map(range)
        .collect();
    assert_eq!(
        ranges,
        [
            (0, 3999, 10_000),
            (1500, 5499, 10_000),
            (5500, 9499, 10_000),
            (9500, 9999, 10_000)
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_upload_deletes_the_draft_and_says_when_to_try_again() {
    let seen: Seen = Arc::default();
    let put = |request: &Request, n: usize| {
        if n == 1 {
            answer(
                "500 Internal Server Error",
                r#"{"error":{"code":"ErrorInternalServerError","message":"try later"}}"#,
            )
        } else {
            take_the_range(request, n)
        }
    };
    let port = serve_script(seen.clone(), drafting(put, None)).await;

    let (why, retry) = retry_of(send_small(port, &message_with(&[("big.bin", 10_000)])).await);

    assert!(why.contains("ErrorInternalServerError"), "{why}");
    assert_eq!(retry, Retry::After(std::time::Duration::from_secs(60)));
    let draft = format!("POST {DRAFT_PATH}");
    assert_eq!(
        lines(&seen),
        [
            "POST /v1.0/me/messages".to_owned(),
            format!("{draft}/attachments/createUploadSession"),
            "PUT /upload/s1?sig=x".to_owned(),
            "PUT /upload/s1?sig=x".to_owned(),
            format!("DELETE {DRAFT_PATH}"),
        ],
        "no send, and no draft left behind in the user's Drafts"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_throttled_upload_session_waits_as_long_as_graph_says_and_leaves_no_draft() {
    let seen: Seen = Arc::default();
    let throttled = || Answer {
        status: "429 Too Many Requests",
        headers: "retry-after: 7\r\n".to_owned(),
        body: r#"{"error":{"code":"ApplicationThrottled","message":"slow down"}}"#.to_owned(),
    };
    let port = serve_script(seen.clone(), drafting(take_the_range, Some(throttled))).await;

    let (_, retry) = retry_of(send_small(port, &message_with(&[("big.bin", 10_000)])).await);

    assert_eq!(retry, Retry::After(std::time::Duration::from_secs(7)));
    assert_eq!(
        lines(&seen).last().map(String::as_str),
        Some(format!("DELETE {DRAFT_PATH}").as_str())
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_sign_in_without_mail_readwrite_is_told_to_consent_again() {
    let seen: Seen = Arc::default();
    let port = serve(
        seen.clone(),
        "403 Forbidden",
        r#"{"error":{"code":"ErrorAccessDenied","message":"Access is denied."}}"#,
    )
    .await;

    let (why, retry) = retry_of(send_small(port, &message_with(&[("big.bin", 10_000)])).await);

    assert!(why.contains("ErrorAccessDenied"), "{why}");
    assert!(
        matches!(retry, Retry::Fatal(ref w) if w.contains("Mail.ReadWrite")
            && w.contains("--microsoft --send graph")),
        "{retry:?}"
    );
    assert_eq!(
        lines(&seen),
        ["POST /v1.0/me/messages"],
        "no draft was made, so there is none to delete"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_refused_token_asks_for_a_new_sign_in_before_any_draft_exists() {
    let seen: Seen = Arc::default();
    let port = serve(seen.clone(), "401 Unauthorized", "").await;

    let (_, retry) = retry_of(send_small(port, &message_with(&[("big.bin", 10_000)])).await);

    assert_eq!(retry, Retry::NeedsReauth);
    assert_eq!(lines(&seen), ["POST /v1.0/me/messages"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_message_larger_than_graph_sends_is_refused_before_anything_is_asked() {
    let seen: Seen = Arc::default();
    let port = serve_script(seen.clone(), drafting(take_the_range, None)).await;

    let result = send_mime_within(
        &reqwest::Client::new(),
        &format!("http://127.0.0.1:{port}/v1.0/me/sendMail"),
        "graph-token",
        &message_with(&[("big.bin", 10_000)]),
        &rcpt_to(),
        &Limits {
            message: 8000,
            ..SMALL
        },
    )
    .await;

    let (why, retry) = retry_of(result);
    assert!(why.contains("share the file by link"), "{why}");
    assert!(matches!(retry, Retry::Fatal(_)), "{retry:?}");
    assert!(seen.lock().unwrap().is_empty(), "nothing was created");
}
