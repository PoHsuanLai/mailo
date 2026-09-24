//! Reading a mailbox through Microsoft Graph (`plan.md` 10.18): an engine driven against a
//! listener in this process that plays Graph's side — folders and their children, delta pages
//! and links, `$value` bodies, `PATCH`es and moves, throttling, and a token it stops accepting.
//!
//! Nothing here reaches the network. What matters is visible from both sides: what the listener
//! was asked, with which token, and what the store holds afterwards.

use chrono::{DateTime, TimeDelta, TimeZone, Utc};
use mail_domain::*;
use mail_runtime::graph::read::{OverHttp, Reader};
use mail_runtime::oauth::Endpoints;
use mail_runtime::{AccountEngine, Held, MapSecrets, Registration, Renewal, Secrets};
use mail_store::{SqliteStore, Store};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::sync::watch;

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));

fn now() -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000, 0).unwrap()
}

// ---------------------------------------------------------------------------------------------
// A listener that answers as a script says.

#[derive(Debug, Default, Clone)]
struct Request {
    method: String,
    /// Path and query, as sent.
    target: String,
    headers: Vec<(String, String)>,
    body: String,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    fn path(&self) -> &str {
        self.target.split('?').next().unwrap_or_default()
    }

    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or(Value::Null)
    }
}

type Seen = Arc<Mutex<Vec<Request>>>;

struct Answer {
    status: &'static str,
    headers: String,
    body: String,
}

fn ok(body: Value) -> Answer {
    Answer {
        status: "200 OK",
        headers: String::new(),
        body: body.to_string(),
    }
}

fn status(status: &'static str, body: &str) -> Answer {
    Answer {
        status,
        headers: String::new(),
        body: body.to_owned(),
    }
}

/// What the listener says to a request, given its own port for the links it hands out.
type Script = Arc<dyn Fn(&Request, u16) -> Answer + Send + Sync>;

async fn serve(script: Script) -> (u16, Seen) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen: Seen = Arc::default();
    let recording = seen.clone();
    tokio::spawn(async move {
        while let Ok((sock, _)) = listener.accept().await {
            let (seen, script) = (recording.clone(), script.clone());
            tokio::spawn(async move {
                let mut reader = BufReader::new(sock);
                // One connection may carry several requests.
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
                        return;
                    }
                    let mut parts = line.split_whitespace();
                    let mut request = Request {
                        method: parts.next().unwrap_or_default().to_owned(),
                        target: parts.next().unwrap_or_default().to_owned(),
                        ..Request::default()
                    };
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
                    let mut body = vec![0; length];
                    reader.read_exact(&mut body).await.unwrap();
                    request.body = String::from_utf8_lossy(&body).into_owned();
                    let answer = script(&request, port);
                    seen.lock().unwrap().push(request);
                    let reply = format!(
                        "HTTP/1.1 {}\r\ncontent-type: application/json\r\n{}\
                         content-length: {}\r\n\r\n{}",
                        answer.status,
                        answer.headers,
                        answer.body.len(),
                        answer.body
                    );
                    if reader.get_mut().write_all(reply.as_bytes()).await.is_err() {
                        return;
                    }
                }
            });
        }
    });
    (port, seen)
}

fn me(port: u16) -> String {
    format!("http://127.0.0.1:{port}/v1.0/me")
}

// ---------------------------------------------------------------------------------------------
// Graph's side: a mailbox of folders and messages.

/// A delta entry for message `n`, in the shape Graph returns with the properties asked for.
fn entry(id: &str, n: u32, read: bool, size: u64) -> Value {
    json!({
        "id": id,
        "internetMessageId": format!("<m{n}@example.test>"),
        "receivedDateTime": format!("2026-09-2{}T08:00:00Z", n % 10),
        "sentDateTime": format!("2026-09-2{}T07:59:00Z", n % 10),
        "isRead": read,
        "flag": { "flagStatus": "notFlagged" },
        "subject": format!("message {n}"),
        "from": { "emailAddress": { "name": "Ada", "address": "ada@example.test" } },
        "toRecipients": [ { "emailAddress": { "name": "Me", "address": "me@example.test" } } ],
        "ccRecipients": [],
        "hasAttachments": false,
        "singleValueExtendedProperties": [ { "id": "Integer 0xe08", "value": size.to_string() } ],
    })
}

