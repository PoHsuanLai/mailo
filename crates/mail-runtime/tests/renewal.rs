//! Tokens renewed inside an engine that outlives them — `mailo watch`, `plan.md` 10.1.
//!
//! An access token lives about an hour and `mailo watch` runs for days. Every test here drives
//! one engine, built once, against servers in this process: an IMAP server that accepts only the
//! bearer tokens it is told to, a token endpoint that answers what it is scripted to, and a
//! stand-in for Graph's `sendMail`. The clock is a number the test moves.
//!
//! What matters is visible only from the servers' side: which token each connection presented,
//! and how many times the issuer was asked — because the failure this guards against in one
//! direction is a watch that stops an hour in, and in the other a watch that asks the issuer
//! for a new token on every operation of every pass.

use base64::Engine as _;
use chrono::{DateTime, TimeDelta, TimeZone, Utc};
use mail_domain::*;
use mail_mime::posting;
use mail_proto::backend::{Authenticate, ImapBackend, Pop3Backend};
use mail_proto::{ImapAuth, ImapCommand, ImapSession, Pop3Command, Pop3Session};
use mail_runtime::oauth::Endpoints;
use mail_runtime::renewal::Now;
use mail_runtime::{
    AccountEngine, Held, MapSecrets, Registration, Renewal, RuntimeError, Secrets, Token,
};
use mail_store::{SqliteStore, Store};
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::sync::watch;

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));
const IDENTITY: IdentityId =
    IdentityId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000b1"));
const ADDRESS: &str = "me@example.test";
const IMAP_SCOPE: &str = "https://outlook.office.com/IMAP.AccessAsUser.All";

fn t0() -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000, 0).unwrap()
}

fn minutes(n: i64) -> TimeDelta {
    TimeDelta::try_minutes(n).unwrap()
}

/// A clock the test moves.
#[derive(Clone)]
struct Clock(Arc<AtomicI64>);

impl Clock {
    fn at(when: DateTime<Utc>) -> Self {
        Self(Arc::new(AtomicI64::new(when.timestamp())))
    }

    fn set(&self, when: DateTime<Utc>) {
        self.0.store(when.timestamp(), Ordering::SeqCst);
    }

    fn now(&self) -> Now {
        let seconds = self.0.clone();
        Arc::new(move || {
            DateTime::from_timestamp(seconds.load(Ordering::SeqCst), 0).expect("in range")
        })
    }
}

// ---------------------------------------------------------------------------------------------
// An HTTP endpoint that answers from a script: the token endpoint, and Graph.

/// One request, as the listener received it.
#[derive(Debug, Default, Clone)]
struct Request {
    line: String,
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

    /// A form field of the body, decoded.
    fn form(&self, name: &str) -> Option<String> {
        url::form_urlencoded::parse(self.body.as_bytes())
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.into_owned())
    }

    /// The `scope` field as a set, since its order is not the subject.
    fn scopes(&self) -> BTreeSet<String> {
        self.form("scope")
            .unwrap_or_default()
            .split(' ')
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect()
    }
}

type Seen = Arc<Mutex<Vec<Request>>>;

