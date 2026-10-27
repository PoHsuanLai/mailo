//! New-mail notifications from the store and from a real watch loop.
//!
//! Nothing here raises a desktop notification: every test hands the loop a recorder. The pure
//! halves — which arrival is announced, how a burst is batched — are tables in `notify.rs`; this
//! file is what needs a store or a socket: the floor surviving a restart, a first backfill staying
//! quiet, and a watch's passes fetching new mail from an IMAP server and announcing it.

use crate::support::account;
use chrono::{DateTime, TimeZone, Utc};
use mail_core::notify::{self, Notification, Notifier, Opens};
use mail_core::sync;
use mail_domain::*;
use mail_proto::backend::{Authenticate, ImapBackend};
use mail_proto::{ImapAuth, ImapCommand, ImapSession};
use mail_runtime::AccountEngine;
use mail_store::{SqliteStore, Store};
use porter_core::Credential;
use porter_core::SecretText;
use porter_secrets::MemorySecrets;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const ME: &str = "me@example.test";

/// Every notification the loop raised, in order.
#[derive(Debug, Default)]
struct Recorder(Mutex<Vec<Notification>>);

impl Notifier for Recorder {
    fn show(&self, notification: &Notification) {
        self.0.lock().unwrap().push(notification.clone());
    }
}

impl Recorder {
    fn seen(&self) -> Vec<Notification> {
        self.0.lock().unwrap().clone()
    }
}

fn open(dir: &std::path::Path) -> SqliteStore {
    SqliteStore::open(dir.join("mail.db"), dir).unwrap()
}

fn with_account(store: &SqliteStore, plan: &AccountPlan) {
    mail_store::testing::seed_account_plan(store, account(), ME, plan, None);
}

fn plan(port: u16) -> AccountPlan {
    AccountPlan {
        address: ME.to_owned(),
        incoming: Incoming::Imap {
            host: "127.0.0.1".to_owned(),
            port,
            tls: Tls::Plaintext,
        },
        outgoing: Outgoing::Smtp {
            host: "127.0.0.1".to_owned(),
            port: 1,
            tls: Tls::Plaintext,
        },
        auth: AuthPlan::Password {
            username: Username::SameAsAddress,
            sasl: vec![SaslMech::Plain],
        },
        identities: Vec::new(),
    }
}

#[test]
fn the_floor_is_armed_once_and_a_restart_keeps_it() {
    let dir = tempfile::tempdir().unwrap();
    let first = Utc.with_ymd_and_hms(2026, 9, 24, 9, 0, 0).unwrap();
    let later = Utc.with_ymd_and_hms(2026, 9, 25, 9, 0, 0).unwrap();
    {
        let store = open(dir.path());
        with_account(&store, &plan(1));
        assert_eq!(
            notify::floor::armed(&store, account(), first).unwrap(),
            first
        );
        assert_eq!(
            notify::floor::armed(&store, account(), later).unwrap(),
            first,
            "a later pass does not move it"
        );
    }
    // A new process on the same database: re-arming here would silence everything that arrived
    // while nothing was watching.
    let store = open(dir.path());
    assert_eq!(
        notify::floor::armed(&store, account(), later).unwrap(),
        first
    );
}

/// A message as a header pass would have stored it.
fn stored(store: &SqliteStore, n: u128, from: &str, date: DateTime<Utc>) -> MessageId {
    stored_as(store, n, from, date, &format!("message {n}"), Body::Absent)
}

