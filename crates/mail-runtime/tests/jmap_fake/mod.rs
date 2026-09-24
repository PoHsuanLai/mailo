//! A JMAP server small enough to read, standing in for a real one on a loopback port.
//!
//! It speaks what the client uses and no more: the session resource, `Mailbox/get` and
//! `Mailbox/changes`, `Email/query`, `Email/get` (with back-references), `Email/changes`,
//! `Email/set`, `Email/import`, `EmailSubmission/set` with `onSuccessUpdateEmail`,
//! `Identity/get`, blob download and upload, and an event source. Written from RFC 8620 and
//! RFC 8621, not from any server's code. HTTP/1.1, one request per connection.

#![allow(dead_code)]

use serde_json::{Map, Value, json};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::sync::broadcast;

pub const ACCOUNT_ID: &str = "A1";
pub const USER: &str = "me@example.test";
pub const PASSWORD: &str = "hunter2";

/// One email the server holds.
#[derive(Debug, Clone)]
pub struct Email {
    pub id: String,
    pub blob: String,
    pub mailboxes: Vec<String>,
    pub keywords: Vec<String>,
    pub received: String,
}

/// What a change log entry records: an email created, updated or destroyed at a state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Created,
    Updated,
    Destroyed,
}

/// A submission the server accepted.
#[derive(Debug, Clone)]
pub struct Submitted {
    pub identity: String,
    pub email: String,
    pub mail_from: String,
    pub rcpt_to: Vec<String>,
}

#[derive(Debug, Default)]
pub struct State {
    pub mailboxes: Vec<(String, String, Option<String>, Option<String>)>,
    pub emails: Vec<Email>,
    pub blobs: Vec<(String, Vec<u8>)>,
    pub state: u64,
    pub log: Vec<(u64, String, Kind)>,
    pub submitted: Vec<Submitted>,
    /// Every method call received, by name, with its arguments.
    pub calls: Vec<(String, Value)>,
    next: u64,
}

impl State {
    fn fresh(&mut self, prefix: &str) -> String {
        self.next += 1;
        format!("{prefix}{}", self.next)
    }

    fn bump(&mut self, id: &str, kind: Kind) {
        self.state += 1;
        self.log.push((self.state, id.to_owned(), kind));
    }

    /// Add an email from its raw bytes, as delivery would.
    pub fn deliver(
        &mut self,
        raw: &[u8],
        mailboxes: &[&str],
        keywords: &[&str],
        received: &str,
    ) -> String {
        let id = self.fresh("M");
        let blob = self.fresh("B");
        self.blobs.push((blob.clone(), raw.to_vec()));
        self.emails.push(Email {
            id: id.clone(),
            blob,
            mailboxes: mailboxes.iter().map(|s| (*s).to_owned()).collect(),
            keywords: keywords.iter().map(|s| (*s).to_owned()).collect(),
            received: received.to_owned(),
        });
        self.bump(&id, Kind::Created);
        id
    }

    pub fn email(&self, id: &str) -> Option<&Email> {
        self.emails.iter().find(|e| e.id == id)
    }

    /// Change an email as another client would.
    pub fn touch(&mut self, id: &str, change: impl FnOnce(&mut Email)) {
        if let Some(email) = self.emails.iter_mut().find(|e| e.id == id) {
            change(email);
        }
        self.bump(id, Kind::Updated);
    }

    pub fn destroy(&mut self, id: &str) {
        self.emails.retain(|e| e.id != id);
        self.bump(id, Kind::Destroyed);
    }

    pub fn email_state(&self) -> String {
        format!("e{}", self.state)
    }

    fn raw(&self, blob: &str) -> Option<&[u8]> {
        self.blobs
            .iter()
            .find(|(b, _)| b == blob)
            .map(|(_, raw)| raw.as_slice())
    }
}

/// The fake: its state, where it listens, and a way to push.
#[derive(Clone)]
pub struct Fake {
    pub state: Arc<Mutex<State>>,
    pub port: u16,
    push: broadcast::Sender<String>,
}

impl Fake {
    pub fn session_url(&self) -> String {
        format!("http://127.0.0.1:{}/.well-known/jmap", self.port)
    }

    /// Tell every open event stream that the email state is now `state`.
    pub fn push_state(&self, state: &str) {
        let data = json!({
            "@type": "StateChange",
            "changed": { ACCOUNT_ID: { "Email": state } }
        });
        let _ = self
            .push
            .send(format!("event: state\r\ndata: {data}\r\n\r\n"));
    }