/// Answer the n-th request with the n-th reply, and every later one with the last.
async fn http(script: Vec<(&'static str, String)>) -> (u16, Seen) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen: Seen = Arc::default();
    let recording = seen.clone();
    let script = Arc::new(script);
    tokio::spawn(async move {
        while let Ok((sock, _)) = listener.accept().await {
            let (seen, script) = (recording.clone(), script.clone());
            tokio::spawn(async move {
                let mut reader = BufReader::new(sock);
                let mut request = Request::default();
                if reader.read_line(&mut request.line).await.unwrap_or(0) == 0 {
                    return;
                }
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
                let (status, answer) = {
                    let mut seen = seen.lock().unwrap();
                    seen.push(request);
                    let turn = (seen.len() - 1).min(script.len() - 1);
                    script[turn].clone()
                };
                let reply = format!(
                    "HTTP/1.1 {status}\r\ncontent-type: application/json\r\n\
                     content-length: {}\r\nconnection: close\r\n\r\n{answer}",
                    answer.len()
                );
                let _ = reader.get_mut().write_all(reply.as_bytes()).await;
            });
        }
    });
    (port, seen)
}

/// A token endpoint's answer: a new access token good for an hour, and perhaps a new refresh
/// token.
fn issued(access: &str, refresh: Option<&str>) -> (&'static str, String) {
    let refresh = refresh
        .map(|r| format!(r#","refresh_token":"{r}""#))
        .unwrap_or_default();
    (
        "200 OK",
        format!(
            r#"{{"access_token":"{access}","token_type":"Bearer","expires_in":3600{refresh}}}"#
        ),
    )
}

/// What an issuer says about a grant that has been revoked.
fn revoked() -> (&'static str, String) {
    (
        "400 Bad Request",
        r#"{"error":"invalid_grant","error_description":"The refresh token has been revoked."}"#
            .to_owned(),
    )
}

async fn token_endpoint(script: Vec<(&'static str, String)>) -> (Registration, Seen) {
    let (port, seen) = http(script).await;
    let registration = Registration::new(OAuthIssuer::Microsoft, "client-id").at(Endpoints {
        auth: format!("http://127.0.0.1:{port}/authorize"),
        token: format!("http://127.0.0.1:{port}/token"),
    });
    (registration, seen)
}

// ---------------------------------------------------------------------------------------------
// An IMAP server that accepts some bearer tokens and refuses the rest.

/// Every bearer token a connection presented, in order.
type Presented = Arc<Mutex<Vec<String>>>;

async fn imap(accepts: &'static [&'static str]) -> (u16, Presented) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let presented: Presented = Arc::default();
    let recording = presented.clone();
    tokio::spawn(async move {
        while let Ok((sock, _)) = listener.accept().await {
            let presented = recording.clone();
            tokio::spawn(async move {
                let mut reader = BufReader::new(sock);
                let greeting = b"* OK [CAPABILITY IMAP4rev1 AUTH=XOAUTH2] ready\r\n";
                if reader.get_mut().write_all(greeting).await.is_err() {
                    return;
                }
                loop {
                    let mut line = String::new();
                    match reader.read_line(&mut line).await {
                        Ok(0) | Err(_) => return,
                        Ok(_) => {}
                    }
                    let line = line.trim_end();
                    let (tag, rest) = line.split_once(' ').unwrap_or((line, ""));
                    let upper = rest.to_ascii_uppercase();
                    let reply = if upper.starts_with("AUTHENTICATE XOAUTH2 ") {
                        let token = bearer(&rest["AUTHENTICATE XOAUTH2 ".len()..]);
                        presented.lock().unwrap().push(token.clone());
                        if accepts.contains(&token.as_str()) {
                            format!("{tag} OK authenticated\r\n")
                        } else {
                            format!("{tag} NO [AUTHENTICATIONFAILED] Invalid credentials\r\n")
                        }
                    } else if upper.starts_with("CAPABILITY") {
                        format!("* CAPABILITY IMAP4rev1 AUTH=XOAUTH2\r\n{tag} OK done\r\n")
                    } else if upper.starts_with("LIST") {
                        format!("* LIST (\\HasNoChildren) \"/\" \"INBOX\"\r\n{tag} OK done\r\n")
                    } else if upper.starts_with("LOGOUT") {
                        format!("* BYE\r\n{tag} OK bye\r\n")
                    } else {
                        format!("{tag} OK\r\n")
                    };
                    if reader.get_mut().write_all(reply.as_bytes()).await.is_err() {
                        return;
                    }
                }
            });
        }
    });
    (port, presented)
}

/// The bearer token inside an `XOAUTH2` initial response.
fn bearer(initial: &str) -> String {
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(initial.trim())
        .unwrap();
    let decoded = String::from_utf8(decoded).unwrap();
    decoded
        .split('\x01')
        .find_map(|part| part.strip_prefix("auth=Bearer "))
        .unwrap_or_default()
        .to_owned()
}

// ---------------------------------------------------------------------------------------------
// The account: Microsoft, reading over IMAP and sending through Graph.

fn plan(imap_port: u16) -> AccountPlan {
    AccountPlan {
        address: ADDRESS.to_owned(),
        incoming: Incoming::Imap {
            host: "127.0.0.1".to_owned(),
            port: imap_port,
            tls: Tls::Plaintext,
        },
        outgoing: Outgoing::Graph,
        // What `send_through_graph` makes of a Microsoft sign-in: two resources, one consent.
        auth: AuthPlan::OAuth {
            issuer: OAuthIssuer::Microsoft,
            scopes: vec![
                IMAP_SCOPE.to_owned(),
                "offline_access".to_owned(),
                "openid".to_owned(),
                presets::GRAPH_SEND_SCOPE.to_owned(),
            ],
        },
        identities: vec![identity()],
    }
}

fn identity() -> Identity {
    Identity {
        id: IDENTITY,
        account: ACCOUNT,
        from: Address {
            name: Some("Me".to_owned()),
            email: ADDRESS.to_owned(),
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
        observed_at: t0(),
    }
}

/// An OAuth credential expiring at `expires_at`.
fn token(access: &str, refresh: &str, expires_at: DateTime<Utc>) -> Credential {
    Credential::OAuth {
        access: access.to_owned(),
        refresh: refresh.to_owned(),
        expires_at,
    }
}

fn key(purpose: SecretPurpose) -> SecretKey {
    SecretKey {
        account: ACCOUNT,
        purpose,
    }
}

fn access(secrets: &dyn Secrets, purpose: SecretPurpose) -> String {
    match secrets.get(&key(purpose)).unwrap() {
        Credential::OAuth { access, .. } => access,
        other => panic!("{other:?}"),
    }
}

fn store() -> (Arc<SqliteStore>, tempfile::TempDir) {
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
    (store, dir)
}

/// The keyring as `account add` leaves it, with `incoming` as the sign-in's token.
fn secrets(incoming: &Credential) -> Arc<dyn Secrets> {
    let secrets: Arc<dyn Secrets> = Arc::new(MapSecrets::default());
    for purpose in [SecretPurpose::IncomingPassword, SecretPurpose::OAuthRefresh] {
        secrets.put(&key(purpose), incoming).unwrap();
    }
    secrets
}

struct Reading {
    engine: AccountEngine<ImapBackend>,
    secrets: Arc<dyn Secrets>,
    clock: Clock,
    _dir: tempfile::TempDir,
}

/// One engine for the account, built once, as `mailo watch` builds it.
///
/// The session factory reads its credential from the renewal's cell on every connection, which
/// is the other half of the contract [`AccountEngine::with_renewal`] states.
fn reading(imap_port: u16, registration: Registration, incoming: Credential) -> Reading {
    let (store, dir) = store();
    let secrets = secrets(&incoming);
    let clock = Clock::at(t0());
    let held = Held::new(incoming);
    let renewal = Renewal::new(
        ACCOUNT,
        &plan(imap_port),
        registration,
        secrets.clone(),
        held.clone(),
    )
    .unwrap()
    .with_clock(clock.now());
    let backend = ImapBackend::new(
        ACCOUNT,
        caps(),
        Box::new(move |authenticate, commands: Vec<ImapCommand>| {
            let mut all = Vec::new();
            if authenticate == Authenticate::First {
                all.push(ImapCommand::AuthenticateXoauth2);
            }
            all.extend(commands);
            ImapSession::new(
                ImapAuth {
                    username: ADDRESS.to_owned(),
                    credential: held.current(),
                    sasl: vec![SaslMech::XOauth2],
                },
                all,
            )
        }),
    );
    let engine = AccountEngine::new(ACCOUNT, plan(imap_port), backend, store, secrets.clone())
        .with_renewal(renewal);
    Reading {
        engine,
        secrets,
        clock,
        _dir: dir,
    }
}

async fn ask(it: &mut Reading) -> Result<AccountCaps, RuntimeError> {
    let (_tx, mut cancel) = watch::channel(false);
    let now = t0();
    it.engine.refresh_caps(&mut cancel, now).await
}

// ---------------------------------------------------------------------------------------------

/// The headline: one engine, a clock that moves past the token's hour, and no restart.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_engine_that_outlives_its_token_renews_it_without_restarting() {
    let (imap_port, presented) = imap(&["first", "second"]).await;
    let (registration, asked) = token_endpoint(vec![issued("second", None)]).await;
    let mut it = reading(
        imap_port,
        registration,
        token("first", "r1", t0() + minutes(60)),
    );

    // With most of its hour left, the token is presented as it is and the issuer hears nothing.
    ask(&mut it).await.expect("the first token is good");
    assert!(asked.lock().unwrap().is_empty(), "renewed a good token");

    // Two minutes from expiry: inside the margin, so renewed before anything presents it.
    it.clock.set(t0() + minutes(58));
    ask(&mut it).await.expect("renewed in place");
    assert_eq!(
        asked.lock().unwrap().len(),
        1,
        "asked once, not per connection"
    );
    let presented = presented.lock().unwrap().clone();
    assert!(
        presented.iter().all(|t| t == "first" || t == "second"),
        "{presented:?}"
    );
    assert_eq!(
        presented.last().map(String::as_str),
        Some("second"),
        "the renewed token never reached a connection: {presented:?}"
    );
    assert!(
        presented
            .iter()
            .skip_while(|t| *t == "first")
            .all(|t| t == "second"),
        "the old token was presented after the renewal: {presented:?}"
    );

    // Saved where the next process will look, under both keys `account add` wrote.
    for purpose in [SecretPurpose::IncomingPassword, SecretPurpose::OAuthRefresh] {
        assert_eq!(
            access(it.secrets.as_ref(), purpose),
            "second",
            "{purpose:?}"
        );
    }

    // And the new token has its own hour: nothing more is asked half an hour on.
    it.clock.set(t0() + minutes(90));
    ask(&mut it).await.expect("still good");
    assert_eq!(asked.lock().unwrap().len(), 1);
}

/// A refusal of a token that looked valid is answered with one renewal and one more attempt.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_refused_token_that_looked_valid_is_renewed_once_and_the_operation_retried() {
    // The server stopped accepting "revoked" long before its expiry says it should have.
    let (imap_port, presented) = imap(&["fresh"]).await;
    let (registration, asked) = token_endpoint(vec![issued("fresh", None)]).await;
    let mut it = reading(
        imap_port,
        registration,
        token("revoked", "r1", t0() + minutes(50)),
    );

    ask(&mut it).await.expect("the retry succeeded");

    assert_eq!(asked.lock().unwrap().len(), 1, "one renewal");
    let presented = presented.lock().unwrap().clone();
    assert_eq!(
        presented.iter().filter(|t| *t == "revoked").count(),
        1,
        "the refused token was presented once and then retired: {presented:?}"
    );
    assert_eq!(presented.first().map(String::as_str), Some("revoked"));
    assert_eq!(
        access(it.secrets.as_ref(), SecretPurpose::IncomingPassword),
        "fresh"
    );
}

/// A server that refuses the renewed token too is not a reason to renew again.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_server_that_refuses_the_renewed_token_too_does_not_start_a_loop() {
    let (imap_port, presented) = imap(&[]).await;
    let (registration, asked) = token_endpoint(vec![issued("fresh", None)]).await;
    let mut it = reading(
        imap_port,
        registration,
        token("looked-fine", "r1", t0() + minutes(50)),
    );

    for _ in 0..3 {
        let e = ask(&mut it).await.expect_err("the server accepts nothing");
        assert!(matches!(e.retry(), Retry::NeedsReauth), "{e}");
    }

    assert_eq!(
        asked.lock().unwrap().len(),
        1,
        "the issuer was asked again for a token the server had already refused"
    );
    // The first operation: refused, renewed, refused. The two after it: refused, and left there.
    assert_eq!(presented.lock().unwrap().len(), 4, "{:?}", presented.lock());
}

