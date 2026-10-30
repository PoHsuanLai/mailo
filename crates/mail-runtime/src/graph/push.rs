//! Push for an account that reads through Microsoft Graph, by way of a relay.
//!
//! Graph sends change notifications only to a public HTTPS webhook, which a desktop client does
//! not have. A relay is that webhook (`relay/` in the repository, a Cloudflare Worker anyone can
//! deploy on an account of their own): Graph posts to it, and it forwards each notification over
//! a WebSocket this side holds open. The relay is told no more than it needs to route a ping:
//!
//! 1. **Register.** A random key names this account's notifications at the relay, and a random
//!    listening secret is what this side presents to hear them. `POST /graph/register` carries
//!    the key and the SHA-256 of the secret and of the subscription's `clientState`, never either
//!    one; the secret is kept in the keyring as an account secret ([`SecretPurpose::ApiKey`]), and
//!    the `clientState` is derived from it ([`client_state`]), so there is one secret to keep.
//! 2. **Subscribe.** `POST /subscriptions` asks Graph for `created` notifications on the inbox's
//!    messages, sent to `<relay>/graph/notify/<key>` with the `clientState`, and its lifecycle
//!    notifications to the same address. Graph's token goes to Graph and nowhere else.
//! 3. **Listen.** `GET <relay>/graph/listen/<key>` upgrades to a WebSocket with the secret as a
//!    bearer. Each frame is an alert with ids in it and no mail: [`Alert`].
//! 4. **Renew.** A subscription lasts [`LIFETIME`] and is renewed with a `PATCH` once it has less
//!    than [`RENEW_WITHIN`] left; one Graph or the relay says is gone is made again.
//!
//! A wait ([`Listener::wait`]) is the IMAP one's shape: an alert is [`Woke::Mail`], a send coming
//! due is [`Woke::Due`], the account's own poll interval is [`Woke::Interval`] (so nothing a poll
//! would have found is found later than without a relay; the relay only brings new mail sooner),
//! and a socket held for [`IDLE_REARM`] is let go and opened afresh at the next wait. A socket
//! that closes or goes silent is an error, and the scheduler reconnects with its backoff while the
//! account polls as before.

use crate::error::Logged;
use crate::graph::{detail, refusal_text, retry_after, segment};
use crate::pass::due_alarm;
use crate::{Cancel, IDLE_REARM, RuntimeError, Woke};
use base64::Engine as _;
use chrono::{DateTime, TimeDelta, Utc};
use futures_util::{SinkExt, StreamExt};
use mail_domain::{AccountPlan, Retry};
use mail_net::{Failure, within};
use mail_secrets::AccountSecrets;
use mail_secrets::tokens::{self, GraphReach};
use mail_store::{GraphPush, Registered, SqliteStore, Subscribed};
use porter_core::{AccountId, Credential, SecretKey, SecretPurpose, SecretText};
use porter_provider::ClientEntry;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::time::Instant;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::{self, Message, client::IntoClientRequest, http};
use zeroize::Zeroizing;

/// Graph's subscriptions, for the signed-in user.
pub const SUBSCRIPTIONS: &str = "https://graph.microsoft.com/v1.0/subscriptions";

/// What is subscribed to: messages arriving in the inbox. Other folders are found by the poll.
pub const RESOURCE: &str = "me/mailFolders('inbox')/messages";

/// How long a subscription is asked for.
///
/// Graph allows a message subscription 10 080 minutes (under seven days) today; 4 230 (under
/// three days) was the limit before, and is what is asked for, so a tenant or a cloud that still
/// holds to the older one does not refuse it. Graph's documentation warns that asking for more
/// than the limit will fail, not be shortened.
pub const LIFETIME: TimeDelta = TimeDelta::minutes(4230);

/// A subscription with less than this left is renewed. A day leaves a computer that was asleep
/// for most of one room to wake and renew before Graph lets the subscription go.
pub const RENEW_WITHIN: TimeDelta = TimeDelta::hours(24);

/// How often the socket is pinged. The relay answers without waking (Cloudflare's hibernation
/// auto-response); a NAT that forgot the connection is found by the silence after.
const PING: Duration = Duration::from_secs(30);

/// How long a ping may go unanswered, with nothing else heard either, before the socket is called
/// lost.
const SILENCE: Duration = Duration::from_secs(90);

/// How a socket is kept: pinged every `ping`, and lost once a ping has gone unanswered for
/// `silence`. [`PING`] and [`SILENCE`], except under test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Pacing {
    ping: Duration,
    silence: Duration,
}

impl Default for Pacing {
    fn default() -> Self {
        Pacing {
            ping: PING,
            silence: SILENCE,
        }
    }
}

/// The largest message or frame taken from the relay. Its alerts are a few hundred bytes.
const MAX_FRAME: usize = 64 * 1024;

/// The longest a request to the relay, or the socket's opening, may take.
const OPENING: Duration = Duration::from_secs(30);

/// The ping the relay answers with [`PONG`] without waking.
const PING_TEXT: &str = "ping";
const PONG: &str = "pong";

/// Bytes of randomness in a key and in a listening secret.
const RANDOM_BYTES: usize = 32;

/// When a subscription made at `now` is asked to end.
pub fn expiry(now: DateTime<Utc>) -> DateTime<Utc> {
    now + LIFETIME
}

