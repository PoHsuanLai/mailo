//! New-mail notifications (`plan.md` 10.6) from the store and from a real watch loop.
//!
//! Nothing here raises a desktop notification: every test hands the loop a recorder. The pure
//! halves — which arrival is announced, how a burst is batched — are tables in `notify.rs`; this
//! file is what needs a store or a socket: the floor surviving a restart, a first backfill staying
//! quiet, and `mailo watch`'s own loop fetching new mail from an IMAP server and announcing it.

use chrono::{DateTime, TimeZone, Utc};
use mail_app::notify::{self, Notification, Notifier, Opens};
use mail_app::sync;
use mail_domain::*;
use mail_proto::backend::{Authenticate, ImapBackend};
use mail_proto::{ImapAuth, ImapCommand, ImapSession};
use mail_runtime::{AccountEngine, MapSecrets};
use mail_store::{SqliteStore, Store};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));
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
    store
        .connection()
        .execute(
            "INSERT OR IGNORE INTO accounts (id, address, plan, created_at)
             VALUES (?1, ?2, ?3, datetime('now'))",
            rusqlite::params![
                ACCOUNT.to_string(),
                ME,
                serde_json::to_string(plan).unwrap()
            ],
        )
        .unwrap();
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
fn the_command_line_can_silence_a_watch_and_change_the_setting() {
    use mail_app::cli::{Command, WatchNotify, parse};
    let args = |s: &str| s.split(' ').map(str::to_owned).collect::<Vec<_>>();
    assert_eq!(
        parse(&args("watch")),
        Ok(Command::Watch {
            notify: WatchNotify::AsSet
        })
    );
    assert_eq!(
        parse(&args("watch --no-notify")),
        Ok(Command::Watch {
            notify: WatchNotify::Never
        })
    );
    assert_eq!(
        parse(&args("notify off")),
        Ok(Command::Notify {
            set: Some(notify::Setting::Off)
        })
    );
    assert_eq!(
        parse(&args("notify on")),
        Ok(Command::Notify {
            set: Some(notify::Setting::On)
        })
    );
    assert_eq!(parse(&args("notify")), Ok(Command::Notify { set: None }));
    assert!(parse(&args("notify loudly")).is_err());
    assert!(mail_app::cli::usage().contains("--no-notify"));
}

#[test]
fn the_floor_is_armed_once_and_a_restart_keeps_it() {
    let dir = tempfile::tempdir().unwrap();
    let first = Utc.with_ymd_and_hms(2026, 9, 24, 9, 0, 0).unwrap();
    let later = Utc.with_ymd_and_hms(2026, 9, 25, 9, 0, 0).unwrap();
    {
        let store = open(dir.path());
        with_account(&store, &plan(1));
        assert_eq!(notify::floor::armed(&store, ACCOUNT, first).unwrap(), first);
        assert_eq!(
            notify::floor::armed(&store, ACCOUNT, later).unwrap(),
            first,
            "a later pass does not move it"
        );
    }
    // A new process on the same database: re-arming here would silence everything that arrived
    // while nothing was watching.
    let store = open(dir.path());
    assert_eq!(notify::floor::armed(&store, ACCOUNT, later).unwrap(), first);
}