fn removed(id: &str) -> Value {
    json!({ "id": id, "@removed": { "reason": "deleted" } })
}

fn mime(n: u32) -> String {
    format!(
        "From: Ada <ada@example.test>\r\nTo: Me <me@example.test>\r\nSubject: message {n}\r\n\
         Message-ID: <m{n}@example.test>\r\nDate: Mon, 21 Sep 2026 07:59:00 +0000\r\n\
         Content-Type: text/plain; charset=utf-8\r\n\r\nthe body of message {n}\r\n"
    )
}

/// The well-known folders and a listing with children, answered for any request about folders.
fn folders(request: &Request, port: u16) -> Option<Answer> {
    let path = request.path();
    let known = |name: &str, id: &str| {
        (path == format!("/v1.0/me/mailFolders/{name}")).then(|| ok(json!({ "id": id })))
    };
    if let Some(answer) = known("inbox", "ID-INBOX")
        .or_else(|| known("sentitems", "ID-SENT"))
        .or_else(|| known("archive", "ID-ARCHIVE"))
    {
        return Some(answer);
    }
    if path.starts_with("/v1.0/me/mailFolders/")
        && ["drafts", "deleteditems", "junkemail"]
            .iter()
            .any(|n| path.ends_with(n))
    {
        return Some(status(
            "404 Not Found",
            r#"{"error":{"code":"ErrorItemNotFound"}}"#,
        ));
    }
    let folder = |id: &str, name: &str, children: u32| json!({ "id": id, "displayName": name, "childFolderCount": children, "isHidden": false });
    match path {
        "/v1.0/me/mailFolders" if !request.target.contains("skiptoken") => Some(ok(json!({
            "value": [
                folder("ID-INBOX", "Inbox", 1),
                folder("ID-SENT", "Sent Items", 0),
                folder("ID-ARCHIVE", "Archive", 0),
            ],
            "@odata.nextLink": format!("{}/mailFolders?$top=100&$skiptoken=2", me(port)),
        }))),
        "/v1.0/me/mailFolders" => Some(ok(json!({
            "value": [
                folder("ID-PROJECTS", "Projects", 1),
                { "id": "ID-HIDDEN", "displayName": "Hidden", "childFolderCount": 0, "isHidden": true },
            ],
        }))),
        "/v1.0/me/mailFolders/ID-INBOX/childFolders" => Some(ok(json!({
            "value": [ folder("ID-RECEIPTS", "Receipts", 0) ],
        }))),
        "/v1.0/me/mailFolders/ID-PROJECTS/childFolders" => Some(ok(json!({
            "value": [ folder("ID-2026", "2026", 0) ],
        }))),
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------------
// This side: an engine for an account that reads through Graph.

fn caps() -> AccountCaps {
    presets::receive_through_graph(presets::microsoft_preset("me@example.test", now()))
        .expected_caps
}

fn plan() -> AccountPlan {
    presets::receive_through_graph(presets::microsoft_preset("me@example.test", now())).plan
}

fn token(access: &str) -> Credential {
    Credential::OAuth {
        access: access.to_owned(),
        refresh: "r1".to_owned(),
        expires_at: now() + TimeDelta::try_hours(1).unwrap(),
    }
}

fn key(purpose: SecretPurpose) -> SecretKey {
    SecretKey {
        account: ACCOUNT,
        purpose,
    }
}

struct Account {
    engine: AccountEngine<OverHttp>,
    store: Arc<SqliteStore>,
    secrets: Arc<dyn Secrets>,
    _dir: tempfile::TempDir,
}

fn account(port: u16) -> Account {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::in_memory(dir.path()).unwrap());
    store
        .connection()
        .execute(
            "INSERT INTO accounts (id, address, plan, created_at)
             VALUES (?1, 'me@example.test', '{}', datetime('now'))",
            [ACCOUNT.to_string()],
        )
        .unwrap();
    let secrets: Arc<dyn Secrets> = Arc::new(MapSecrets::default());
    for purpose in [SecretPurpose::IncomingPassword, SecretPurpose::OAuthRefresh] {
        secrets.put(&key(purpose), &token("sign-in")).unwrap();
    }
    secrets
        .put(&key(SecretPurpose::OutgoingPassword), &token("graph-token"))
        .unwrap();
    let reader = Reader::new(ACCOUNT, caps()).unwrap().at(me(port));
    let engine = AccountEngine::new(
        ACCOUNT,
        plan(),
        OverHttp::new(caps()),
        store.clone(),
        secrets.clone(),
    )
    .with_graph_reader(reader);
    Account {
        engine,
        store,
        secrets,
        _dir: dir,
    }
}

fn inbox() -> MailboxRef {
    MailboxRef {
        account: ACCOUNT,
        path: "INBOX".to_owned(),
    }
}

fn cancel() -> (watch::Sender<bool>, mail_runtime::Cancel) {
    watch::channel(false)
}

/// The message whose `Message-ID` is `<m{n}@example.test>`.
fn held(store: &SqliteStore, n: u32) -> Option<Message> {
    let id: Option<String> = store
        .connection()
        .query_row(
            "SELECT id FROM messages WHERE rfc_message_id = ?1",
            [format!("m{n}@example.test")],
            |r| r.get(0),
        )
        .ok();
    let id = MessageId::from_uuid(id?.parse().unwrap());
    store.message(id).ok()
}

fn delta_requests(seen: &Seen) -> Vec<Request> {
    seen.lock()
        .unwrap()
        .iter()
        .filter(|r| r.path().ends_with("/messages/delta"))
        .cloned()
        .collect()
}

/// An inbox of three messages over two pages, then one change of each kind.
fn inbox_script() -> Script {
    Arc::new(|request, port| {
        if let Some(answer) = folders(request, port) {
            return answer;
        }
        let delta = format!("{}/mailFolders/inbox/messages/delta", me(port));
        match (
            request.path(),
            request.target.split('?').nth(1).unwrap_or(""),
        ) {
            ("/v1.0/me/mailFolders/inbox/messages/delta", query) if query.contains("select") => {
                ok(json!({
                    "value": [ entry("G1", 1, true, 2_000), entry("G2", 2, false, 900_000) ],
                    "@odata.nextLink": format!("{delta}?$skiptoken=page2"),
                }))
            }
            (_, "$skiptoken=page2") => ok(json!({
                "value": [ entry("G3", 3, false, 1_000) ],
                "@odata.deltaLink": format!("{delta}?$deltatoken=d1"),
            })),
            (_, "$deltatoken=d1") => ok(json!({
                "value": [ entry("G4", 4, false, 500), entry("G2", 2, true, 900_000), removed("G3") ],
                "@odata.deltaLink": format!("{delta}?$deltatoken=d2"),
            })),
            (_, "$deltatoken=d2") => ok(json!({
                "value": [],
                "@odata.deltaLink": format!("{delta}?$deltatoken=d2"),
            })),
            (path, _) if path.ends_with("/$value") => {
                let n: u32 = path
                    .trim_end_matches("/$value")
                    .rsplit('G')
                    .next()
                    .and_then(|n| n.parse().ok())
                    .unwrap_or(0);
                Answer {
                    status: "200 OK",
                    headers: String::new(),
                    body: mime(n),
                }
            }
            _ => status("404 Not Found", r#"{"error":{"code":"ErrorItemNotFound"}}"#),
        }
    })
}

// ---------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn folders_are_listed_with_their_children_and_the_well_known_ones_get_roles() {
    let (port, seen) = serve(inbox_script()).await;
    let mut it = account(port);
    let (_tx, mut cancel) = cancel();
    let folders = it.engine.refresh_folders(&mut cancel, now()).await.unwrap();

    let mut paths: Vec<&str> = folders.iter().map(|f| f.path.as_str()).collect();
    paths.sort();
    assert_eq!(
        paths,
        [
            "Archive",
            "INBOX",
            "INBOX/Receipts",
            "Projects",
            "Projects/2026",
            "Sent Items"
        ],
        "the hidden folder is left out, and the second page and the children are followed"
    );
    let sent = folders.iter().find(|f| f.path == "Sent Items").unwrap();
    assert_eq!(sent.special, Some(SpecialUse::Sent));

    let caps: AccountCaps = serde_json::from_str(
        &it.store
            .connection()
            .query_row(
                "SELECT caps FROM account_caps WHERE account = ?1",
                [ACCOUNT.to_string()],
                |r| r.get::<_, String>(0),
            )
            .unwrap(),
    )
    .unwrap();
    assert_eq!(caps.folders.path(MailboxRole::Sent), Some("Sent Items"));
    assert_eq!(caps.folders.path(MailboxRole::Archive), Some("Archive"));
    assert_eq!(
        caps.observed_at,
        now(),
        "the listing dates the capabilities"
    );
    for request in seen.lock().unwrap().iter() {
        assert_eq!(
            request.header("authorization"),
            Some("Bearer graph-token"),
            "{}",
            request.target
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_first_sync_follows_the_pages_and_keeps_the_delta_link() {
    let (port, seen) = serve(inbox_script()).await;
    let mut it = account(port);
    let (_tx, mut cancel) = cancel();
    let report = it
        .engine
        .sync(&inbox(), &mut cancel, now(), 200)
        .await
        .unwrap();

    assert_eq!(report.headers_fetched, 3);
    assert_eq!(report.arrived.len(), 3);
    let first = held(&it.store, 1).expect("message 1 is stored");
    assert_eq!(first.subject, "message 1");
    assert_eq!(first.read, ReadState::Read, "the delta's isRead is applied");
    assert_eq!(
        first.body,
        Body::Absent,
        "headers first; bodies behind them"
    );
    assert_eq!(first.mailbox, MailboxRole::Inbox);
    assert_eq!(held(&it.store, 2).unwrap().read, ReadState::Unread);
    assert_eq!(
        it.store.cursor(&inbox()).unwrap(),
        Some(SyncCursor::Graph {
            delta_link: format!(
                "{}/mailFolders/inbox/messages/delta?$deltatoken=d1",
                me(port)
            ),
        })
    );

    let deltas = delta_requests(&seen);
    assert_eq!(deltas.len(), 2);
    assert!(
        deltas[0].target.contains("select=") && deltas[0].target.contains("expand="),
        "the first request names what it wants: {}",
        deltas[0].target
    );
    assert_eq!(
        deltas[0].header("prefer"),
        Some("odata.maxpagesize=100"),
        "pages are sized"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_later_sync_takes_what_is_new_what_changed_and_what_left() {
    let (port, _seen) = serve(inbox_script()).await;
    let mut it = account(port);
    let (_tx, mut cancel) = cancel();
    it.engine
        .sync(&inbox(), &mut cancel, now(), 200)
        .await
        .unwrap();
    let report = it
        .engine
        .sync(&inbox(), &mut cancel, now(), 200)
        .await
        .unwrap();

    assert_eq!(report.headers_fetched, 1, "only message 4 is new");
    assert!(held(&it.store, 4).is_some());
    assert_eq!(
        held(&it.store, 2).unwrap().read,
        ReadState::Read,
        "read on another device"
    );
    assert!(
        held(&it.store, 3).is_none(),
        "removed from the only folder that held it"
    );
    assert!(matches!(
        it.store.cursor(&inbox()).unwrap(),
        Some(SyncCursor::Graph { delta_link }) if delta_link.ends_with("d2")
    ));

    // And a pass with nothing to report changes nothing.
    let quiet = it
        .engine
        .sync(&inbox(), &mut cancel, now(), 200)
        .await
        .unwrap();
    assert_eq!(quiet.headers_fetched, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bodies_arrive_as_the_messages_own_mime_smallest_first() {
    let (port, seen) = serve(inbox_script()).await;
    let mut it = account(port);
    let (_tx, mut cancel) = cancel();
    it.engine
        .sync(&inbox(), &mut cancel, now(), 200)
        .await
        .unwrap();
    let report = it
        .engine
        .fetch_bodies(&inbox(), &mut cancel, now(), 100)
        .await
        .unwrap();
    assert_eq!(report.bodies_fetched, 3);

    let first = held(&it.store, 1).unwrap();
    let Body::Present { text, .. } = &first.body else {
        panic!("{:?}", first.body)
    };
    assert!(
        text.as_deref()
            .unwrap_or("")
            .contains("the body of message 1"),
        "{text:?}"
    );
    let fetched: Vec<String> = seen
        .lock()
        .unwrap()
        .iter()
        .filter(|r| r.path().ends_with("/$value"))
        .map(|r| r.path().to_owned())
        .collect();
    assert_eq!(
        fetched.last().map(String::as_str),
        Some("/v1.0/me/messages/G2/$value"),
        "the 900 KB message waits for the small ones: {fetched:?}"
    );
}

/// Queue `intent` as the window would, with an empty undo.
fn queue(store: &SqliteStore, intent: RemoteIntent) {
    store
        .enqueue(
            ACCOUNT,
            intent,
            &Patch {
                id: ChangeId::generate(),
                changes: Vec::new(),
            },
            now(),
        )
        .unwrap()
        .expect("queued");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn marking_read_here_patches_is_read_there() {
    let (port, seen) = serve(Arc::new(|request, port| {
        if request.method == "PATCH" {
            return ok(json!({ "id": "G1" }));
        }
        inbox_script()(request, port)
    }))
    .await;
    let mut it = account(port);
    let (_tx, mut cancel) = cancel();
    it.engine
        .sync(&inbox(), &mut cancel, now(), 200)
        .await
        .unwrap();
    let message = held(&it.store, 2).unwrap();
    queue(
        &it.store,
        RemoteIntent::SetFlags {
            messages: vec![message.id],
            read: Some(ReadState::Read),
            star: Some(Star::Starred),
        },
    );
    let drained = it.engine.drain_outbox(&mut cancel, now()).await.unwrap();
    assert_eq!(drained.outbox_settled, 1, "{:?}", drained.needs_attention);

    let patches: Vec<Request> = seen
        .lock()
        .unwrap()
        .iter()
        .filter(|r| r.method == "PATCH")
        .cloned()
        .collect();
    assert_eq!(patches.len(), 1);
    assert_eq!(patches[0].path(), "/v1.0/me/messages/G2");
    assert_eq!(
        patches[0].json(),
        json!({ "isRead": true, "flag": { "flagStatus": "flagged" } })
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_move_takes_the_new_id_graph_gives_the_message() {
    let (port, seen) = serve(Arc::new(|request, port| {
        if request.method == "POST" && request.path().ends_with("/move") {
            return Answer {
                status: "201 Created",
                headers: String::new(),
                body: json!({ "id": "G2-ARCHIVED", "parentFolderId": "ID-ARCHIVE" }).to_string(),
            };
        }
        if request.method == "PATCH" {
            return ok(json!({}));
        }
        inbox_script()(request, port)
    }))
    .await;
    let mut it = account(port);
    let (_tx, mut cancel) = cancel();
    it.engine.refresh_folders(&mut cancel, now()).await.unwrap();
    it.engine
        .sync(&inbox(), &mut cancel, now(), 200)
        .await
        .unwrap();
    let message = held(&it.store, 2).unwrap();
    queue(
        &it.store,
        RemoteIntent::SetMailbox {
            messages: vec![message.id],
            role: MailboxRole::Archive,
        },
    );
    it.engine.drain_outbox(&mut cancel, now()).await.unwrap();

    let moved = seen
        .lock()
        .unwrap()
        .iter()
        .find(|r| r.path().ends_with("/move"))
        .cloned()
        .expect("a move was asked for");
    assert_eq!(moved.path(), "/v1.0/me/messages/G2/move");
    assert_eq!(moved.json(), json!({ "destinationId": "archive" }));
    assert_eq!(
        it.store.remotes_of(message.id).unwrap(),
        vec![RemoteRef::Graph {
            mailbox: "Archive".to_owned(),
            id: "G2-ARCHIVED".to_owned(),
        }],
        "the message is addressed where it now is"
    );

    // So the next change goes to the new id, not the one that no longer exists.
    queue(
        &it.store,
        RemoteIntent::SetFlags {
            messages: vec![message.id],
            read: Some(ReadState::Read),
            star: None,
        },
    );
    it.engine.drain_outbox(&mut cancel, now()).await.unwrap();
    let patched = seen
        .lock()
        .unwrap()
        .iter()
        .rfind(|r| r.method == "PATCH")
        .cloned()
        .unwrap();
    assert_eq!(patched.path(), "/v1.0/me/messages/G2-ARCHIVED");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_second_move_queued_before_the_first_went_uses_the_id_the_first_returned() {
    // Graph gives a moved message a new id, and the old one then names nothing. Both moves are
    // queued before either is sent, so only addressing each as it leaves finds the new one.
    let (port, seen) = serve(Arc::new(|request, port| {
        if request.method == "POST" && request.path().ends_with("/move") {
            let id = if request.path().contains("/G2/") {
                "G2-ARCHIVED"
            } else {
                "G2-TRASHED"
            };
            return Answer {
                status: "201 Created",
                headers: String::new(),
                body: json!({ "id": id }).to_string(),
            };
        }
        inbox_script()(request, port)
    }))
    .await;
    let mut it = account(port);
    let (_tx, mut cancel) = cancel();
    it.engine.refresh_folders(&mut cancel, now()).await.unwrap();
    it.engine
        .sync(&inbox(), &mut cancel, now(), 200)
        .await
        .unwrap();
    let message = held(&it.store, 2).unwrap();
    for role in [MailboxRole::Archive, MailboxRole::Trash] {
        queue(
            &it.store,
            RemoteIntent::SetMailbox {
                messages: vec![message.id],
                role,
            },
        );
    }
    let drained = it.engine.drain_outbox(&mut cancel, now()).await.unwrap();
    assert_eq!(drained.outbox_settled, 2, "{:?}", drained.needs_attention);

    let moves: Vec<String> = seen
        .lock()
        .unwrap()
        .iter()
        .filter(|r| r.path().ends_with("/move"))
        .map(|r| r.path().to_owned())
        .collect();
    assert_eq!(
        moves,
        vec![
            "/v1.0/me/messages/G2/move".to_owned(),
            "/v1.0/me/messages/G2-ARCHIVED/move".to_owned(),
        ]
    );
    assert!(
        matches!(
            it.store.remotes_of(message.id).unwrap().as_slice(),
            [RemoteRef::Graph { id, .. }] if id == "G2-TRASHED"
        ),
        "{:?}",
        it.store.remotes_of(message.id)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn throttling_waits_as_long_as_graph_asks() {
    let (port, _seen) = serve(Arc::new(|request, port| {
        if request.path().ends_with("/messages/delta") {
            return Answer {
                status: "429 Too Many Requests",
                headers: "retry-after: 7\r\n".to_owned(),
                body: r#"{"error":{"code":"ApplicationThrottled","message":"slow down"}}"#
                    .to_owned(),
            };
        }
        inbox_script()(request, port)
    }))
    .await;
    let mut it = account(port);
    let (_tx, mut cancel) = cancel();
    let e = it
        .engine
        .sync(&inbox(), &mut cancel, now(), 200)
        .await
        .expect_err("throttled");
    assert_eq!(e.retry(), Retry::After(std::time::Duration::from_secs(7)));
    assert!(e.to_string().contains("ApplicationThrottled"), "{e}");
    assert_eq!(it.store.cursor(&inbox()).unwrap(), None, "no place kept");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_refused_token_is_renewed_once_with_both_graph_permissions() {
    // Graph takes only the renewed token; the issuer mints it.
    let (port, seen) = serve(Arc::new(|request, port| {
        if request.header("authorization") != Some("Bearer renewed") {
            return status(
                "401 Unauthorized",
                r#"{"error":{"code":"InvalidAuthenticationToken"}}"#,
            );
        }
        inbox_script()(request, port)
    }))
    .await;
    let (issuer, asked) = serve(Arc::new(|_, _| {
        ok(json!({ "access_token": "renewed", "token_type": "Bearer", "expires_in": 3600 }))
    }))
    .await;
    let registration = Registration::new(OAuthIssuer::Microsoft, "client-id").at(Endpoints {
        auth: format!("http://127.0.0.1:{issuer}/authorize"),
        token: format!("http://127.0.0.1:{issuer}/token"),
    });
    let it = account(port);
    let renewal = Renewal::new(
        ACCOUNT,
        &plan(),
        registration,
        it.secrets.clone(),
        Held::new(token("sign-in")),
    )
    .unwrap()
    .with_clock(Arc::new(now));
    let mut engine = it.engine.with_renewal(renewal);
    let (_tx, mut cancel) = cancel();
    let report = engine
        .sync(&inbox(), &mut cancel, now(), 200)
        .await
        .expect("the renewed token is accepted");
    assert_eq!(report.headers_fetched, 3);

    let asked = asked.lock().unwrap().clone();
    assert_eq!(asked.len(), 1, "renewed once");
    let form: Vec<(String, String)> = url::form_urlencoded::parse(asked[0].body.as_bytes())
        .into_owned()
        .collect();
    let scope = form
        .iter()
        .find(|(k, _)| k == "scope")
        .map(|(_, v)| v.clone())
        .unwrap_or_default();
    assert!(
        scope.contains(presets::GRAPH_WRITE_SCOPE) && scope.contains(presets::GRAPH_SEND_SCOPE),
        "one token for reading and sending: {scope}"
    );
    let stored = it
        .secrets
        .get(&key(SecretPurpose::OutgoingPassword))
        .unwrap();
    assert!(matches!(stored, Credential::OAuth { ref access, .. } if access == "renewed"));
    let first = seen.lock().unwrap()[0].clone();
    assert_eq!(first.header("authorization"), Some("Bearer graph-token"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn nothing_is_ever_deleted_for_good() {
    let (port, seen) = serve(inbox_script()).await;
    let mut it = account(port);
    let (_tx, mut cancel) = cancel();
    it.engine
        .sync(&inbox(), &mut cancel, now(), 200)
        .await
        .unwrap();
    let message = held(&it.store, 1).unwrap();
    // No intent produces an expunge for an account that forbids it; ask the reader directly.
    let mut reader = Reader::new(ACCOUNT, caps()).unwrap().at(me(port));
    let e = reader
        .run(
            ProtoOp::Expunge {
                remotes: it.store.remotes_of(message.id).unwrap(),
            },
            "graph-token",
            None,
        )
        .await
        .expect_err("refused");
    assert!(matches!(e.retry(), Retry::Fatal(_)), "{e}");
    assert!(
        !seen.lock().unwrap().iter().any(|r| r.method == "DELETE"),
        "nothing was asked to delete anything"
    );
}