/// Whether a subscription ending at `expires` is renewed at `now`: it has less than
/// [`RENEW_WITHIN`] left, or has ended.
pub fn renewal_due(expires: DateTime<Utc>, now: DateTime<Utc>) -> bool {
    expires - now <= RENEW_WITHIN
}

/// Whether a subscription ending at `expires` has ended at `now`, so is made again rather than
/// renewed.
pub fn lapsed(expires: DateTime<Utc>, now: DateTime<Utc>) -> bool {
    expires <= now
}

/// A relay's base URL, checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relay(url::Url);

/// Why a relay's address was not taken.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RelayRefused {
    #[error("that is not a web address")]
    NotAnAddress,
    /// Graph posts only to HTTPS, and the listening secret must not cross the network in the
    /// clear. Plain HTTP is taken on this computer's own loopback, for a relay run locally.
    #[error("the relay's address must start with https://")]
    NotHttps,
    #[error("the relay's address cannot carry a query, a fragment or a password")]
    Extras,
}

impl Relay {
    /// The relay at `text`: `https://` (or `http://` on loopback), a host, and optionally a path
    /// it is mounted under. A trailing `/` is dropped.
    pub fn parse(text: &str) -> Result<Relay, RelayRefused> {
        let url = url::Url::parse(text.trim()).map_err(|_| RelayRefused::NotAnAddress)?;
        let host = url.host_str().ok_or(RelayRefused::NotAnAddress)?;
        match url.scheme() {
            "https" => {}
            "http" if loopback(host) => {}
            _ => return Err(RelayRefused::NotHttps),
        }
        if url.query().is_some()
            || url.fragment().is_some()
            || url.password().is_some()
            || !url.username().is_empty()
        {
            return Err(RelayRefused::Extras);
        }
        let mut url = url;
        let path = url.path().trim_end_matches('/').to_owned();
        url.set_path(&path);
        Ok(Relay(url))
    }

    /// The base, as written: no trailing `/`.
    pub fn as_str(&self) -> &str {
        self.0.as_str().trim_end_matches('/')
    }

    /// `<relay>/graph/<rest>`.
    fn at(&self, rest: &str) -> String {
        format!("{}/graph/{rest}", self.as_str())
    }

    /// Where Graph posts the notifications for `key`, and its lifecycle notifications.
    pub fn notify(&self, key: &str) -> String {
        self.at(&format!("notify/{key}"))
    }

    /// Where this side registers a key.
    pub fn register(&self) -> String {
        self.at("register")
    }

    /// Where this side listens for `key`'s alerts: the same address as a WebSocket's.
    pub fn listen(&self, key: &str) -> String {
        let http = self.at(&format!("listen/{key}"));
        match http.strip_prefix("https://") {
            Some(rest) => format!("wss://{rest}"),
            None => format!("ws://{}", http.trim_start_matches("http://")),
        }
    }

    fn host(&self) -> &str {
        // `parse` refused an address without a host: there is always one.
        // An IPv6 host is written in brackets in an address, and not in a connection.
        self.0
            .host_str()
            // Never empty: `parse` refused an address without a host.
            .unwrap_or_default()
            .trim_start_matches('[')
            .trim_end_matches(']')
    }

    fn port(&self) -> u16 {
        self.0.port_or_known_default().unwrap_or(443)
    }

    fn secure(&self) -> bool {
        self.0.scheme() == "https"
    }
}