    pub fn calls(&self, name: &str) -> Vec<Value> {
        self.state
            .lock()
            .unwrap()
            .calls
            .iter()
            .filter(|(n, _)| n == name)
            .map(|(_, a)| a.clone())
            .collect()
    }
}

/// Start the fake with the usual mailboxes.
pub async fn start() -> Fake {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut state = State::default();
    for (id, name, parent, role) in [
        ("mbI", "Inbox", None, Some("inbox")),
        ("mbA", "Archive", None, Some("archive")),
        ("mbD", "Drafts", None, Some("drafts")),
        ("mbS", "Sent", None, Some("sent")),
        ("mbT", "Trash", None, Some("trash")),
        ("mbJ", "Junk", None, Some("junk")),
        ("mbW", "Work", None, None),
    ] {
        state.mailboxes.push((
            id.to_owned(),
            name.to_owned(),
            parent.map(str::to_owned),
            role.map(str::to_owned),
        ));
    }
    let (push, _) = broadcast::channel(16);
    let fake = Fake {
        state: Arc::new(Mutex::new(state)),
        port,
        push,
    };
    let served = fake.clone();
    tokio::spawn(async move {
        while let Ok((sock, _)) = listener.accept().await {
            let fake = served.clone();
            tokio::spawn(async move { connection(fake, sock).await });
        }
    });
    fake
}

async fn connection(fake: Fake, sock: tokio::net::TcpStream) {
    let mut reader = BufReader::new(sock);
    let mut line = String::new();
    if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
        return;
    }
    let mut length = 0usize;
    let mut authorization = String::new();
    loop {
        let mut header = String::new();
        reader.read_line(&mut header).await.unwrap();
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                length = value.trim().parse().unwrap();
            }
            if name.eq_ignore_ascii_case("authorization") {
                authorization = value.trim().to_owned();
            }
        }
    }
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body).await.unwrap();
    let mut sock = reader.into_inner();

    use base64::Engine as _;
    let expected = format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("{USER}:{PASSWORD}"))
    );
    if authorization != expected {
        return respond(&mut sock, "401 Unauthorized", "application/json", b"{}").await;
    }

    let mut words = line.split_whitespace();
    let (method, target) = (words.next().unwrap_or(""), words.next().unwrap_or(""));
    let path = target.split('?').next().unwrap_or("");
    let base = format!("http://127.0.0.1:{}", fake.port);
    match (method, path) {
        ("GET", "/.well-known/jmap") => {
            let session = json!({
                "capabilities": {
                    "urn:ietf:params:jmap:core": {
                        "maxSizeUpload": 50_000_000, "maxConcurrentUpload": 4,
                        "maxSizeRequest": 10_000_000, "maxConcurrentRequest": 4,
                        "maxCallsInRequest": 16, "maxObjectsInGet": 2,
                        "maxObjectsInSet": 100, "collationAlgorithms": []
                    },
                    "urn:ietf:params:jmap:mail": {},
                    "urn:ietf:params:jmap:submission": {}
                },
                "accounts": { ACCOUNT_ID: {
                    "name": USER, "isPersonal": true, "isReadOnly": false,
                    "accountCapabilities": {
                        "urn:ietf:params:jmap:mail": {},
                        "urn:ietf:params:jmap:submission": {}
                    }
                } },
                "primaryAccounts": {
                    "urn:ietf:params:jmap:mail": ACCOUNT_ID,
                    "urn:ietf:params:jmap:submission": ACCOUNT_ID
                },
                "username": USER,
                // Relative, as a session may give them: resolved against where it was found.
                "apiUrl": "/api/",
                "downloadUrl": format!("{base}/download/{{accountId}}/{{blobId}}/{{name}}?accept={{type}}"),
                "uploadUrl": "/upload/{accountId}/",
                "eventSourceUrl": format!("{base}/events?types={{types}}&closeafter={{closeafter}}&ping={{ping}}"),
                "state": "s1"
            });
            respond(
                &mut sock,
                "200 OK",
                "application/json",
                session.to_string().as_bytes(),
            )
            .await
        }
        ("POST", "/api/") => {
            let request: Value = serde_json::from_slice(&body).unwrap();
            let answer = api(&fake, &request);
            respond(
                &mut sock,
                "200 OK",
                "application/json",
                answer.to_string().as_bytes(),
            )
            .await
        }
        ("GET", p) if p.starts_with("/download/") => {
            let blob = p.split('/').nth(3).unwrap_or("").to_owned();
            let raw = fake.state.lock().unwrap().raw(&blob).map(<[u8]>::to_vec);
            match raw {
                Some(raw) => respond(&mut sock, "200 OK", "message/rfc822", &raw).await,
                None => respond(&mut sock, "404 Not Found", "text/plain", b"").await,
            }
        }
        ("POST", p) if p.starts_with("/upload/") => {
            let blob = {
                let mut state = fake.state.lock().unwrap();
                let blob = state.fresh("U");
                state.blobs.push((blob.clone(), body.clone()));
                blob
            };
            let answer = json!({ "accountId": ACCOUNT_ID, "blobId": blob, "type": "message/rfc822", "size": body.len() });
            respond(
                &mut sock,
                "201 Created",
                "application/json",
                answer.to_string().as_bytes(),
            )
            .await
        }
        ("GET", "/events") => {
            let mut events = fake.push.subscribe();
            let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n";
            if sock.write_all(head.as_bytes()).await.is_err() {
                return;
            }
            // The current state at once, as a server may send on connect.
            let now = fake.state.lock().unwrap().email_state();
            let data =
                json!({ "@type": "StateChange", "changed": { ACCOUNT_ID: { "Email": now } } });
            let _ = sock
                .write_all(format!("event: state\r\ndata: {data}\r\n\r\n").as_bytes())
                .await;
            while let Ok(event) = events.recv().await {
                if sock.write_all(event.as_bytes()).await.is_err() {
                    return;
                }
            }
        }
        _ => respond(&mut sock, "404 Not Found", "text/plain", b"").await,
    }
}