/// A message with this `subject` and `body`, as a pass would have stored it.
fn stored_as(
    store: &SqliteStore,
    n: u128,
    from: &str,
    date: DateTime<Utc>,
    subject: &str,
    body: Body,
) -> MessageId {
    let id = MessageId::from_uuid(uuid::Uuid::from_u128(0x9000 + n));
    let message = Message {
        id,
        thread: ThreadId::from_uuid(uuid::Uuid::from_u128(0x7000 + n)),
        account: account(),
        key: MessageKey::Rfc(format!("m{n}@example.test")),
        date,
        from: Address {
            name: None,
            email: from.to_owned(),
        },
        reply_to: Vec::new(),
        to: Vec::new(),
        cc: Vec::new(),
        bcc: Vec::new(),
        subject: subject.to_owned(),
        in_reply_to: None,
        references: Vec::new(),
        rfc_message_id: None,
        read: ReadState::Unread,
        star: Star::Unstarred,
        flag: mail_domain::Hue::Red,
        mailbox: MailboxRole::Inbox,
        labels: Vec::new(),
        body,
        attachments: Vec::new(),
    };
    store
        .apply(
            account(),
            &Patch {
                id: ChangeId::generate(),
                changes: vec![Change::MessageUpsert(Box::new(message))],
            },
        )
        .unwrap();
    id
}

#[test]
fn an_accounts_first_backfill_raises_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(dir.path());
    with_account(&store, &plan(1));
    let watched = Utc.with_ymd_and_hms(2026, 9, 24, 9, 0, 0).unwrap();
    // Two hundred messages from the years before, all unread, all stored by this pass.
    let arrived: Vec<MessageId> = (0..200)
        .map(|n| {
            stored(
                &store,
                n,
                "ada@example.test",
                watched - chrono::Duration::days(n as i64 + 1),
            )
        })
        .collect();
    let recorder = Recorder::default();
    let raised = notify::announce(
        &store,
        account(),
        &arrived,
        &recorder,
        notify::Whom::Everyone,
        watched,
    )
    .unwrap();
    assert_eq!(raised, 0);
    assert!(recorder.seen().is_empty(), "{:?}", recorder.seen());

    // The next pass stores one that is actually new, and one this user sent from elsewhere.
    let later = watched + chrono::Duration::minutes(5);
    let new = stored(&store, 500, "bob@example.test", later);
    let mine = stored(&store, 501, "ME@example.test", later);
    notify::announce(
        &store,
        account(),
        &[new, mine],
        &recorder,
        notify::Whom::Everyone,
        later,
    )
    .unwrap();
    let seen = recorder.seen();
    assert_eq!(seen.len(), 1, "{seen:?}");
    assert_eq!(seen[0].summary, "bob@example.test");
    assert_eq!(seen[0].body, "message 500");
    assert_eq!(
        seen[0].opens,
        Opens::Thread(ThreadId::from_uuid(uuid::Uuid::from_u128(0x7000 + 500)))
    );
}

/// A new message with a one-time code is announced as every other is, by its sender and its
/// subject, as Apple Mail, Gmail and Outlook do: the code is never the headline, never offered
/// to copy and never copied on arrival, whether it is in the subject, in a body not here or in
/// the HTML of a message that has no plain part.
#[test]
fn an_announcement_is_the_ordinary_banner_whatever_code_the_message_carries() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(dir.path());
    with_account(&store, &plan(1));
    let watched = Utc.with_ymd_and_hms(2026, 9, 24, 9, 0, 0).unwrap();
    notify::floor::armed(&store, account(), watched).unwrap();
    let later = watched + chrono::Duration::minutes(5);
    let headers = stored_as(
        &store,
        600,
        "no-reply@service.example",
        later,
        "482913 is your Example code",
        Body::Absent,
    );
    let html = b"From: no-reply@bank.example\r\nContent-Type: text/html; charset=utf-8\r\n\r\n\
        <html><body><p>Your verification code is <b>771204</b>.</p></body></html>\r\n";
    let raw = store.blobs().put(html).unwrap();
    let html_only = stored_as(
        &store,
        601,
        "no-reply@bank.example",
        later,
        "Sign in to your bank",
        Body::Present { text: None, raw },
    );
    let plain = stored(&store, 602, "ada@example.test", later);

    let recorder = Recorder::default();
    let raised = notify::announce(
        &store,
        account(),
        &[headers, html_only, plain],
        &recorder,
        notify::Whom::Everyone,
        later,
    )
    .unwrap();
    assert_eq!(raised, 3);
    let seen = recorder.seen();
    assert_eq!(seen[0].summary, "no-reply@service.example");
    assert_eq!(seen[0].body, "482913 is your Example code");
    assert_eq!(seen[1].summary, "no-reply@bank.example");
    assert_eq!(seen[1].body, "Sign in to your bank");
    assert_eq!(seen[2].summary, "ada@example.test");
    assert!(
        seen.iter().all(|n| !n.summary.contains("771204")),
        "a code read from a body is not on the banner"
    );
}