fn loopback(host: &str) -> bool {
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    bare.eq_ignore_ascii_case("localhost")
        || bare
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// `bytes` of randomness, as URL-safe text.
fn random() -> String {
    use rand::RngCore as _;
    let mut bytes = [0u8; RANDOM_BYTES];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// The SHA-256 of `text`, in lowercase hex: what the relay keeps of a secret.
pub fn digest(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// The `clientState` Graph is given, derived from the listening secret.
///
/// One secret kept rather than two, and a subscription made again later carries the same
/// `clientState` the relay was registered with. The relay holds only the SHA-256 of the secret,
/// and that does not give this (a different input is hashed), so the relay cannot listen with
/// what it learns from Graph's posts, nor post with what it keeps.
pub fn client_state(secret: &str) -> String {
    let derived = Sha256::digest(format!("mailo graph clientState\n{secret}").as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(derived)
}

/// Where the listening secret is filed: one per account, gone with the account.
pub fn secret_key(account: AccountId) -> SecretKey {
    SecretKey {
        account,
        purpose: SecretPurpose::ApiKey,
    }
}

/// What a frame from the relay says.
///
/// A lifecycle frame names the subscription it is about. One made before, by this listener or an
/// earlier one, can still be heard of after it was let go, and is not the current one's to act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Alert {
    /// A message was created in the inbox.
    Mail,
    /// The relay dropped notifications while nobody listened, or Graph says it did (`missed`).
    Missed,
    /// Graph removed this subscription (`subscriptionRemoved`): it is made again.
    Removed(String),
    /// Graph wants this subscription reauthorised (`reauthorizationRequired`): it is renewed.
    Reauthorize(String),
    /// The relay answered a ping.
    Pong,
}

/// What a text frame from the relay is, or `None` for one this side does not know.
///
/// `{"type":"change","subscriptionId":…,"changeType":"created","resource":…}`,
/// `{"type":"lifecycle","subscriptionId":…,"lifecycleEvent":"subscriptionRemoved"}`,
/// `{"type":"missed"}`, or `pong`.
pub fn alert_of(text: &str) -> Option<Alert> {
    if text.trim() == PONG {
        return Some(Alert::Pong);
    }
    // A frame that is not JSON is not an alert, and waking for it would be a pass for nothing.
    let value: Value = serde_json::from_str(text).ok()?;
    match value.get("type")?.as_str()? {
        "change" => Some(Alert::Mail),
        "missed" => Some(Alert::Missed),
        "lifecycle" => {
            let id = || {
                value
                    .get("subscriptionId")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            };
            match value.get("lifecycleEvent")?.as_str()? {
                "subscriptionRemoved" => Some(Alert::Removed(id()?)),
                "reauthorizationRequired" => Some(Alert::Reauthorize(id()?)),
                "missed" => Some(Alert::Missed),
                _ => None,
            }
        }
        _ => None,
    }
}

/// What a refusal from Graph's subscriptions means.
fn refused(status: u16, body: &str, after: Option<Duration>, doing: &str) -> RuntimeError {
    let why = format!(
        "Microsoft Graph refused to {doing} ({status}): {}",
        detail(body)
    );
    let retry = match status {
        401 => Retry::NeedsReauth,
        // Most often the tenant does not allow subscriptions, or the notification address was
        // not reachable when Graph checked it: neither changes by asking again soon.
        400 | 403 => Retry::After(Duration::from_secs(6 * 3600)),
        429 | 503 | 504 => Retry::After(after.unwrap_or(Duration::from_secs(60))),
        _ => Retry::After(Duration::from_secs(60)),
    };
    RuntimeError::Graph { why, retry }
}

/// The relay did not do what it was asked.
fn relay_refused(status: u16, body: &str) -> RuntimeError {
    let said: String = body.chars().take(200).collect();
    RuntimeError::Graph {
        why: format!("the push relay refused ({status}): {said}"),
        retry: Retry::After(Duration::from_secs(if status == 409 { 5 } else { 600 })),
    }
}

fn unreachable(what: &str, e: impl Into<mail_net::Source>) -> RuntimeError {
    RuntimeError::Connect(Failure::new(what.to_owned(), e))
}

/// Register `key` at `relay`, to be listened to with `secret`.
pub async fn register(
    http: &reqwest::Client,
    relay: &Relay,
    key: &str,
    secret: &str,
) -> Result<(), RuntimeError> {
    let body = json!({
        "key": key,
        "clientStateHash": digest(&client_state(secret)),
        "listenTokenHash": digest(secret),
    });
    let response = http
        .post(relay.register())
        .json(&body)
        .timeout(OPENING)
        .send()
        .await
        .map_err(|e| unreachable("the push relay", e))?;
    let status = response.status();
    if status.is_success() {
        return Ok(());
    }
    let text = refusal_text(response).await;
    Err(relay_refused(status.as_u16(), &text))
}

/// Ask Graph at `subscriptions` to post `key`'s notifications to `relay`, until [`expiry`].
pub async fn subscribe(
    http: &reqwest::Client,
    subscriptions: &str,
    access: &str,
    relay: &Relay,
    key: &str,
    secret: &str,
    now: DateTime<Utc>,
) -> Result<Subscribed, RuntimeError> {
    let notify = relay.notify(key);
    let body = json!({
        "changeType": "created",
        "notificationUrl": notify,
        "lifecycleNotificationUrl": notify,
        "resource": RESOURCE,
        "expirationDateTime": stamp(expiry(now)),
        "clientState": client_state(secret),
    });
    let answer = match graph(
        http,
        reqwest::Method::POST,
        subscriptions,
        access,
        &body,
        "subscribe",
    )
    .await?
    {
        Answered::Body(answer) => answer,
        Answered::Gone => Value::Null,
    };
    let id = answer
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| RuntimeError::Graph {
            why: "Microsoft Graph did not say which subscription it made".to_owned(),
            retry: Retry::After(Duration::from_secs(60)),
        })?;
    Ok(Subscribed {
        id: id.to_owned(),
        expires: ends(&answer).unwrap_or_else(|| expiry(now)),
    })
}

/// How a renewal went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Renewed {
    /// The subscription now ends then.
    Until(DateTime<Utc>),
    /// Graph no longer has it: it is made again.
    Gone,
}

/// Renew subscription `id` at `subscriptions`, to [`expiry`] from `now`. A `PATCH` also answers
/// Graph's `reauthorizationRequired`.
pub async fn renew(
    http: &reqwest::Client,
    subscriptions: &str,
    access: &str,
    id: &str,
    now: DateTime<Utc>,
) -> Result<Renewed, RuntimeError> {
    let wanted = expiry(now);
    let url = format!("{subscriptions}/{}", segment(id));
    let body = json!({ "expirationDateTime": stamp(wanted) });
    let answered = graph(
        http,
        reqwest::Method::PATCH,
        &url,
        access,
        &body,
        "renew the subscription",
    )
    .await?;
    Ok(match answered {
        Answered::Body(answer) => Renewed::Until(ends(&answer).unwrap_or(wanted)),
        Answered::Gone => Renewed::Gone,
    })
}

/// Delete subscription `id` at `subscriptions`. One already gone is deleted.
pub async fn unsubscribe(
    http: &reqwest::Client,
    subscriptions: &str,
    access: &str,
    id: &str,
) -> Result<(), RuntimeError> {
    let url = format!("{subscriptions}/{}", segment(id));
    let response = http
        .delete(url)
        .bearer_auth(access)
        .send()
        .await
        .map_err(|e| unreachable("Microsoft Graph", e))?;
    let status = response.status();
    if status.is_success() || status.as_u16() == 404 {
        return Ok(());
    }
    let after = retry_after(&response);
    let text = refusal_text(response).await;
    Err(refused(
        status.as_u16(),
        &text,
        after,
        "remove the subscription",
    ))
}

/// What Graph answered a subscription request with.
enum Answered {
    /// Its JSON, or `Null` for none.
    Body(Value),
    /// `404`: the subscription is not there.
    Gone,
}

/// One authenticated JSON request to Graph's subscriptions.
async fn graph(
    http: &reqwest::Client,
    method: reqwest::Method,
    url: &str,
    access: &str,
    body: &Value,
    doing: &str,
) -> Result<Answered, RuntimeError> {
    let response = http
        .request(method, url)
        .bearer_auth(access)
        .json(body)
        .send()
        .await
        .map_err(|e| unreachable("Microsoft Graph", e))?;
    let status = response.status();
    if status.as_u16() == 404 {
        return Ok(Answered::Gone);
    }
    if status.is_success() {
        let text = mail_net::http::text_up_to(response, mail_net::http::MAX_REFUSAL)
            .await
            .or_log_default("Microsoft Graph's answer about a subscription could not be read");
        return Ok(Answered::Body(
            serde_json::from_str(&text).unwrap_or(Value::Null),
        ));
    }
    let after = retry_after(&response);
    let text = refusal_text(response).await;
    Err(refused(status.as_u16(), &text, after, doing))
}

/// An instant as Graph writes one.
fn stamp(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// When Graph says a subscription ends.
fn ends(answer: &Value) -> Option<DateTime<Utc>> {
    let text = answer.get("expirationDateTime")?.as_str()?;
    DateTime::parse_from_rfc3339(text)
        // An instant written some other way is the one asked for: the caller falls back to it.
        .ok()
        .map(|at| at.with_timezone(&Utc))
}

/// A byte stream a socket is held over: plain on loopback, TLS everywhere else.
trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}

/// The WebSocket to the relay.
struct Socket {
    ws: WebSocketStream<Box<dyn Io>>,
    /// When it opened: it is let go at [`IDLE_REARM`].
    opened: Instant,
    /// When the next ping is due. Kept here rather than timed from each call, since a wait ends
    /// at every poll interval, which may be shorter than [`PING`].
    ping: Instant,
    /// When the ping still waiting for an answer was sent. Any frame answers it: what matters is
    /// that the relay is still there.
    unanswered: Option<Instant>,
    pacing: Pacing,
}

/// What opening the socket found.
enum Opened {
    Socket(Box<Socket>),
    /// The relay does not know the key, or not with this secret: it was redeployed, or its
    /// storage cleared. Registered again.
    Unknown,
}

fn lost(why: &str) -> RuntimeError {
    RuntimeError::Connect(Failure::said(format!("the push relay {why}")))
}

/// Open the socket to `relay` for `key`, presenting `secret`, to be kept at `pacing`.
async fn open_socket(
    relay: &Relay,
    key: &str,
    secret: &str,
    pacing: Pacing,
) -> Result<Opened, RuntimeError> {
    within(OPENING, "opening the push relay's connection", async {
        let tcp = tokio::net::TcpStream::connect((relay.host(), relay.port()))
            .await
            .map_err(|e| unreachable("the push relay", e))?;
        let io: Box<dyn Io> = if relay.secure() {
            Box::new(mail_net::transport::Transport::wrap(tcp, relay.host()).await?)
        } else {
            Box::new(tcp)
        };
        let mut request = relay
            .listen(key)
            .into_client_request()
            .map_err(|e| unreachable("the push relay's address", e))?;
        let bearer = http::HeaderValue::from_str(&format!("Bearer {secret}"))
            .map_err(|e| unreachable("the push relay's secret", e))?;
        request
            .headers_mut()
            .insert(http::header::AUTHORIZATION, bearer);
        let config = tungstenite::protocol::WebSocketConfig::default()
            .max_message_size(Some(MAX_FRAME))
            .max_frame_size(Some(MAX_FRAME));
        match tokio_tungstenite::client_async_with_config(request, io, Some(config)).await {
            Ok((ws, _)) => {
                let now = Instant::now();
                Ok(Opened::Socket(Box::new(Socket {
                    ws,
                    opened: now,
                    ping: now + pacing.ping,
                    unanswered: None,
                    pacing,
                })))
            }
            Err(tungstenite::Error::Http(response))
                if matches!(response.status().as_u16(), 401 | 403 | 404 | 410) =>
            {
                Ok(Opened::Unknown)
            }
            Err(e) => Err(unreachable("the push relay", e)),
        }
    })
    .await
}

impl Socket {
    /// The next alert, pinging the relay while it is quiet. An error when the socket closes, or a
    /// ping has gone unanswered, with nothing else heard either, for the pacing's `silence`.
    ///
    /// Nothing reads the socket while a pass runs between two waits, so the frames that came
    /// meanwhile are read first (`biased`) and only then the clock: a pass longer than the silence
    /// is not the relay going quiet.
    async fn next(&mut self) -> Result<Alert, RuntimeError> {
        loop {
            let deadline = match self.unanswered {
                Some(sent) => sent + self.pacing.silence,
                None => self.ping,
            };
            tokio::select! {
                biased;
                frame = self.ws.next() => {
                    let frame = match frame {
                        None => return Err(lost("closed the connection")),
                        Some(Err(e)) => return Err(unreachable("the push relay", e)),
                        Some(Ok(frame)) => frame,
                    };
                    self.unanswered = None;
                    match frame {
                        Message::Text(text) => match alert_of(text.as_str()) {
                            Some(Alert::Pong) | None => {}
                            Some(alert) => return Ok(alert),
                        },
                        Message::Close(_) => return Err(lost("closed the connection")),
                        Message::Ping(_) | Message::Pong(_) | Message::Binary(_) | Message::Frame(_) => {}
                    }
                }
                () = tokio::time::sleep_until(deadline) => {
                    if self.unanswered.is_some() {
                        return Err(RuntimeError::Stalled {
                            doing: "listening to the push relay",
                            after: self.pacing.silence,
                        });
                    }
                    let now = Instant::now();
                    self.ping = now + self.pacing.ping;
                    self.unanswered = Some(now);
                    self.ws
                        .send(Message::text(PING_TEXT))
                        .await
                        .map_err(|e| unreachable("the push relay", e))?;
                }
            }
        }
    }
}

/// What ended one turn of a wait.
enum Step {
    Alert(Result<Alert, RuntimeError>),
    Rearm,
    Interval,
    Due,
    Cancel,
}

/// One account's push: what is registered at the relay, the subscription at Graph, and the
/// socket. Built by [`Listener::open`] for each connection a watch holds, and waited on by
/// [`Listener::wait`] as an engine is.
pub struct Listener {
    account: AccountId,
    store: Arc<SqliteStore>,
    secrets: Arc<dyn AccountSecrets>,
    client: ClientEntry,
    reach: GraphReach,
    http: reqwest::Client,
    /// Graph's subscriptions: [`SUBSCRIPTIONS`], except under test.
    subscriptions: String,
    relay: Relay,
    key: String,
    secret: Zeroizing<String>,
    subscription: Option<Subscribed>,
    socket: Option<Box<Socket>>,
    /// How often the outbox is looked in while waiting.
    outbox: Duration,
    /// Set up anew since the last wait, so notifications may have been missed: the next wait
    /// says so at once.
    owed: bool,
    /// Registered by this listener: a relay that refuses the key again is refusing, not
    /// forgetting.
    fresh: bool,
    pacing: Pacing,
}

impl std::fmt::Debug for Listener {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Listener")
            .field("account", &self.account)
            .field("relay", &self.relay)
            .field("subscription", &self.subscription)
            .finish_non_exhaustive()
    }
}