async fn respond(sock: &mut tokio::net::TcpStream, status: &str, kind: &str, body: &[u8]) {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = sock.write_all(head.as_bytes()).await;
    let _ = sock.write_all(body).await;
    let _ = sock.shutdown().await;
}

/// Answer one API request, call by call.
fn api(fake: &Fake, request: &Value) -> Value {
    let mut state = fake.state.lock().unwrap();
    let mut answers: Vec<Value> = Vec::new();
    let mut created_ids: Vec<(String, String)> = Vec::new();
    for call in request["methodCalls"].as_array().unwrap() {
        let name = call[0].as_str().unwrap().to_owned();
        let mut args = call[1].clone();
        let id = call[2].as_str().unwrap().to_owned();
        state.calls.push((name.clone(), args.clone()));
        resolve_references(&mut args, &answers);
        let produced = method(&mut state, &name, &args, &mut created_ids);
        for (answer_name, answer_args) in produced {
            answers.push(json!([answer_name, answer_args, id]));
        }
    }
    json!({ "methodResponses": answers, "sessionState": "s1" })
}

/// Replace `#ids` back-references with the values they point at (RFC 8620 §3.7).
fn resolve_references(args: &mut Value, answers: &[Value]) {
    let Some(map) = args.as_object_mut() else {
        return;
    };
    let refs: Vec<String> = map.keys().filter(|k| k.starts_with('#')).cloned().collect();
    for key in refs {
        let reference = map.remove(&key).unwrap();
        let (call, path) = (
            reference["resultOf"].as_str().unwrap(),
            reference["path"].as_str().unwrap(),
        );
        let answer = answers
            .iter()
            .find(|a| a[2] == call && a[0] == reference["name"])
            .map(|a| a[1].pointer(path).cloned().unwrap_or(Value::Null))
            .unwrap_or(Value::Null);
        map.insert(key[1..].to_owned(), answer);
    }
}

fn error(kind: &str) -> Vec<(String, Value)> {
    vec![("error".to_owned(), json!({ "type": kind }))]
}