/// "Notify for: VIPs only": of one pass's new mail only the VIP's is said, the VIPs read from
/// the store as the pass ends, so one marked between passes is heard on the next.
#[test]
fn vips_only_says_a_vip_s_new_mail_and_holds_back_the_rest() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(dir.path());
    with_account(&store, &plan(1));
    let watched = Utc.with_ymd_and_hms(2026, 9, 24, 9, 0, 0).unwrap();
    notify::floor::armed(&store, account(), watched).unwrap();
    let recorder = Recorder::default();
    store.put_vip("Dana@Example.test", Some("Dana")).unwrap();

    let later = watched + chrono::Duration::minutes(5);
    let dana = stored(&store, 600, "dana@example.test", later);
    let bob = stored(&store, 601, "bob@example.test", later);
    let raised = notify::announce(
        &store,
        account(),
        &[dana, bob],
        &recorder,
        notify::Whom::Vips,
        later,
    )
    .unwrap();
    let seen = recorder.seen();
    assert_eq!(raised, 1, "{seen:?}");
    assert_eq!(seen[0].summary, "dana@example.test");

    // Bob marked now: his next message is heard, and everyone's again under All new mail.
    store.put_vip("bob@example.test", None).unwrap();
    let again = later + chrono::Duration::minutes(5);
    let from_bob = stored(&store, 602, "bob@example.test", again);
    let from_cy = stored(&store, 603, "cy@example.test", again);
    notify::announce(
        &store,
        account(),
        &[from_bob, from_cy],
        &recorder,
        notify::Whom::Vips,
        again,
    )
    .unwrap();
    let from_eve = stored(&store, 604, "eve@example.test", again);
    notify::announce(
        &store,
        account(),
        &[from_eve],
        &recorder,
        notify::Whom::Everyone,
        again,
    )
    .unwrap();
    let said: Vec<String> = recorder.seen().into_iter().map(|n| n.summary).collect();
    assert_eq!(
        said,
        ["dana@example.test", "bob@example.test", "eve@example.test"]
    );
}

/// What the fake server holds: UID, raw bytes and flags, which a test adds to while the client
/// is watching.
type Maildrop = Arc<Mutex<Vec<(u32, String, Vec<String>)>>>;

fn raw(uid: u32, from: &str, subject: &str, date: DateTime<Utc>) -> String {
    format!(
        "From: {from}\r\n\
         To: {ME}\r\n\
         Subject: {subject}\r\n\
         Date: {}\r\n\
         Message-ID: <{uid}@example.test>\r\n\
         \r\n\
         body of {uid}\r\n",
        date.to_rfc2822()
    )
}

/// An IMAP server on a real socket, after the one in `mail-runtime/tests/runtime/imap_end_to_end.rs`:
/// only as much of RFC 3501 as the client uses, with a maildrop that can grow, and an IDLE that
/// reports the moment it does.
///
/// Also returns how many times a client has begun to `IDLE`, so a test can add mail at a
/// known point in the loop. Mail added between a pass's survey and the `EXAMINE` before its
/// `IDLE` is counted in that `EXAMINE`, not announced by the `IDLE`; the client catches it by
/// comparing `UIDNEXT`, and `a_watch_…` waits for `IDLE` only to keep its ordering exact.
async fn serve(drop: Maildrop) -> (u16, Idling) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let idling = Idling::default();
    let counting = idling.clone();
    tokio::spawn(async move {
        while let Ok((sock, _)) = listener.accept().await {
            tokio::spawn(session(sock, drop.clone(), counting.clone()));
        }
    });
    (port, idling)
}