/// What the account's relay is, as a failure when it has none to push through.
fn relay_of(held: &GraphPush) -> Result<Relay, RuntimeError> {
    let fatal = |why: String| RuntimeError::Graph {
        retry: Retry::Fatal(why.clone()),
        why,
    };
    let text = held
        .relay
        .as_deref()
        .ok_or_else(|| fatal("no push relay is set for this account".to_owned()))?;
    Relay::parse(text).map_err(|e| fatal(format!("the push relay {text}: {e}")))
}

impl Listener {
    /// Set `account`'s push up as far as it is not already: registered at its relay, subscribed
    /// at `subscriptions` (normally [`SUBSCRIPTIONS`]), and the subscription not near its end.
    /// The socket is opened by the first wait.
    pub async fn open(
        account: AccountId,
        plan: &AccountPlan,
        store: Arc<SqliteStore>,
        secrets: Arc<dyn AccountSecrets>,
        client: ClientEntry,
        subscriptions: &str,
    ) -> Result<Self, RuntimeError> {
        let held = store.graph_push(account.clone())?;
        let relay = relay_of(&held)?;
        let mut this = Listener {
            account,
            store,
            secrets,
            client,
            reach: GraphReach::of(plan),
            http: mail_net::http::http_client()?,
            subscriptions: subscriptions.trim_end_matches('/').to_owned(),
            relay,
            key: String::new(),
            secret: Zeroizing::new(String::new()),
            subscription: None,
            socket: None,
            outbox: crate::pass::OUTBOX_LOOK,
            owed: false,
            fresh: false,
            pacing: Pacing::default(),
        };
        let kept = this.kept_secret().await?;
        match (held.registered, kept) {
            (Some(registered), Some(secret)) if registered.relay == this.relay.as_str() => {
                this.key = registered.key;
                this.secret = secret;
                this.subscription = held.subscription;
            }
            (_, _) => {
                // Made for another relay, or its secret is gone: Graph would go on posting to an
                // address nobody listens at until it lapsed.
                if let Some(old) = held.subscription {
                    this.let_go(&old.id).await;
                }
                this.register_again().await?;
            }
        }
        this.keep_subscribed(Utc::now(), false).await?;
        Ok(this)
    }