fn method(
    state: &mut State,
    name: &str,
    args: &Value,
    created: &mut Vec<(String, String)>,
) -> Vec<(String, Value)> {
    let one = |args: Value| vec![(name.to_owned(), args)];
    match name {
        "Mailbox/get" => one(json!({
            "accountId": ACCOUNT_ID,
            "state": "m1",
            "list": state.mailboxes.iter().map(|(id, n, parent, role)| json!({
                "id": id, "name": n, "parentId": parent, "role": role,
                "sortOrder": 0, "isSubscribed": true
            })).collect::<Vec<_>>(),
            "notFound": []
        })),
        "Mailbox/changes" => one(json!({
            "accountId": ACCOUNT_ID, "oldState": args["sinceState"], "newState": "m1",
            "hasMoreChanges": false, "created": [], "updated": [], "destroyed": []
        })),
        "Email/query" => {
            let excluded: Vec<String> = args["filter"]["inMailboxOtherThan"]
                .as_array()
                .map(|a| a.iter().map(|v| v.as_str().unwrap().to_owned()).collect())
                .unwrap_or_default();
            let mut matching: Vec<&Email> = state
                .emails
                .iter()
                .filter(|e| e.mailboxes.iter().any(|m| !excluded.contains(m)))
                .collect();
            matching.sort_by(|a, b| b.received.cmp(&a.received));
            let position = args["position"].as_u64().unwrap_or(0) as usize;
            let limit = args["limit"].as_u64().unwrap_or(100) as usize;
            let ids: Vec<&str> = matching
                .iter()
                .skip(position)
                // Pages of two at most, so a client that does not page is caught.
                .take(limit.min(2))
                .map(|e| e.id.as_str())
                .collect();
            one(json!({
                "accountId": ACCOUNT_ID, "queryState": "q", "canCalculateChanges": false,
                "position": position, "total": matching.len(), "ids": ids
            }))
        }
        "Email/get" => {
            let wanted: Vec<String> = args["ids"]
                .as_array()
                .map(|a| a.iter().map(|v| v.as_str().unwrap().to_owned()).collect())
                .unwrap_or_default();
            let properties: Vec<String> = args["properties"]
                .as_array()
                .map(|a| a.iter().map(|v| v.as_str().unwrap().to_owned()).collect())
                .unwrap_or_default();
            let mut list = Vec::new();
            let mut not_found = Vec::new();
            for id in &wanted {
                match state.email(id) {
                    Some(email) => list.push(describe(state, email, &properties)),
                    None => not_found.push(id.clone()),
                }
            }
            one(json!({
                "accountId": ACCOUNT_ID, "state": state.email_state(),
                "list": list, "notFound": not_found
            }))
        }
        "Email/changes" => {
            let since: u64 = args["sinceState"].as_str().unwrap()[1..].parse().unwrap();
            if since > state.state {
                return error("cannotCalculateChanges");
            }
            let mut created_ids = Vec::new();
            let mut updated = Vec::new();
            let mut destroyed = Vec::new();
            for (at, id, kind) in &state.log {
                if *at <= since {
                    continue;
                }
                match kind {
                    Kind::Created => created_ids.push(id.clone()),
                    Kind::Updated if !created_ids.contains(id) && !updated.contains(id) => {
                        updated.push(id.clone())
                    }
                    Kind::Updated => {}
                    Kind::Destroyed => {
                        created_ids.retain(|c| c != id);
                        updated.retain(|u| u != id);
                        destroyed.push(id.clone());
                    }
                }
            }
            one(json!({
                "accountId": ACCOUNT_ID, "oldState": args["sinceState"],
                "newState": state.email_state(), "hasMoreChanges": false,
                "created": created_ids, "updated": updated, "destroyed": destroyed
            }))
        }
        "Email/set" => {
            let mut updated = Map::new();
            let mut not_updated = Map::new();
            for (id, patch) in args["update"].as_object().cloned().unwrap_or_default() {
                if state.email(&id).is_none() {
                    not_updated.insert(id, json!({ "type": "notFound" }));
                    continue;
                }
                state.touch(&id, |email| apply_patch(email, &patch));
                updated.insert(id, Value::Null);
            }
            let mut destroyed = Vec::new();
            for id in args["destroy"].as_array().cloned().unwrap_or_default() {
                let id = id.as_str().unwrap().to_owned();
                state.destroy(&id);
                destroyed.push(id);
            }
            one(json!({
                "accountId": ACCOUNT_ID, "newState": state.email_state(),
                "updated": updated, "notUpdated": not_updated, "destroyed": destroyed
            }))
        }
        "Email/import" => {
            let mut made = Map::new();
            for (creation, email) in args["emails"].as_object().cloned().unwrap_or_default() {
                let blob = email["blobId"].as_str().unwrap().to_owned();
                let raw = state.raw(&blob).unwrap().to_vec();
                let mailboxes: Vec<String> = email["mailboxIds"]
                    .as_object()
                    .unwrap()
                    .keys()
                    .cloned()
                    .collect();
                let keywords: Vec<String> = email["keywords"]
                    .as_object()
                    .map(|k| k.keys().cloned().collect())
                    .unwrap_or_default();
                let id = state.deliver(
                    &raw,
                    &mailboxes.iter().map(String::as_str).collect::<Vec<_>>(),
                    &keywords.iter().map(String::as_str).collect::<Vec<_>>(),
                    "2026-09-24T12:00:00Z",
                );
                created.push((creation.clone(), id.clone()));
                made.insert(
                    creation,
                    json!({ "id": id, "blobId": blob, "threadId": "T", "size": raw.len() }),
                );
            }
            one(
                json!({ "accountId": ACCOUNT_ID, "newState": state.email_state(), "created": made }),
            )
        }
        "EmailSubmission/set" => {
            let mut made = Map::new();
            let mut done: Vec<(String, String)> = Vec::new();
            for (creation, submission) in args["create"].as_object().cloned().unwrap_or_default() {
                let email_ref = submission["emailId"].as_str().unwrap();
                let email = match email_ref.strip_prefix('#') {
                    Some(c) => created
                        .iter()
                        .find(|(k, _)| k == c)
                        .map(|(_, v)| v.clone())
                        .unwrap(),
                    None => email_ref.to_owned(),
                };
                state.submitted.push(Submitted {
                    identity: submission["identityId"].as_str().unwrap().to_owned(),
                    email: email.clone(),
                    mail_from: submission["envelope"]["mailFrom"]["email"]
                        .as_str()
                        .unwrap()
                        .to_owned(),
                    rcpt_to: submission["envelope"]["rcptTo"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|r| r["email"].as_str().unwrap().to_owned())
                        .collect(),
                });
                let sub = state.fresh("S");
                made.insert(creation.clone(), json!({ "id": sub }));
                done.push((creation, email));
            }
            let mut out = one(json!({ "accountId": ACCOUNT_ID, "created": made }));
            // onSuccessUpdateEmail, answered as the implicit Email/set RFC 8621 §7.5 describes.
            let mut updated = Map::new();
            for (key, patch) in args["onSuccessUpdateEmail"]
                .as_object()
                .cloned()
                .unwrap_or_default()
            {
                let creation = key.trim_start_matches('#');
                if let Some((_, email)) = done.iter().find(|(c, _)| c == creation) {
                    state.touch(email, |e| apply_patch(e, &patch));
                    updated.insert(email.clone(), Value::Null);
                }
            }
            out.push((
                "Email/set".to_owned(),
                json!({ "accountId": ACCOUNT_ID, "updated": updated }),
            ));
            out
        }
        "Identity/get" => one(json!({
            "accountId": ACCOUNT_ID, "state": "i1",
            "list": [{ "id": "I1", "name": "Me", "email": USER }], "notFound": []
        })),
        _ => error("unknownMethod"),
    }
}