/// How many `IDLE`s the fake server has begun.
type Idling = Arc<std::sync::atomic::AtomicUsize>;

async fn session(mut sock: tokio::net::TcpStream, drop: Maildrop, idling: Idling) {
    if sock
        .write_all(b"* OK [CAPABILITY IMAP4rev1 IDLE] server ready\r\n")
        .await
        .is_err()
    {
        return;
    }
    let mut buf = Vec::new();
    let mut idle_tag: Option<String> = None;
    // How many messages this connection's SELECT reported. A real server's IDLE reports any
    // change since then, not only one that happens after IDLE begins.
    let mut selected = 0;
    loop {
        let mut chunk = [0u8; 4096];
        let read = match sock.read(&mut chunk).await {
            Ok(0) | Err(_) => return,
            Ok(n) => n,
        };
        buf.extend_from_slice(&chunk[..read]);
        while let Some(at) = buf.windows(2).position(|w| w == b"\r\n") {
            let line = String::from_utf8_lossy(&buf[..at]).to_string();
            buf.drain(..at + 2);
            let mut parts = line.splitn(2, ' ');
            let tag = parts.next().unwrap_or("*").to_owned();
            let upper = parts.next().unwrap_or("").to_uppercase();
            let held = drop.lock().unwrap().clone();

            let reply = if upper.starts_with("CAPABILITY") {
                format!("* CAPABILITY IMAP4rev1 IDLE\r\n{tag} OK done\r\n")
            } else if upper.starts_with("LOGIN") || upper.starts_with("NOOP") {
                format!("{tag} OK done\r\n")
            } else if upper.starts_with("LIST") {
                format!("* LIST (\\HasNoChildren) \"/\" \"INBOX\"\r\n{tag} OK done\r\n")
            } else if upper.starts_with("LSUB") {
                // A folder listing is `LIST` then `LSUB`; a pass lists folders once per account.
                format!("* LSUB () \"/\" \"INBOX\"\r\n{tag} OK done\r\n")
            } else if upper.starts_with("SELECT") || upper.starts_with("EXAMINE") {
                let next = held.iter().map(|(uid, ..)| uid + 1).max().unwrap_or(1);
                selected = held.len();
                format!(
                    "* {} EXISTS\r\n\
                     * OK [UIDVALIDITY 42] uids valid\r\n\
                     * OK [UIDNEXT {next}] next\r\n\
                     {tag} OK [READ-ONLY] done\r\n",
                    held.len()
                )
            } else if upper.starts_with("UID FETCH") {
                fetch(&held, &upper, &tag)
            } else if upper.starts_with("UID SEARCH") {
                let uids: Vec<String> = held.iter().map(|(uid, ..)| uid.to_string()).collect();
                format!("* SEARCH {}\r\n{tag} OK done\r\n", uids.join(" "))
            } else if upper.starts_with("IDLE") {
                // Parked, as a real server parks, until the maildrop differs from what SELECT
                // reported; then the new count, which is the only thing IDLE is for.
                idle_tag = Some(tag.clone());
                if sock.write_all(b"+ idling\r\n").await.is_err() {
                    return;
                }
                idling.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let grown = loop {
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                    let now = drop.lock().unwrap().len();
                    if now != selected {
                        break now;
                    }
                };
                let _ = sock
                    .write_all(format!("* {grown} EXISTS\r\n").as_bytes())
                    .await;
                continue;
            } else if tag.eq_ignore_ascii_case("DONE") {
                match idle_tag.take() {
                    Some(idle) => format!("{idle} OK idle done\r\n"),
                    None => "* BAD DONE without IDLE\r\n".to_owned(),
                }
            } else if upper.starts_with("LOGOUT") {
                let _ = sock
                    .write_all(format!("* BYE\r\n{tag} OK done\r\n").as_bytes())
                    .await;
                return;
            } else {
                format!("{tag} BAD unknown command\r\n")
            };
            if sock.write_all(reply.as_bytes()).await.is_err() {
                return;
            }
        }
    }
}