    /// The listening secret kept for the account, or `None` when there is none.
    async fn kept_secret(&self) -> Result<Option<Zeroizing<String>>, RuntimeError> {
        match self.secrets.get(&secret_key(self.account.clone())).await {
            Ok(Credential::ApiKey(secret)) => Ok(Some(Zeroizing::new(secret.expose().to_owned()))),
            Ok(_) => Ok(None),
            Err(e) => match mail_secrets::SecretTrouble::of(&e) {
                mail_secrets::SecretTrouble::Absent => Ok(None),
                mail_secrets::SecretTrouble::Unreachable(_) => Err(e.into()),
            },
        }
    }

    /// A Graph access token for the account, renewed if it has to be.
    async fn access(&self) -> Result<Zeroizing<String>, RuntimeError> {
        let credential = tokens::graph_token(
            self.account.clone(),
            &self.client,
            self.reach,
            self.secrets.as_ref(),
            Utc::now(),
        )
        .await?;
        match credential {
            Credential::OAuth { access, .. } => Ok(Zeroizing::new(access.expose().to_owned())),
            _ => Err(RuntimeError::Graph {
                why: "the account has no Microsoft Graph sign-in to subscribe with".to_owned(),
                retry: Retry::NeedsReauth,
            }),
        }
    }

    /// Delete subscription `id`, as well as can be done: one that stays lapses on its own.
    async fn let_go(&self, id: &str) {
        let gone = match self.access().await {
            Ok(access) => unsubscribe(&self.http, &self.subscriptions, &access, id).await,
            Err(e) => Err(e),
        };
        gone.or_log("a Graph subscription no longer listened to could not be removed");
    }