fn apply_patch(email: &mut Email, patch: &Value) {
    for (path, value) in patch.as_object().unwrap() {
        let (list, key) = match path.split_once('/') {
            Some(("keywords", k)) => (&mut email.keywords, k),
            Some(("mailboxIds", m)) => (&mut email.mailboxes, m),
            _ => continue,
        };
        list.retain(|x| x != key);
        if value == &json!(true) {
            list.push(key.to_owned());
        }
    }
}

/// An email with the properties asked for.
fn describe(state: &State, email: &Email, properties: &[String]) -> Value {
    let raw = state.raw(&email.blob).unwrap_or(b"");
    let mut out = Map::new();
    for property in properties {
        let value = match property.as_str() {
            "id" => json!(email.id),
            "blobId" => json!(email.blob),
            "threadId" => json!(format!("T{}", email.id)),
            "mailboxIds" => json!(
                email
                    .mailboxes
                    .iter()
                    .map(|m| (m.clone(), json!(true)))
                    .collect::<Map<_, _>>()
            ),
            "keywords" => json!(
                email
                    .keywords
                    .iter()
                    .map(|k| (k.clone(), json!(true)))
                    .collect::<Map<_, _>>()
            ),
            "size" => json!(raw.len()),
            "receivedAt" => json!(email.received),
            "headers" => json!(
                headers(raw)
                    .into_iter()
                    .map(|(n, v)| json!({ "name": n, "value": v }))
                    .collect::<Vec<_>>()
            ),
            "preview" => json!(""),
            "hasAttachment" => json!(false),
            _ => Value::Null,
        };
        out.insert(property.clone(), value);
    }
    Value::Object(out)
}

/// Header fields in their raw form: name, and everything after the colon with folding kept.
fn headers(raw: &[u8]) -> Vec<(String, String)> {
    let text = String::from_utf8_lossy(raw);
    let block = text.split("\r\n\r\n").next().unwrap_or("");
    let mut out: Vec<(String, String)> = Vec::new();
    for line in block.split("\r\n") {
        if line.starts_with(' ') || line.starts_with('\t') {
            if let Some(last) = out.last_mut() {
                last.1.push_str("\r\n");
                last.1.push_str(line);
            }
        } else if let Some((name, value)) = line.split_once(':') {
            out.push((name.to_owned(), value.to_owned()));
        }
    }
    out
}
