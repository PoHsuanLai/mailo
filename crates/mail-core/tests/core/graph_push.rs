//! Push for a Microsoft 365 account through a relay: Graph's subscriptions and the relay, both
//! played by listeners on loopback in this process.
//!
//! The fake Graph answers `POST /subscriptions`, `PATCH` and `DELETE` of one, and forgets one
//! when told to; the fake relay registers keys (`POST /graph/register`, keeping only the hashes it
//! is sent) and upgrades `GET /graph/listen/<key>` to a WebSocket for the bearer whose hash it
//! keeps, through which the test sends what the relay would forward. What is under test is the
//! watch: that it registers and subscribes as the relay and Graph expect, that an alert is heard
//! as mail (which the scheduler starts a pass for), that a subscription near its end is renewed
//! and one Graph removed is made again, and that an account with no relay, or an unreachable
//! one, is polled as before. Nothing reaches the network beyond loopback, and the keyring is a
//! `MemorySecrets`.

use crate::support::{account, now};
use mail_core::fetch::{Event, Trigger};
use mail_core::sync::live::{Heard, Watch};
use mail_core::sync::{graph_push, live};
use mail_domain::*;
use mail_runtime::graph::push::{client_state, digest, secret_key};
use mail_secrets::AccountSecrets;
use mail_store::{Registered, SqliteStore, Subscribed};
use porter_core::{Credential, SecretKey, SecretPurpose, SecretText};
use porter_ids::UnixSeconds;
use porter_secrets::MemorySecrets;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::Role;

// ---------------------------------------------------------------------------------------------
// One HTTP request off a socket.

#[derive(Debug, Clone, Default)]
struct Request {
    method: String,
    path: String,
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

    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or(Value::Null)
    }
}

/// Read one request's head and body; `None` once the client has gone.
async fn read_request(reader: &mut BufReader<TcpStream>) -> Option<Request> {
    let mut line = String::new();
    if reader.read_line(&mut line).await.ok()? == 0 {
        return None;
    }
    let mut parts = line.split_whitespace();
    let mut request = Request {
        method: parts.next()?.to_owned(),
        path: parts.next()?.to_owned(),
        ..Request::default()
    };
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).await.ok()?;
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
    reader.read_exact(&mut body).await.ok()?;
    request.body = String::from_utf8_lossy(&body).into_owned();
    Some(request)
}

async fn answer(sock: &mut BufReader<TcpStream>, status: &str, body: &str) {
    let text = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    sock.get_mut().write_all(text.as_bytes()).await.unwrap();
}

// ---------------------------------------------------------------------------------------------
// Graph's subscriptions.

#[derive(Default)]
struct Graph {
    asked: Vec<Request>,
    /// Subscriptions Graph holds.
    held: BTreeSet<String>,
    made: u32,
}

type Shared<T> = Arc<Mutex<T>>;

async fn graph() -> (String, Shared<Graph>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!(
        "http://127.0.0.1:{}/v1.0/subscriptions",
        listener.local_addr().unwrap().port()
    );
    let state: Shared<Graph> = Arc::default();
    let shared = state.clone();
    tokio::spawn(async move {
        while let Ok((sock, _)) = listener.accept().await {
            let state = shared.clone();
            tokio::spawn(async move {
                let mut sock = BufReader::new(sock);
                while let Some(request) = read_request(&mut sock).await {
                    let (status, body) = graph_answer(&state, request);
                    answer(&mut sock, status, &body).await;
                }
            });
        }
    });
    (base, state)
}

fn graph_answer(state: &Shared<Graph>, request: Request) -> (&'static str, String) {
    let mut graph = state.lock().unwrap();
    graph.asked.push(request.clone());
    if request.header("authorization") != Some("Bearer graph-token") {
        return (
            "401 Unauthorized",
            r#"{"error":{"code":"InvalidAuthenticationToken"}}"#.to_owned(),
        );
    }
    let id = request
        .path
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_owned();
    match request.method.as_str() {
        "POST" => {
            graph.made += 1;
            let id = format!("sub-{}", graph.made);
            graph.held.insert(id.clone());
            let asked = request.json();
            (
                "201 Created",
                json!({ "id": id, "expirationDateTime": asked["expirationDateTime"] }).to_string(),
            )
        }
        "PATCH" if graph.held.contains(&id) => (
            "200 OK",
            json!({ "id": id, "expirationDateTime": request.json()["expirationDateTime"] })
                .to_string(),
        ),
        "DELETE" => {
            graph.held.remove(&id);
            ("204 No Content", String::new())
        }
        _ => (
            "404 Not Found",
            r#"{"error":{"code":"ResourceNotFound"}}"#.to_owned(),
        ),
    }
}