/// An issuer that refuses to renew has revoked the grant, and the user has to sign in again.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_refused_renewal_asks_the_user_to_sign_in_and_is_not_repeated() {
    let (imap_port, presented) = imap(&["never"]).await;
    let (registration, asked) = token_endpoint(vec![revoked()]).await;
    let mut it = reading(
        imap_port,
        registration,
        token("expired", "r1", t0() - minutes(5)),
    );

    for _ in 0..3 {
        let e = ask(&mut it)
            .await
            .expect_err("there is no token to sign in with");
        // `NeedsReauth` is what stops a watch and raises the account's needs-attention state.
        assert!(matches!(e.retry(), Retry::NeedsReauth), "{e}");
        let said = e.to_string();
        assert!(
            said.contains("invalid_grant"),
            "the issuer's reason was lost: {said}"
        );
        assert!(said.contains("mailo account add me@example.test"), "{said}");
    }

    assert_eq!(
        asked.lock().unwrap().len(),
        1,
        "a revoked grant was asked again"
    );
    assert!(
        presented.lock().unwrap().is_empty(),
        "a token known to be expired was presented to the server anyway"
    );
}

/// An issuer that cannot be reached has refused nothing, and the account does not need the user.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unreachable_issuer_is_waited_out_rather_than_called_a_refusal() {
    let (imap_port, _presented) = imap(&["fresh"]).await;
    // Bound and dropped, so nothing is listening there.
    let closed = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let registration = Registration::new(OAuthIssuer::Microsoft, "client-id").at(Endpoints {
        auth: format!("http://127.0.0.1:{closed}/authorize"),
        token: format!("http://127.0.0.1:{closed}/token"),
    });
    let mut it = reading(
        imap_port,
        registration,
        token("expired", "r1", t0() - minutes(5)),
    );

    let e = ask(&mut it).await.expect_err("the issuer is down");
    assert!(matches!(e.retry(), Retry::After(_)), "{e}: {:?}", e.retry());
}