/// `UID FETCH`: whole messages, headers, or the survey, answered in mailbox order.
fn fetch(held: &[(u32, String, Vec<String>)], upper: &str, tag: &str) -> String {
    let set = upper
        .strip_prefix("UID FETCH ")
        .and_then(|rest| rest.split(' ').next())
        .unwrap_or("");
    let named: Vec<u32> = set.split(',').filter_map(|n| n.parse().ok()).collect();
    let mut out = String::new();
    for (seq, (uid, raw, flags)) in held.iter().enumerate() {
        if !set.contains(':') && !named.contains(uid) {
            continue;
        }
        let flags = flags.join(" ");
        let item = if upper.contains("BODY.PEEK[HEADER]") {
            let head = raw.split("\r\n\r\n").next().unwrap_or("").to_owned() + "\r\n\r\n";
            format!("FLAGS ({flags}) BODY[HEADER] {{{}}}\r\n{head}", head.len())
        } else if upper.contains("BODY.PEEK[]") {
            format!("BODY[] {{{}}}\r\n{raw}", raw.len())
        } else {
            format!("FLAGS ({flags}) RFC822.SIZE {}", raw.len())
        };
        out.push_str(&format!("* {} FETCH (UID {uid} {item})\r\n", seq + 1));
    }
    out.push_str(&format!("{tag} OK done\r\n"));
    out
}

fn engine(port: u16, store: Arc<SqliteStore>) -> AccountEngine<ImapBackend> {
    let auth = ImapAuth {
        username: ME.to_owned(),
        credential: Credential::Password(SecretText::new("s3cr3t".to_owned())),
        sasl: vec![SaslMech::Plain],
    };
    let backend = ImapBackend::new(
        account(),
        caps(),
        Box::new(move |authenticate, commands: Vec<ImapCommand>| {
            let mut all = Vec::new();
            if authenticate == Authenticate::First {
                all.push(ImapCommand::Login);
            }
            all.extend(commands);
            ImapSession::new(auth.clone(), all)
        }),
    );
    AccountEngine::new(
        account(),
        plan(port),
        backend,
        store,
        Arc::new(MemorySecrets::default()),
    )
}