// ---------------------------------------------------------------------------------------------
// The relay.

#[derive(Default)]
struct Relay {
    /// key → (clientStateHash, listenTokenHash), as registered.
    keys: BTreeMap<String, (String, String)>,
    /// What each open socket is sent through.
    sockets: Vec<mpsc::UnboundedSender<String>>,
    listens: u32,
}

async fn relay() -> (String, Shared<Relay>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let state: Shared<Relay> = Arc::default();
    let shared = state.clone();
    tokio::spawn(async move {
        while let Ok((sock, _)) = listener.accept().await {
            tokio::spawn(relay_connection(sock, shared.clone()));
        }
    });
    (base, state)
}

async fn relay_connection(sock: TcpStream, state: Shared<Relay>) {
    let mut sock = BufReader::new(sock);
    let Some(request) = read_request(&mut sock).await else {
        return;
    };
    if request.method == "POST" && request.path == "/graph/register" {
        let asked = request.json();
        let key = asked["key"].as_str().unwrap_or_default().to_owned();
        let hashes = (
            asked["clientStateHash"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            asked["listenTokenHash"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
        );
        state.lock().unwrap().keys.insert(key, hashes);
        answer(&mut sock, "201 Created", "{}").await;
        return;
    }
    let Some(key) = request.path.strip_prefix("/graph/listen/") else {
        answer(&mut sock, "404 Not Found", "{}").await;
        return;
    };
    let bearer = request
        .header("authorization")
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or_default();
    let known = state
        .lock()
        .unwrap()
        .keys
        .get(key)
        .is_some_and(|(_, token)| *token == digest(bearer));
    if !known {
        answer(&mut sock, "401 Unauthorized", "{}").await;
        return;
    }
    let accept = tokio_tungstenite::tungstenite::handshake::derive_accept_key(
        request
            .header("sec-websocket-key")
            .unwrap_or_default()
            .as_bytes(),
    );
    let upgrade = format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
         Sec-WebSocket-Accept: {accept}\r\n\r\n"
    );
    sock.get_mut().write_all(upgrade.as_bytes()).await.unwrap();
    let mut ws = WebSocketStream::from_raw_socket(sock.into_inner(), Role::Server, None).await;
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    {
        let mut relay = state.lock().unwrap();
        relay.sockets.push(tx);
        relay.listens += 1;
    }
    use futures_util::{SinkExt, StreamExt};
    loop {
        tokio::select! {
            said = rx.recv() => match said {
                Some(text) => {
                    if ws.send(Message::text(text)).await.is_err() {
                        return;
                    }
                }
                None => return,
            },
            heard = ws.next() => match heard {
                Some(Ok(Message::Text(text))) if text.as_str() == "ping" => {
                    if ws.send(Message::text("pong")).await.is_err() {
                        return;
                    }
                }
                Some(Ok(_)) => {}
                _ => return,
            },
        }
    }
}

/// Send `frame` down every socket the relay holds.
fn forward(state: &Shared<Relay>, frame: Value) {
    for socket in &state.lock().unwrap().sockets {
        let _ = socket.send(frame.to_string());
    }
}

// ---------------------------------------------------------------------------------------------
// The account.

fn plan() -> AccountPlan {
    presets::receive_through_graph(presets::microsoft_preset("me@contoso.example", now())).plan
}

fn caps() -> AccountCaps {
    AccountCaps {
        watch: WatchMode::Poll {
            every: Duration::from_secs(60),
        },
        ..crate::support::caps()
    }
}

/// A store holding one Graph account, pushed through `relay` when there is one.
fn store(relay: Option<&str>) -> Arc<SqliteStore> {
    let store = Arc::new(mail_store::testing::in_memory());
    mail_store::testing::seed_account_plan(&store, account(), "me@contoso.example", &plan(), None);
    mail_store::testing::seed_caps(&store, account(), &caps(), now()).unwrap();
    if let Some(relay) = relay {
        graph_push::set_relay(&store, account(), relay).unwrap();
    }
    store
}

/// A keyring holding a Graph token good for two hours of the wall clock: the watch reads the
/// wall clock, and a token it finds fresh is used without asking Microsoft.
fn secrets() -> Arc<MemorySecrets> {
    let secrets = MemorySecrets::default();
    let token = Credential::OAuth {
        access: SecretText::new("graph-token".to_owned()),
        refresh: SecretText::new("refresh".to_owned()),
        expires_at: UnixSeconds((chrono::Utc::now() + chrono::TimeDelta::hours(2)).timestamp()),
        granted: Default::default(),
    };
    mail_secrets::block_on(secrets.put(
        &SecretKey {
            account: account(),
            purpose: SecretPurpose::OutgoingPassword,
        },
        &token,
    ))
    .unwrap();
    Arc::new(secrets)
}

/// A watch on a thread of its own, and what it has heard.
struct Watching {
    heard: Arc<Mutex<Vec<Heard>>>,
    stop: watch::Sender<bool>,
    done: std::thread::JoinHandle<Result<(), live::Lost>>,
}

fn watching(store: Arc<SqliteStore>, secrets: Arc<MemorySecrets>, graph: &str) -> Watching {
    let heard = Arc::new(Mutex::new(Vec::new()));
    let said = heard.clone();
    let (stop, cancel) = watch::channel(false);
    let graph = graph.to_owned();
    let done = std::thread::spawn(move || {
        crate::blocking::block_on(live::listen_at(
            store,
            secrets,
            &mail_secrets::testing::fake_clients(),
            &graph,
            account(),
            Watch {
                cancel,
                hold: None,
                grace: Duration::from_millis(100),
            },
            &|h| said.lock().unwrap().push(h),
        ))
    });
    Watching { heard, stop, done }
}

impl Watching {
    /// Wait up to five seconds for `n` of `heard`, then say how many there were.
    async fn heard(&self, wanted: Heard, n: usize) -> usize {
        let count = || {
            self.heard
                .lock()
                .unwrap()
                .iter()
                .filter(|h| **h == wanted)
                .count()
        };
        for _ in 0..100 {
            if count() >= n {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        count()
    }

    fn stop(self) -> Result<(), live::Lost> {
        self.stop.send(true).unwrap();
        self.done.join().unwrap()
    }
}

async fn until(what: impl Fn() -> bool) -> bool {
    for _ in 0..100 {
        if what() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    what()
}

// ---------------------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_alert_from_the_relay_is_heard_as_mail_and_starts_a_pass() {
    let (graph_at, graph) = graph().await;
    let (relay_at, relay) = relay().await;
    let store = store(Some(&relay_at));
    let secrets = secrets();
    let started = chrono::Utc::now();
    let watch = watching(store.clone(), secrets.clone(), &graph_at);

    assert!(
        until(|| relay.lock().unwrap().listens == 1).await,
        "the socket is opened"
    );
    // Registered: the key, and hashes of the secret and the clientState, never either one.
    let held = store.graph_push(account()).unwrap();
    let Registered {
        relay: registered_at,
        key,
    } = held.registered.clone().unwrap();
    assert_eq!(registered_at, relay_at);
    let Credential::ApiKey(secret) =
        mail_secrets::block_on(secrets.get(&secret_key(account()))).unwrap()
    else {
        panic!("the listening secret is an account secret")
    };
    let (state_hash, token_hash) = relay.lock().unwrap().keys[&key].clone();
    assert_eq!(token_hash, digest(secret.expose()));
    assert_eq!(state_hash, digest(&client_state(secret.expose())));

    // Subscribed: the inbox's new messages, to the relay, with the clientState it checks.
    let made = graph.lock().unwrap().asked[0].clone();
    assert_eq!(
        (made.method.as_str(), made.path.as_str()),
        ("POST", "/v1.0/subscriptions")
    );
    let asked = made.json();
    let notify = format!("{relay_at}/graph/notify/{key}");
    assert_eq!(asked["resource"], "me/mailFolders('inbox')/messages");
    assert_eq!(asked["changeType"], "created");
    assert_eq!(asked["notificationUrl"], notify.as_str());
    assert_eq!(asked["lifecycleNotificationUrl"], notify.as_str());
    assert_eq!(asked["clientState"], client_state(secret.expose()).as_str());
    let expires =
        chrono::DateTime::parse_from_rfc3339(asked["expirationDateTime"].as_str().unwrap())
            .unwrap()
            .with_timezone(&chrono::Utc);
    let lifetime = expires - started;
    assert!(
        lifetime > chrono::TimeDelta::minutes(4200) && lifetime <= chrono::TimeDelta::minutes(4231),
        "within Graph's limit for messages: {lifetime}"
    );
    let subscribed = held.subscription.unwrap();
    assert_eq!(subscribed.id, "sub-1");

    // Set up anew, so what arrived meanwhile is looked for at once; then held.
    assert_eq!(watch.heard(Heard::Mail, 1).await, 1);
    assert_eq!(watch.heard(Heard::Established, 1).await, 1);

    forward(
        &relay,
        json!({ "type": "change", "subscriptionId": "sub-1", "changeType": "created",
                "resource": "Users('u')/Messages('m')" }),
    );
    assert_eq!(
        watch.heard(Heard::Mail, 2).await,
        2,
        "the alert is heard as mail"
    );
    // Which the scheduler answers with a pass.
    assert_eq!(
        mail_core::schedule::event_for(Heard::Mail),
        Event::Start(Trigger::Push)
    );

    // A relay that kept a missed flag says so on connecting, and that is mail too.
    forward(&relay, json!({ "type": "missed" }));
    assert_eq!(watch.heard(Heard::Mail, 3).await, 3);

    assert_eq!(watch.stop(), Ok(()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_subscription_near_its_end_is_renewed_and_one_graph_removed_is_made_again() {
    let (graph_at, graph) = graph().await;
    let (relay_at, relay) = relay().await;
    let store = store(Some(&relay_at));
    let secrets = secrets();
    // Set up by an earlier run: registered, and subscribed until an hour from now.
    let secret = "kept-secret";
    relay.lock().unwrap().keys.insert(
        "kept-key".to_owned(),
        (digest(&client_state(secret)), digest(secret)),
    );
    graph.lock().unwrap().held.insert("sub-old".to_owned());
    mail_secrets::block_on(secrets.put(
        &secret_key(account()),
        &Credential::ApiKey(SecretText::new(secret.to_owned())),
    ))
    .unwrap();
    store
        .push_registered(
            account(),
            &Registered {
                relay: relay_at.clone(),
                key: "kept-key".to_owned(),
            },
        )
        .unwrap();
    let soon = chrono::Utc::now() + chrono::TimeDelta::hours(1);
    store
        .push_subscribed(
            account(),
            Some(&Subscribed {
                id: "sub-old".to_owned(),
                expires: soon,
            }),
        )
        .unwrap();

    let watch = watching(store.clone(), secrets.clone(), &graph_at);
    assert!(until(|| relay.lock().unwrap().listens == 1).await);
    // Renewed with a PATCH before it ended, and kept: nothing was made again or registered.
    let asked = graph.lock().unwrap().asked.clone();
    assert_eq!(asked.len(), 1, "{asked:?}");
    assert_eq!(
        (asked[0].method.as_str(), asked[0].path.as_str()),
        ("PATCH", "/v1.0/subscriptions/sub-old")
    );
    let renewed = store.graph_push(account()).unwrap().subscription.unwrap();
    assert_eq!(renewed.id, "sub-old");
    assert!(
        renewed.expires - soon > chrono::TimeDelta::days(2),
        "{renewed:?}"
    );
    assert_eq!(relay.lock().unwrap().keys.len(), 1);
    assert_eq!(watch.heard(Heard::Established, 1).await, 1);
    assert_eq!(watch.heard(Heard::Mail, 0).await, 0, "nothing was missed");

    // Graph lets it go and says so through the relay: made again, and looked for at once.
    graph.lock().unwrap().held.remove("sub-old");
    forward(
        &relay,
        json!({ "type": "lifecycle", "subscriptionId": "sub-old",
                "lifecycleEvent": "subscriptionRemoved" }),
    );
    assert_eq!(watch.heard(Heard::Mail, 1).await, 1);
    let remade = store.graph_push(account()).unwrap().subscription.unwrap();
    assert_eq!(remade.id, "sub-1");
    let last = graph.lock().unwrap().asked.last().cloned().unwrap();
    assert_eq!(last.method, "POST");
    assert_eq!(last.json()["clientState"], client_state(secret).as_str());

    // Asked to reauthorise: renewed, and nothing missed.
    forward(
        &relay,
        json!({ "type": "lifecycle", "subscriptionId": "sub-1",
                "lifecycleEvent": "reauthorizationRequired" }),
    );
    assert!(
        until(|| {
            let graph = graph.lock().unwrap();
            graph
                .asked
                .last()
                .is_some_and(|r| r.method == "PATCH" && r.path.ends_with("/sub-1"))
        })
        .await
    );
    assert_eq!(watch.stop(), Ok(()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_lifecycle_frame_about_another_subscription_is_not_acted_on() {
    let (graph_at, graph) = graph().await;
    let (relay_at, relay) = relay().await;
    let store = store(Some(&relay_at));
    let watch = watching(store.clone(), secrets(), &graph_at);
    assert!(until(|| relay.lock().unwrap().listens == 1).await);
    assert_eq!(watch.heard(Heard::Mail, 1).await, 1, "set up anew");
    assert_eq!(
        store
            .graph_push(account())
            .unwrap()
            .subscription
            .unwrap()
            .id,
        "sub-1"
    );

    // About one let go before: Graph is not asked to make or renew anything.
    for lifecycle_event in ["subscriptionRemoved", "reauthorizationRequired"] {
        forward(
            &relay,
            json!({ "type": "lifecycle", "subscriptionId": "sub-earlier",
                    "lifecycleEvent": lifecycle_event }),
        );
    }
    // A change after them, heard once they have been read.
    forward(
        &relay,
        json!({ "type": "change", "subscriptionId": "sub-1", "changeType": "created" }),
    );
    assert_eq!(watch.heard(Heard::Mail, 2).await, 2);
    let methods: Vec<String> = graph
        .lock()
        .unwrap()
        .asked
        .iter()
        .map(|r| r.method.clone())
        .collect();
    assert_eq!(methods, ["POST"], "only the first subscription was made");
    assert_eq!(
        store
            .graph_push(account())
            .unwrap()
            .subscription
            .unwrap()
            .id,
        "sub-1"
    );
    assert_eq!(watch.stop(), Ok(()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_relay_changed_while_listening_is_listened_to_afresh() {
    let (graph_at, graph) = graph().await;
    let (first_at, first) = relay().await;
    let (second_at, second) = relay().await;
    let store = store(Some(&first_at));
    let secrets = secrets();
    let watch = watching(store.clone(), secrets.clone(), &graph_at);
    assert!(until(|| first.lock().unwrap().listens == 1).await);
    assert_eq!(watch.heard(Heard::Mail, 1).await, 1);

    // Changed in Settings while the socket to the first is held: the next wait lets this
    // listener go, to be opened again at once for the second.
    graph_push::set_relay(&store, account(), &second_at).unwrap();
    forward(
        &first,
        json!({ "type": "change", "subscriptionId": "sub-1", "changeType": "created" }),
    );
    let Watching {
        done, stop: _stop, ..
    } = watch;
    let lost = done.join().unwrap().unwrap_err();
    assert_eq!(lost.retry, Retry::Now, "{lost:?}");

    // Opened again, as the scheduler does: registered and subscribed at the second, and the
    // subscription to the first let go.
    let again = watching(store.clone(), secrets, &graph_at);
    assert!(until(|| second.lock().unwrap().listens == 1).await);
    let held = store.graph_push(account()).unwrap();
    let registered = held.registered.unwrap();
    assert_eq!(registered.relay, second_at);
    assert!(second.lock().unwrap().keys.contains_key(&registered.key));
    assert_eq!(held.subscription.unwrap().id, "sub-2");
    assert!(
        !graph.lock().unwrap().held.contains("sub-1"),
        "the subscription to the first relay is let go"
    );
    assert_eq!(again.stop(), Ok(()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_renewal_graph_no_longer_knows_makes_the_subscription_again() {
    let (graph_at, graph) = graph().await;
    let (relay_at, relay) = relay().await;
    let store = store(Some(&relay_at));
    let secrets = secrets();
    let secret = "kept-secret";
    relay.lock().unwrap().keys.insert(
        "kept-key".to_owned(),
        (digest(&client_state(secret)), digest(secret)),
    );
    mail_secrets::block_on(secrets.put(
        &secret_key(account()),
        &Credential::ApiKey(SecretText::new(secret.to_owned())),
    ))
    .unwrap();
    store
        .push_registered(
            account(),
            &Registered {
                relay: relay_at.clone(),
                key: "kept-key".to_owned(),
            },
        )
        .unwrap();
    // Graph has never heard of it.
    store
        .push_subscribed(
            account(),
            Some(&Subscribed {
                id: "sub-gone".to_owned(),
                expires: chrono::Utc::now() + chrono::TimeDelta::hours(1),
            }),
        )
        .unwrap();
    let watch = watching(store.clone(), secrets, &graph_at);
    assert!(until(|| relay.lock().unwrap().listens == 1).await);
    let methods: Vec<String> = graph
        .lock()
        .unwrap()
        .asked
        .iter()
        .map(|r| r.method.clone())
        .collect();
    assert_eq!(methods, ["PATCH", "POST"]);
    assert_eq!(
        store
            .graph_push(account())
            .unwrap()
            .subscription
            .unwrap()
            .id,
        "sub-1"
    );
    assert_eq!(
        watch.heard(Heard::Mail, 1).await,
        1,
        "made again: looked for at once"
    );
    assert_eq!(watch.stop(), Ok(()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_relay_that_forgot_the_key_is_registered_with_again() {
    let (graph_at, graph) = graph().await;
    let (relay_at, relay) = relay().await;
    let store = store(Some(&relay_at));
    let secrets = secrets();
    // Registered and subscribed once, and the relay has since lost its storage.
    mail_secrets::block_on(secrets.put(
        &secret_key(account()),
        &Credential::ApiKey(SecretText::new("old-secret".to_owned())),
    ))
    .unwrap();
    graph.lock().unwrap().held.insert("sub-old".to_owned());
    store
        .push_registered(
            account(),
            &Registered {
                relay: relay_at.clone(),
                key: "old-key".to_owned(),
            },
        )
        .unwrap();
    store
        .push_subscribed(
            account(),
            Some(&Subscribed {
                id: "sub-old".to_owned(),
                expires: chrono::Utc::now() + chrono::TimeDelta::days(2),
            }),
        )
        .unwrap();
    let watch = watching(store.clone(), secrets, &graph_at);
    assert_eq!(watch.heard(Heard::Mail, 1).await, 1);
    let held = store.graph_push(account()).unwrap();
    let key = held.registered.unwrap().key;
    assert_ne!(key, "old-key");
    assert!(relay.lock().unwrap().keys.contains_key(&key));
    assert_eq!(held.subscription.unwrap().id, "sub-1");
    assert!(
        !graph.lock().unwrap().held.contains("sub-old"),
        "the subscription to the forgotten key is let go"
    );
    assert!(until(|| relay.lock().unwrap().listens == 1).await);
    assert_eq!(watch.stop(), Ok(()));
}

#[test]
fn a_graph_account_without_a_relay_is_polled() {
    let store = store(None);
    assert!(live::pushing(&store).is_empty());
    assert_eq!(graph_push::relay(&store, account()), None);
    // Polled at its interval, as before.
    let intervals = mail_core::sync::due::intervals(&store);
    assert_eq!(intervals, [(account(), Duration::from_secs(60))]);
    // A watch asked of it anyway says there is nothing to wait on, and for good.
    let (_stop, cancel) = watch::channel(false);
    let lost = crate::blocking::block_on(live::listen_at(
        store.clone(),
        secrets(),
        &mail_secrets::testing::fake_clients(),
        "http://127.0.0.1:9/v1.0/subscriptions",
        account(),
        Watch {
            cancel,
            hold: None,
            grace: Duration::from_millis(100),
        },
        &|_| {},
    ))
    .unwrap_err();
    assert!(matches!(lost.retry, Retry::Fatal(_)), "{lost:?}");

    // A relay set makes it pushed to; cleared, polled again.
    graph_push::set_relay(&store, account(), "https://relay.example.test/").unwrap();
    assert_eq!(live::pushing(&store), [account()]);
    assert_eq!(
        graph_push::relay(&store, account()).as_deref(),
        Some("https://relay.example.test")
    );
    graph_push::set_relay(&store, account(), "  ").unwrap();
    assert!(live::pushing(&store).is_empty());
    // An address that cannot be a relay's is refused, and the setting stays as it was.
    assert!(graph_push::set_relay(&store, account(), "http://relay.example.test").is_err());
    assert!(live::pushing(&store).is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unreachable_relay_is_a_loss_the_scheduler_waits_out_while_polling() {
    let (graph_at, _graph) = graph().await;
    // Bound and closed: nothing listens there.
    let port = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    };
    let store = store(Some(&format!("http://127.0.0.1:{port}")));
    let (_stop, cancel) = watch::channel(false);
    let secrets = secrets();
    let lost = tokio::task::spawn_blocking(move || {
        crate::blocking::block_on(live::listen_at(
            store,
            secrets,
            &mail_secrets::testing::fake_clients(),
            &graph_at,
            account(),
            Watch {
                cancel,
                hold: None,
                grace: Duration::from_millis(100),
            },
            &|_| {},
        ))
    })
    .await
    .unwrap()
    .unwrap_err();
    assert!(matches!(lost.retry, Retry::After(_)), "{lost:?}");
}