/// Microsoft issues one token per resource and refuses a request that names two.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn microsoft_renews_each_resource_with_its_own_scopes() {
    let (registration, asked) =
        token_endpoint(vec![issued("imap-new", None), issued("graph-new", None)]).await;
    let secrets = secrets(&token("imap-old", "r1", t0() - minutes(1)));
    secrets
        .put(
            &key(SecretPurpose::OutgoingPassword),
            &token("graph-old", "r1", t0() - minutes(1)),
        )
        .unwrap();
    let held = Held::new(token("imap-old", "r1", t0() - minutes(1)));
    let renewal = Renewal::new(
        ACCOUNT,
        &plan(1),
        registration,
        secrets.clone(),
        held.clone(),
    )
    .unwrap()
    .with_clock(Clock::at(t0()).now());

    renewal.ahead(Token::Incoming).await.unwrap();
    renewal.ahead(Token::Sending).await.unwrap();

    let asked = asked.lock().unwrap().clone();
    assert_eq!(asked.len(), 2);
    let set = |scopes: &[&str]| {
        scopes
            .iter()
            .map(|s| (*s).to_owned())
            .collect::<BTreeSet<_>>()
    };
    assert_eq!(
        asked[0].scopes(),
        set(&[IMAP_SCOPE, "offline_access", "openid"]),
        "the IMAP token named another resource"
    );
    assert_eq!(
        asked[1].scopes(),
        set(&[presets::GRAPH_SEND_SCOPE, "offline_access"]),
        "the Graph token named another resource"
    );
    assert_eq!(
        access(secrets.as_ref(), SecretPurpose::IncomingPassword),
        "imap-new"
    );
    assert_eq!(
        access(secrets.as_ref(), SecretPurpose::OutgoingPassword),
        "graph-new"
    );
    match held.current() {
        Credential::OAuth { access, .. } => assert_eq!(access, "imap-new"),
        other => panic!("{other:?}"),
    }
}