/// Run a watch's passes for one account until `until` says stop, or give up after a while.
///
/// The scheduler (`mail_runtime::schedule`) decides when a watch passes; what is checked here is
/// what the watch does with each pass that ends, so the passes are run on a short timer and each
/// is handed to [`sync::watcher::told`], as the watch does. It never returns of its own accord,
/// so it is raced against `until`, which the test uses to add mail to the server and decide when
/// it has seen enough.
async fn watching<F: std::future::Future<Output = ()>>(
    port: u16,
    store: &Arc<SqliteStore>,
    recorder: &Recorder,
    until: F,
) {
    let mut engine = engine(port, store.clone());
    let account = sync::Configured {
        id: account(),
        address: ME.to_owned(),
        plan: plan(port),
        caps: caps(),
        keep: mail_core::offline::Keep::Bodies,
        every: mail_core::offline::Every::default(),
        window: mail_core::offline::Window::default(),
        remove: mail_core::offline::Remove::default(),
    };
    let inbox = vec![MailboxRef {
        account: account.id.clone(),
        path: "INBOX".to_owned(),
    }];
    let (_tx, mut cancel) = tokio::sync::watch::channel(false);
    let announce = sync::Announce::To {
        store,
        notifier: recorder,
        whom: notify::Whom::Everyone,
    };
    let loop_ = async {
        loop {
            let at = Utc::now();
            let end = sync::drive(&mut engine, &account, &inbox, &mut cancel, at).await;
            sync::watcher::told(&end, announce, at, &|_| {});
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    };
    let raced = tokio::time::timeout(std::time::Duration::from_secs(20), async {
        tokio::select! {
            _ = loop_ => panic!("the watch loop returned on its own"),
            _ = until => {}
        }
    })
    .await;
    assert!(raced.is_ok(), "the watch never got there");
}

/// Poll `ready` until it holds, yielding to the loop in between.
async fn eventually(ready: impl Fn() -> bool) {
    while !ready() {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn a_watch_announces_what_arrives_and_not_what_was_already_there() {
    let old = Utc.with_ymd_and_hms(2023, 11, 14, 22, 13, 20).unwrap();
    let drop: Maildrop = Arc::new(Mutex::new(vec![
        // Unread, and years old: the first backfill must not announce either.
        (
            101,
            raw(101, "Ada <ada@example.test>", "old news", old),
            vec![],
        ),
        (
            102,
            raw(102, "Bob <bob@example.test>", "older news", old),
            vec![],
        ),
    ]));
    let (port, _idling) = serve(drop.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(open(dir.path()));
    with_account(&store, &plan(port));
    let recorder = Recorder::default();

    let count = |store: &SqliteStore| store.count(&Filter::All, Utc::now()).unwrap();
    watching(port, &store, &recorder, async {
        // The backfill lands, quietly.
        eventually(|| count(&store) == 2).await;
        // Then mail arrives while the loop is watching: one worth announcing, one already read
        // elsewhere, and one this user sent from another device.
        let now = Utc::now() + chrono::Duration::minutes(1);
        drop.lock().unwrap().extend([
            (
                103,
                raw(103, "Cy <cy@example.test>", "lunch on friday", now),
                vec![],
            ),
            (
                104,
                raw(104, "Dee <dee@example.test>", "read on the phone", now),
                vec!["\\Seen".to_owned()],
            ),
            (105, raw(105, ME, "note to self", now), vec![]),
        ]);
        eventually(|| count(&store) == 5 && !recorder.seen().is_empty()).await;
    })
    .await;

    let seen = recorder.seen();
    assert_eq!(seen.len(), 1, "{seen:?}");
    assert_eq!(seen[0].summary, "Cy");
    assert_eq!(seen[0].body, "lunch on friday");
    assert_eq!(seen[0].account, account());
    let Opens::Thread(thread) = seen[0].opens else {
        panic!("one message opens its conversation: {:?}", seen[0].opens);
    };
    assert_eq!(
        store.thread(thread).unwrap().summary.subject,
        "lunch on friday",
        "the thread a click would open is the one announced"
    );

    // A restart: a new loop over the same database. What was there before is not said again —
    // not the backfill, and not the message already announced — and what arrives next is.
    let store = Arc::new(open(dir.path()));
    let again = Recorder::default();
    watching(port, &store, &again, async {
        // A pass over what is already held, with nothing new.
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        assert!(again.seen().is_empty(), "{:?}", again.seen());
        let now = Utc::now() + chrono::Duration::minutes(1);
        drop.lock().unwrap().push((
            106,
            raw(106, "Eve <eve@example.test>", "after the restart", now),
            vec![],
        ));
        eventually(|| !again.seen().is_empty()).await;
    })
    .await;
    let seen = again.seen();
    assert_eq!(seen.len(), 1, "{seen:?}");
    assert_eq!(seen[0].body, "after the restart");
}

fn caps() -> AccountCaps {
    AccountCaps {
        watch: WatchMode::Idle,
        // Fresh, so the loop does not stop to re-read capabilities.
        observed_at: Utc::now(),
        ..crate::support::caps()
    }
}