/// A message as a header pass would have stored it.
fn stored(store: &SqliteStore, n: u128, from: &str, date: DateTime<Utc>) -> MessageId {
    let id = MessageId::from_uuid(uuid::Uuid::from_u128(0x9000 + n));
    let message = Message {
        id,
        thread: ThreadId::from_uuid(uuid::Uuid::from_u128(0x7000 + n)),
        account: ACCOUNT,
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
        subject: format!("message {n}"),
        in_reply_to: None,
        references: Vec::new(),
        rfc_message_id: None,
        read: ReadState::Unread,
        star: Star::Unstarred,
        mailbox: MailboxRole::Inbox,
        labels: Vec::new(),
        body: Body::Absent,
        attachments: Vec::new(),
    };
    store
        .apply(
            ACCOUNT,
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
    let raised = notify::announce(&store, ACCOUNT, &arrived, &recorder, watched).unwrap();
    assert_eq!(raised, 0);
    assert!(recorder.seen().is_empty(), "{:?}", recorder.seen());

    // The next pass stores one that is actually new, and one this user sent from elsewhere.
    let later = watched + chrono::Duration::minutes(5);
    let new = stored(&store, 500, "bob@example.test", later);
    let mine = stored(&store, 501, "ME@example.test", later);
    notify::announce(&store, ACCOUNT, &[new, mine], &recorder, later).unwrap();
    let seen = recorder.seen();
    assert_eq!(seen.len(), 1, "{seen:?}");
    assert_eq!(seen[0].summary, "bob@example.test");
    assert_eq!(seen[0].body, "message 500");
    assert_eq!(
        seen[0].opens,
        Opens::Thread(ThreadId::from_uuid(uuid::Uuid::from_u128(0x7000 + 500)))
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

/// An IMAP server on a real socket, after the one in `mail-runtime/tests/imap_end_to_end.rs`:
/// only as much of RFC 3501 as the client uses, with a maildrop that can grow, and an IDLE that
/// reports the moment it does.
async fn serve(drop: Maildrop) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((sock, _)) = listener.accept().await {
            tokio::spawn(session(sock, drop.clone()));
        }
    });
    port
}

async fn session(mut sock: tokio::net::TcpStream, drop: Maildrop) {
    if sock
        .write_all(b"* OK [CAPABILITY IMAP4rev1 IDLE] server ready\r\n")
        .await
        .is_err()
    {
        return;
    }
    let mut buf = Vec::new();
    let mut idle_tag: Option<String> = None;
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
                // Parked, as a real server parks, until the maildrop grows; then the new count,
                // which is the only thing IDLE is for.
                idle_tag = Some(tag.clone());
                if sock.write_all(b"+ idling\r\n").await.is_err() {
                    return;
                }
                let grown = loop {
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                    let now = drop.lock().unwrap().len();
                    if now != held.len() {
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

fn caps() -> AccountCaps {
    AccountCaps {
        labels: ServerLabels::LocalOnly,
        threads: ServerThreads::Jwz,
        watch: WatchMode::Idle,
        archive: ArchiveMeans::LocalOnly,
        folders: FolderRoles::default(),
        condstore: Condstore::Absent,
        move_ext: MoveExt::Absent,
        expunge: ExpungeMeans::Forbidden,
        top: Supported::Absent,
        pipelining: Supported::Absent,
        connections: ConnectionBudget { max: 1 },
        // Fresh, so the loop does not stop to re-read capabilities.
        observed_at: Utc::now(),
    }
}

fn engine(port: u16, store: Arc<SqliteStore>) -> AccountEngine<ImapBackend> {
    let auth = ImapAuth {
        username: ME.to_owned(),
        credential: Credential::Password("s3cr3t".to_owned()),
        sasl: vec![SaslMech::Plain],
    };
    let backend = ImapBackend::new(
        ACCOUNT,
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
        ACCOUNT,
        plan(port),
        backend,
        store,
        Arc::new(MapSecrets::default()),
    )
}

/// Run `mailo watch`'s loop for one account until `until` says stop, or give up after a while.
///
/// The loop never returns of its own accord — that is what watching is — so it is raced against
/// `until`, which the test uses to add mail to the server and decide when it has seen enough.
async fn watching<F: std::future::Future<Output = ()>>(
    port: u16,
    store: &Arc<SqliteStore>,
    recorder: &Recorder,
    until: F,
) {
    let mut engine = engine(port, store.clone());
    let account = sync::Configured {
        id: ACCOUNT,
        address: ME.to_owned(),
        plan: plan(port),
        caps: caps(),
    };
    let inbox = vec![MailboxRef {
        account: ACCOUNT,
        path: "INBOX".to_owned(),
    }];
    let (_tx, mut cancel) = tokio::sync::watch::channel(false);
    let announce = sync::Announce::To {
        store,
        notifier: recorder,
    };
    let loop_ = sync::drive(
        &mut engine,
        &account,
        &inbox,
        &mut cancel,
        Utc::now(),
        sync::Mode::Watch,
        announce,
    );
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
    let port = serve(drop.clone()).await;
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
    assert_eq!(seen[0].account, ACCOUNT);
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
        // A pass over what is already held, then IDLE with nothing new.
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