/// An issuer that rotates the refresh token expects the new one next time.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_rotated_refresh_token_is_saved_and_spent_next_time() {
    let (registration, asked) = token_endpoint(vec![
        issued("second", Some("r2")),
        issued("third", Some("r3")),
    ])
    .await;
    let secrets = secrets(&token("first", "r1", t0() - minutes(1)));
    let clock = Clock::at(t0());
    let renewal = Renewal::new(
        ACCOUNT,
        &plan(1),
        registration,
        secrets.clone(),
        Held::new(token("first", "r1", t0() - minutes(1))),
    )
    .unwrap()
    .with_clock(clock.now());

    renewal.ahead(Token::Incoming).await.unwrap();
    for purpose in [SecretPurpose::IncomingPassword, SecretPurpose::OAuthRefresh] {
        match secrets.get(&key(purpose)).unwrap() {
            Credential::OAuth { refresh, .. } => assert_eq!(refresh, "r2", "{purpose:?}"),
            other => panic!("{other:?}"),
        }
    }

    clock.set(t0() + minutes(120));
    renewal.ahead(Token::Incoming).await.unwrap();
    let asked = asked.lock().unwrap().clone();
    assert_eq!(asked[0].form("refresh_token").as_deref(), Some("r1"));
    assert_eq!(
        asked[1].form("refresh_token").as_deref(),
        Some("r2"),
        "the rotated refresh token was not the one spent"
    );
}