    /// Register a new key and secret at the relay, and keep them. The subscription made for the
    /// old key is forgotten with it.
    async fn register_again(&mut self) -> Result<(), RuntimeError> {
        let key = random();
        let secret = Zeroizing::new(random());
        register(&self.http, &self.relay, &key, &secret).await?;
        // The secret first: a key kept without its secret is registered again next time, where
        // a secret without its key is only overwritten.
        self.secrets
            .put(
                &secret_key(self.account.clone()),
                &Credential::ApiKey(SecretText::new(secret.as_str().to_owned())),
            )
            .await?;
        let registered = Registered {
            relay: self.relay.as_str().to_owned(),
            key,
        };
        self.store
            .push_registered(self.account.clone(), &registered)?;
        self.key = registered.key;
        self.secret = secret;
        self.subscription = None;
        self.socket = None;
        self.owed = true;
        self.fresh = true;
        Ok(())
    }

    /// Make sure there is a subscription that is not near its end at `now`: renewed when it is
    /// (or `now` regardless, with `reauthorize`), made again when Graph no longer has it.
    async fn keep_subscribed(
        &mut self,
        now: DateTime<Utc>,
        reauthorize: bool,
    ) -> Result<(), RuntimeError> {
        if let Some(held) = &self.subscription {
            if !reauthorize && !renewal_due(held.expires, now) {
                return Ok(());
            }
            if !lapsed(held.expires, now) {
                let access = self.access().await?;
                match renew(&self.http, &self.subscriptions, &access, &held.id, now).await? {
                    Renewed::Until(expires) => {
                        let renewed = Subscribed {
                            id: held.id.clone(),
                            expires,
                        };
                        self.store
                            .push_subscribed(self.account.clone(), Some(&renewed))?;
                        self.subscription = Some(renewed);
                        return Ok(());
                    }
                    Renewed::Gone => {}
                }
            }
        }
        self.subscribe_again(now).await
    }

    /// Make a new subscription, the one before it (if any) being gone.
    async fn subscribe_again(&mut self, now: DateTime<Utc>) -> Result<(), RuntimeError> {
        self.subscription = None;
        self.store.push_subscribed(self.account.clone(), None)?;
        let access = self.access().await?;
        let made = subscribe(
            &self.http,
            &self.subscriptions,
            &access,
            &self.relay,
            &self.key,
            &self.secret,
            now,
        )
        .await?;
        self.store
            .push_subscribed(self.account.clone(), Some(&made))?;
        self.subscription = Some(made);
        self.owed = true;
        Ok(())
    }

    /// The subscription as it stands: Graph's id, and when it ends.
    pub fn subscription(&self) -> Option<&Subscribed> {
        self.subscription.as_ref()
    }

    /// Whether `id` is the subscription held now, so a lifecycle frame about it is acted on.
    fn current(&self, id: &str) -> bool {
        self.subscription.as_ref().is_some_and(|held| held.id == id)
    }