// ---------------------------------------------------------------------------------------------
// Sending through Graph, with its own token.

struct Sending {
    engine: AccountEngine<Pop3Backend>,
    secrets: Arc<dyn Secrets>,
    _store: Arc<SqliteStore>,
    _dir: tempfile::TempDir,
}

/// A message queued for an account that sends through Graph at `graph_port`, holding `graph`
/// as its Graph token.
fn queued(graph_port: u16, registration: Registration, graph: Credential) -> Sending {
    let (store, dir) = store();
    let secrets = secrets(&token("imap-token", "r1", t0() + minutes(60)));
    secrets
        .put(&key(SecretPurpose::OutgoingPassword), &graph)
        .unwrap();

    let draft = Draft {
        id: DraftId::generate(),
        account: ACCOUNT,
        identity: IDENTITY,
        to: vec![Address {
            name: None,
            email: "bea@example.test".to_owned(),
        }],
        cc: Vec::new(),
        bcc: Vec::new(),
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
        updated: t0(),
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
                mail_from: post.mail_from.clone(),
                rcpt_to: post.rcpt_to.clone(),
            },
            &Patch {
                id: ChangeId::generate(),
                changes: Vec::new(),
            },
            t0(),
        )
        .unwrap()
        .unwrap();

    // Never reached: submission goes to Graph, and nothing here reads mail.
    let backend = Pop3Backend::new(
        ACCOUNT,
        caps(),
        Box::new(|auth, commands| {
            let mut all = Vec::new();
            if auth == Authenticate::First {
                all.push(Pop3Command::AuthPlain);
            }
            all.extend(commands);
            Pop3Session::new(ADDRESS, "unused", all)
        }),
    );
    let renewal = Renewal::new(
        ACCOUNT,
        &plan(1),
        registration,
        secrets.clone(),
        Held::new(token("imap-token", "r1", t0() + minutes(60))),
    )
    .unwrap()
    .with_clock(Clock::at(t0()).now());
    let engine = AccountEngine::new(ACCOUNT, plan(1), backend, store.clone(), secrets.clone())
        .with_graph_url(format!("http://127.0.0.1:{graph_port}/v1.0/me/sendMail"))
        .with_renewal(renewal);
    Sending {
        engine,
        secrets,
        _store: store,
        _dir: dir,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_graph_token_near_expiry_is_renewed_before_sending() {
    let (graph_port, posted) = http(vec![("202 Accepted", String::new())]).await;
    let (registration, asked) = token_endpoint(vec![issued("graph-new", None)]).await;
    let mut it = queued(
        graph_port,
        registration,
        token("graph-old", "r1", t0() + minutes(2)),
    );
    let (_tx, mut cancel) = watch::channel(false);

    let report = it.engine.drain_outbox(&mut cancel, t0()).await.unwrap();

    assert_eq!(report.submitted, 1, "{report:?}");
    let posted = posted.lock().unwrap().clone();
    assert_eq!(posted.len(), 1);
    assert_eq!(posted[0].header("authorization"), Some("Bearer graph-new"));
    let asked = asked.lock().unwrap().clone();
    assert_eq!(asked.len(), 1);
    assert!(
        asked[0].scopes().contains(presets::GRAPH_SEND_SCOPE)
            && !asked[0].scopes().contains(IMAP_SCOPE),
        "{:?}",
        asked[0].scopes()
    );
    assert_eq!(
        access(it.secrets.as_ref(), SecretPurpose::OutgoingPassword),
        "graph-new"
    );
    // The incoming token was not the subject and is untouched.
    assert_eq!(
        access(it.secrets.as_ref(), SecretPurpose::IncomingPassword),
        "imap-token"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_graph_401_renews_the_graph_token_once_and_sends_again() {
    let (graph_port, posted) = http(vec![
        ("401 Unauthorized", r#"{"error":{"code":"InvalidAuthenticationToken","message":"Access token has expired."}}"#.to_owned()),
        ("202 Accepted", String::new()),
    ])
    .await;
    let (registration, asked) = token_endpoint(vec![issued("graph-new", None)]).await;
    let mut it = queued(
        graph_port,
        registration,
        token("graph-looked-fine", "r1", t0() + minutes(40)),
    );
    let (_tx, mut cancel) = watch::channel(false);

    let report = it.engine.drain_outbox(&mut cancel, t0()).await.unwrap();

    assert_eq!(report.submitted, 1, "{report:?}");
    assert!(!report.needs_reauth, "{report:?}");
    let posted = posted.lock().unwrap().clone();
    let bearers: Vec<_> = posted.iter().map(|r| r.header("authorization")).collect();
    assert_eq!(
        bearers,
        vec![Some("Bearer graph-looked-fine"), Some("Bearer graph-new")]
    );
    assert_eq!(asked.lock().unwrap().len(), 1, "exactly one renewal");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_graph_refusal_after_renewing_surfaces_rather_than_renewing_again() {
    let (graph_port, posted) = http(vec![(
        "401 Unauthorized",
        r#"{"error":{"code":"InvalidAuthenticationToken","message":"nope"}}"#.to_owned(),
    )])
    .await;
    let (registration, asked) = token_endpoint(vec![issued("graph-new", None)]).await;
    let mut it = queued(
        graph_port,
        registration,
        token("graph-looked-fine", "r1", t0() + minutes(40)),
    );
    let (_tx, mut cancel) = watch::channel(false);

    let report = it.engine.drain_outbox(&mut cancel, t0()).await.unwrap();

    assert_eq!(report.submitted, 0);
    assert!(report.needs_reauth, "{report:?}");
    assert!(!report.needs_attention.is_empty(), "{report:?}");
    assert_eq!(posted.lock().unwrap().len(), 2, "one attempt and one retry");
    assert_eq!(asked.lock().unwrap().len(), 1);
}