    /// Wait for an alert from the relay, a send coming due, or `poll`, whichever is first. See
    /// the module's documentation. Reads the wall clock, as a watch must.
    pub async fn wait(
        &mut self,
        cancel: &mut Cancel,
        poll: Duration,
    ) -> Result<Woke, RuntimeError> {
        // The relay set now. One changed while this listened is listened to afresh, by a listener
        // opened for it: this one's key, subscription and socket are all the old relay's.
        let set = relay_of(&self.store.graph_push(self.account.clone())?)?;
        if set != self.relay {
            self.socket = None;
            return Err(RuntimeError::Graph {
                why: "the push relay was changed".to_owned(),
                retry: Retry::Now,
            });
        }
        if std::mem::take(&mut self.owed) {
            return Ok(Woke::Mail);
        }
        let started = Utc::now();
        self.keep_subscribed(started, false).await?;
        let alarm = due_alarm(
            self.store.clone(),
            self.account.clone(),
            started,
            self.outbox,
        );
        let interval = tokio::time::sleep(poll);
        tokio::pin!(alarm, interval);
        loop {
            if self.socket.is_none() {
                match open_socket(&self.relay, &self.key, &self.secret, self.pacing).await? {
                    Opened::Socket(socket) => self.socket = Some(socket),
                    Opened::Unknown if self.fresh => {
                        return Err(RuntimeError::Graph {
                            why: "the push relay refused a key it had just registered".to_owned(),
                            retry: Retry::After(Duration::from_secs(600)),
                        });
                    }
                    Opened::Unknown => {
                        if let Some(old) = self.subscription.take() {
                            self.let_go(&old.id).await;
                        }
                        self.register_again().await?;
                        self.subscribe_again(Utc::now()).await?;
                        self.owed = false;
                        return Ok(Woke::Mail);
                    }
                }
            }
            let Some(socket) = self.socket.as_mut() else {
                continue;
            };
            let rearm = socket.opened + IDLE_REARM;
            let step = tokio::select! {
                alert = socket.next() => Step::Alert(alert),
                () = tokio::time::sleep_until(rearm) => Step::Rearm,
                () = &mut interval => Step::Interval,
                () = &mut alarm => Step::Due,
                changed = cancel.changed() => {
                    if changed.is_err() || *cancel.borrow() {
                        Step::Cancel
                    } else {
                        continue;
                    }
                }
            };
            match step {
                Step::Alert(Ok(Alert::Mail | Alert::Missed)) => return Ok(Woke::Mail),
                Step::Alert(Ok(Alert::Removed(id))) if self.current(&id) => {
                    self.subscribe_again(Utc::now()).await?;
                    self.owed = false;
                    return Ok(Woke::Mail);
                }
                Step::Alert(Ok(Alert::Reauthorize(id))) if self.current(&id) => {
                    self.keep_subscribed(Utc::now(), true).await?;
                    if std::mem::take(&mut self.owed) {
                        return Ok(Woke::Mail);
                    }
                }
                // About a subscription let go before, or the relay answering a ping.
                Step::Alert(Ok(Alert::Removed(_) | Alert::Reauthorize(_) | Alert::Pong)) => {}
                Step::Alert(Err(e)) => {
                    self.socket = None;
                    return Err(e);
                }
                Step::Rearm => {
                    self.socket = None;
                    return Ok(Woke::Interval);
                }
                Step::Interval => return Ok(Woke::Interval),
                Step::Due => return Ok(Woke::Due),
                Step::Cancel => return Err(RuntimeError::Cancelled),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(minutes: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_700_000_000, 0).unwrap() + TimeDelta::minutes(minutes)
    }

    #[test]
    fn a_subscription_is_asked_for_within_graph_s_limit_for_messages() {
        // 10 080 minutes today, 4 230 before: what is asked for is under both.
        assert!(LIFETIME <= TimeDelta::minutes(4230));
        assert_eq!(expiry(at(0)), at(4230));
        assert!(RENEW_WITHIN < LIFETIME);
    }

    #[test]
    fn a_subscription_is_renewed_a_day_before_it_ends_and_made_again_once_it_has() {
        let expires = at(4230);
        assert!(!renewal_due(expires, at(0)));
        assert!(!renewal_due(expires, at(4230 - 24 * 60 - 1)));
        assert!(renewal_due(expires, at(4230 - 24 * 60)));
        assert!(!lapsed(expires, at(4229)));
        assert!(lapsed(expires, at(4230)));
        assert!(renewal_due(expires, at(5000)));
    }

    #[test]
    fn a_relay_is_https_or_this_computer() {
        let relay = Relay::parse(" https://relay.example.test/mailo/ ").unwrap();
        assert_eq!(relay.as_str(), "https://relay.example.test/mailo");
        assert_eq!(
            relay.notify("k1"),
            "https://relay.example.test/mailo/graph/notify/k1"
        );
        assert_eq!(
            relay.listen("k1"),
            "wss://relay.example.test/mailo/graph/listen/k1"
        );
        assert_eq!(
            relay.register(),
            "https://relay.example.test/mailo/graph/register"
        );
        let local = Relay::parse("http://127.0.0.1:8787").unwrap();
        assert_eq!(local.listen("k"), "ws://127.0.0.1:8787/graph/listen/k");
        assert_eq!(
            Relay::parse("http://relay.example.test"),
            Err(RelayRefused::NotHttps)
        );
        assert_eq!(
            Relay::parse("https://relay.example.test/?a=b"),
            Err(RelayRefused::Extras)
        );
        assert_eq!(
            Relay::parse("relay.example.test"),
            Err(RelayRefused::NotAnAddress)
        );
        assert_eq!(
            Relay::parse("ftp://relay.example.test"),
            Err(RelayRefused::NotHttps)
        );
    }

    #[test]
    fn the_relay_keeps_hashes_that_give_neither_secret() {
        let secret = "s3cret";
        let state = client_state(secret);
        assert_eq!(
            state,
            client_state(secret),
            "the same secret, the same state"
        );
        assert_ne!(state, client_state("other"));
        assert!(state.len() <= 128, "Graph takes at most 128 characters");
        assert_ne!(digest(secret), digest(&state));
        assert_eq!(digest("abc").len(), 64);
        assert_ne!(random(), random());
    }

    #[test]
    fn a_frame_says_what_happened_and_nothing_unknown_wakes() {
        assert_eq!(
            alert_of(
                r#"{"type":"change","subscriptionId":"s","changeType":"created","resource":"r"}"#
            ),
            Some(Alert::Mail)
        );
        assert_eq!(alert_of(r#"{"type":"missed"}"#), Some(Alert::Missed));
        assert_eq!(
            alert_of(
                r#"{"type":"lifecycle","subscriptionId":"s1","lifecycleEvent":"subscriptionRemoved"}"#
            ),
            Some(Alert::Removed("s1".to_owned()))
        );
        assert_eq!(
            alert_of(
                r#"{"type":"lifecycle","subscriptionId":"s2","lifecycleEvent":"reauthorizationRequired"}"#
            ),
            Some(Alert::Reauthorize("s2".to_owned()))
        );
        // One that names no subscription is about none this side holds.
        assert_eq!(
            alert_of(r#"{"type":"lifecycle","lifecycleEvent":"subscriptionRemoved"}"#),
            None
        );
        assert_eq!(
            alert_of(r#"{"type":"lifecycle","lifecycleEvent":"missed"}"#),
            Some(Alert::Missed)
        );
        assert_eq!(alert_of("pong"), Some(Alert::Pong));
        assert_eq!(alert_of(r#"{"type":"other"}"#), None);
        assert_eq!(alert_of("not json"), None);
    }

    /// A relay on loopback that upgrades whatever asks, answers `ping` with `pong` while
    /// `answers` is true, and sends down the socket what it is given.
    async fn relay_on_loopback(
        answers: bool,
    ) -> (Relay, tokio::sync::mpsc::UnboundedSender<&'static str>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let relay = Relay::parse(&format!(
            "http://127.0.0.1:{}",
            listener.local_addr().unwrap().port()
        ))
        .unwrap();
        let (say, mut said) = tokio::sync::mpsc::unbounded_channel::<&'static str>();
        tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
            loop {
                tokio::select! {
                    frame = said.recv() => match frame {
                        Some(frame) => ws.send(Message::text(frame)).await.unwrap(),
                        None => return,
                    },
                    heard = ws.next() => match heard {
                        Some(Ok(Message::Text(text))) if answers && text.as_str() == PING_TEXT => {
                            ws.send(Message::text(PONG)).await.unwrap();
                        }
                        Some(Ok(_)) => {}
                        _ => return,
                    },
                }
            }
        });
        (relay, say)
    }

    const QUICK: Pacing = Pacing {
        ping: Duration::from_millis(40),
        silence: Duration::from_millis(150),
    };

    async fn socket_to(relay: &Relay) -> Box<Socket> {
        match open_socket(relay, "k", "s", QUICK).await.unwrap() {
            Opened::Socket(socket) => socket,
            Opened::Unknown => panic!("the loopback relay knows every key"),
        }
    }

    const CHANGE: &str = r#"{"type":"change","subscriptionId":"s","changeType":"created"}"#;

    #[tokio::test]
    async fn a_pass_longer_than_the_silence_is_not_the_relay_going_quiet() {
        let (relay, say) = relay_on_loopback(true).await;
        let mut socket = socket_to(&relay).await;
        say.send(CHANGE).unwrap();
        assert_eq!(socket.next().await.unwrap(), Alert::Mail);
        // A pass runs, and nobody reads the socket for longer than the silence. The relay is
        // there all the while: the next wait hears the next alert rather than calling it lost.
        tokio::time::sleep(QUICK.silence * 3).await;
        say.send(CHANGE).unwrap();
        assert_eq!(socket.next().await.unwrap(), Alert::Mail);
        // And it keeps pinging, answered, past the silence, until an alert.
        let later = say.clone();
        tokio::spawn(async move {
            tokio::time::sleep(QUICK.silence * 3).await;
            later.send(CHANGE).unwrap();
        });
        assert_eq!(socket.next().await.unwrap(), Alert::Mail);
    }

    #[tokio::test]
    async fn a_ping_unanswered_for_the_silence_is_a_lost_socket() {
        let (relay, _say) = relay_on_loopback(false).await;
        let mut socket = socket_to(&relay).await;
        let started = Instant::now();
        let error = socket.next().await.unwrap_err();
        assert!(
            matches!(error, RuntimeError::Stalled { after, .. } if after == QUICK.silence),
            "{error:?}"
        );
        assert!(started.elapsed() >= QUICK.ping + QUICK.silence);
    }

    #[test]
    fn graph_s_refusals_wait_rather_than_spin() {
        let RuntimeError::Graph { retry, .. } = refused(401, "", None, "subscribe") else {
            panic!()
        };
        assert_eq!(retry, Retry::NeedsReauth);
        let RuntimeError::Graph { retry, .. } = refused(403, "", None, "subscribe") else {
            panic!()
        };
        assert!(matches!(retry, Retry::After(d) if d >= Duration::from_secs(3600)));
        let RuntimeError::Graph { retry, .. } =
            refused(429, "", Some(Duration::from_secs(9)), "subscribe")
        else {
            panic!()
        };
        assert_eq!(retry, Retry::After(Duration::from_secs(9)));
    }
}
