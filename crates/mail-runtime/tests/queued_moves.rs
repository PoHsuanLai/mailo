//! Operations queued behind a move reach the message where the move put it (FINDINGS F153).
//!
//! Over a real socket, against a server with several mailboxes that really moves messages
//! between them and numbers each arrival with the destination's next UID. A second move queued
//! behind a first used to be sent to the UID the first had moved away from; the server answered
//! OK, because a UID set naming nothing is not an error, and the second move was lost.

use chrono::{DateTime, TimeZone, Utc};
use mail_domain::*;
use mail_proto::backend::{Authenticate, ImapBackend};
use mail_proto::{ImapAuth, ImapCommand, ImapSession};
use mail_runtime::{AccountEngine, MapSecrets, Secrets};
use mail_store::{SqliteStore, Store};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::watch;

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a2"));
const PASSWORD: &str = "s3cr3t";

fn now() -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000, 0).unwrap()
}

const RAW: &str = "From: Ada Lovelace <ada@example.test>\r\n\
                   To: me@example.test\r\n\
                   Subject: the engine\r\n\
                   Date: Tue, 14 Nov 2023 22:13:20 +0000\r\n\
                   Message-ID: <engine@example.test>\r\n\
                   \r\n\
                   Notes attached.\r\n";

/// One mailbox on the fake server.
#[derive(Debug, Clone)]
struct Mailbox {
    validity: u32,
    next: u32,
    held: BTreeMap<u32, String>,
}

impl Mailbox {
    fn empty(validity: u32) -> Self {
        Self {
            validity,
            next: 1,
            held: BTreeMap::new(),
        }
    }
}

/// Everything the server holds, and every command it was sent.
#[derive(Debug, Default)]
struct Server {
    mailboxes: BTreeMap<String, Mailbox>,
    commands: Vec<String>,
}

type Shared = Arc<Mutex<Server>>;

/// Whether the server says where a moved message landed: `UIDPLUS`'s `COPYUID`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Uidplus {
    Yes,
    No,
}

async fn serve(uidplus: Uidplus) -> (u16, Shared) {
    let mut inbox = Mailbox::empty(42);
    inbox.held.insert(10, RAW.to_owned());
    inbox.next = 11;
    let server = Server {
        mailboxes: BTreeMap::from([
            ("INBOX".to_owned(), inbox),
            ("Archive".to_owned(), Mailbox::empty(50)),
            ("Trash".to_owned(), Mailbox::empty(60)),
        ]),
        commands: Vec::new(),
    };
    let shared: Shared = Arc::new(Mutex::new(server));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let state = shared.clone();
    tokio::spawn(async move {
        while let Ok((sock, _)) = listener.accept().await {
            tokio::spawn(session(sock, state.clone(), uidplus));
        }
    });
    (port, shared)
}

/// The quoted mailbox name at the end of a command.
fn quoted(rest: &str) -> String {
    let parts: Vec<&str> = rest.split('"').collect();
    parts
        .get(parts.len().saturating_sub(2))
        .unwrap_or(&"")
        .to_string()
}

/// The UIDs a set names, of those held.
fn named(set: &str, held: &BTreeMap<u32, String>) -> Vec<u32> {
    let mut out = Vec::new();
    for part in set.split(',') {
        match part.split_once(':') {
            Some((a, _)) => {
                let from: u32 = a.parse().unwrap_or(1);
                out.extend(held.keys().filter(|uid| **uid >= from));
            }
            None => out.extend(part.parse::<u32>().ok().filter(|u| held.contains_key(u))),
        }
    }
    out
}

async fn session(mut sock: tokio::net::TcpStream, shared: Shared, uidplus: Uidplus) {
    if sock.write_all(b"* OK ready\r\n").await.is_err() {
        return;
    }
    let mut selected = String::new();
    let mut buf = Vec::new();
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
            let (tag, rest) = line.split_once(' ').unwrap_or((&line, ""));
            let upper = rest.to_uppercase();
            let (reply, done) = {
                let mut server = shared.lock().unwrap();
                server.commands.push(if upper.starts_with("LOGIN") {
                    "LOGIN".to_owned()
                } else {
                    rest.to_owned()
                });
                let reply = if upper.starts_with("CAPABILITY") {
                    let plus = if uidplus == Uidplus::Yes {
                        " UIDPLUS"
                    } else {
                        ""
                    };
                    format!("* CAPABILITY IMAP4rev1 MOVE{plus}\r\n{tag} OK done\r\n")
                } else if upper.starts_with("LOGIN") {
                    format!("{tag} OK logged in\r\n")
                } else if upper.starts_with("SELECT") || upper.starts_with("EXAMINE") {
                    selected = quoted(rest);
                    match server.mailboxes.get(&selected) {
                        Some(mailbox) => format!(
                            "* {} EXISTS\r\n* OK [UIDVALIDITY {}] valid\r\n\
                         * OK [UIDNEXT {}] next\r\n{tag} OK done\r\n",
                            mailbox.held.len(),
                            mailbox.validity,
                            mailbox.next
                        ),
                        None => format!("{tag} NO no such mailbox\r\n"),
                    }
                } else if upper.starts_with("UID FETCH") {
                    let set = rest.split(' ').nth(2).unwrap_or("");
                    let mailbox = &server.mailboxes[&selected];
                    let mut out = String::new();
                    for (seq, uid) in named(set, &mailbox.held).into_iter().enumerate() {
                        let raw = &mailbox.held[&uid];
                        let n = seq + 1;
                        if upper.contains("BODY.PEEK[HEADER]") {
                            let head =
                                raw.split("\r\n\r\n").next().unwrap_or("").to_owned() + "\r\n\r\n";
                            out.push_str(&format!(
                            "* {n} FETCH (UID {uid} FLAGS () BODY[HEADER] {{{}}}\r\n{head})\r\n",
                            head.len()
                        ));
                        } else if upper.contains("BODY.PEEK[]") {
                            out.push_str(&format!(
                                "* {n} FETCH (UID {uid} BODY[] {{{}}}\r\n{raw})\r\n",
                                raw.len()
                            ));
                        } else {
                            out.push_str(&format!(
                                "* {n} FETCH (UID {uid} FLAGS () RFC822.SIZE {})\r\n",
                                raw.len()
                            ));
                        }
                    }
                    out.push_str(&format!("{tag} OK done\r\n"));
                    out
                } else if upper.starts_with("UID MOVE") {
                    let set = rest.split(' ').nth(2).unwrap_or("").to_owned();
                    let target = quoted(rest);
                    let uids = named(&set, &server.mailboxes[&selected].held);
                    let mut from = Vec::new();
                    let mut to = Vec::new();
                    for uid in uids {
                        let raw = server
                            .mailboxes
                            .get_mut(&selected)
                            .unwrap()
                            .held
                            .remove(&uid)
                            .unwrap();
                        let dest = server.mailboxes.get_mut(&target).unwrap();
                        let new = dest.next;
                        dest.next += 1;
                        dest.held.insert(new, raw);
                        from.push(uid.to_string());
                        to.push(new.to_string());
                    }
                    let validity = server.mailboxes[&target].validity;
                    // RFC 6851 §4.3: `COPYUID` in an untagged OK, before the tagged one.
                    let copyuid = if uidplus == Uidplus::Yes && !from.is_empty() {
                        format!(
                            "* OK [COPYUID {validity} {} {}] moved\r\n",
                            from.join(","),
                            to.join(",")
                        )
                    } else {
                        String::new()
                    };
                    format!("{copyuid}{tag} OK done\r\n")
                } else if upper.starts_with("UID STORE") {
                    format!("{tag} OK stored\r\n")
                } else if upper.starts_with("LOGOUT") {
                    format!("* BYE\r\n{tag} OK done\r\n")
                } else {
                    format!("{tag} BAD unknown command\r\n")
                };
                (reply, upper.starts_with("LOGOUT"))
            };
            if sock.write_all(reply.as_bytes()).await.is_err() || done {
                return;
            }
        }
    }
}

fn caps() -> AccountCaps {
    AccountCaps {
        labels: ServerLabels::LocalOnly,
        threads: ServerThreads::Jwz,
        watch: WatchMode::Poll {
            every: std::time::Duration::from_secs(300),
        },
        archive: ArchiveMeans::MoveToFolder("Archive".to_owned()),
        folders: FolderRoles(vec![
            ("Archive".to_owned(), MailboxRole::Archive),
            ("Trash".to_owned(), MailboxRole::Trash),
        ]),
        condstore: Condstore::Absent,
        move_ext: MoveExt::Supported,
        expunge: ExpungeMeans::Forbidden,
        top: Supported::Absent,
        pipelining: Supported::Absent,
        connections: ConnectionBudget { max: 1 },
        observed_at: now(),
    }
}

struct Fixture {
    store: Arc<SqliteStore>,
    engine: AccountEngine<ImapBackend>,
    _dir: tempfile::TempDir,
}

fn engine(port: u16) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::open(dir.path().join("mail.db"), dir.path()).unwrap());
    store
        .connection()
        .execute(
            "INSERT INTO accounts (id, address, plan, created_at)
             VALUES (?1, 'me@example.test', '{}', datetime('now'))",
            [ACCOUNT.to_string()],
        )
        .unwrap();
    let secrets = MapSecrets::default();
    secrets
        .put(
            &SecretKey {
                account: ACCOUNT,
                purpose: SecretPurpose::IncomingPassword,
            },
            &Credential::Password(PASSWORD.to_owned()),
        )
        .unwrap();
    let auth = ImapAuth {
        username: "me@example.test".to_owned(),
        credential: Credential::Password(PASSWORD.to_owned()),
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
    let plan = AccountPlan {
        address: "me@example.test".to_owned(),
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
    };
    let engine = AccountEngine::new(ACCOUNT, plan, backend, store.clone(), Arc::new(secrets));
    Fixture {
        store,
        engine,
        _dir: dir,
    }
}

fn mailbox(path: &str) -> MailboxRef {
    MailboxRef {
        account: ACCOUNT,
        path: path.to_owned(),
    }
}

/// The one message, once a sync of the inbox has found it.
fn the_message(store: &SqliteStore) -> MessageId {
    let page = store
        .threads(
            &Query {
                filter: Filter::All,
                sort: Sort {
                    property: Property::Date,
                    dir: SortDir::Desc,
                },
                page: PageReq {
                    after: None,
                    limit: 10,
                },
            },
            now(),
        )
        .unwrap();
    assert_eq!(page.items.len(), 1, "one message held");
    store.thread(page.items[0].id).unwrap().messages[0]
}

/// Queue `intent` as the window does, with an empty undo.
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

fn file_into(message: MessageId, role: MailboxRole) -> RemoteIntent {
    RemoteIntent::SetMailbox {
        messages: vec![message],
        role,
    }
}

/// What the client asked the server to change, in order: selections and moves and stores.
fn changes(server: &Shared) -> Vec<String> {
    server
        .lock()
        .unwrap()
        .commands
        .iter()
        .filter(|c| {
            let upper = c.to_uppercase();
            upper.starts_with("SELECT")
                || upper.starts_with("UID MOVE")
                || upper.starts_with("UID STORE")
        })
        .cloned()
        .collect()
}

fn holds(server: &Shared, path: &str) -> Vec<u32> {
    server.lock().unwrap().mailboxes[path]
        .held
        .keys()
        .copied()
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_second_move_in_the_same_drain_goes_to_the_uid_the_first_was_given() {
    let (port, server) = serve(Uidplus::Yes).await;
    let mut it = engine(port);
    let (_tx, mut cancel) = watch::channel(false);
    it.engine
        .sync(&mailbox("INBOX"), &mut cancel, now(), 200)
        .await
        .unwrap();
    let message = the_message(&it.store);
    server.lock().unwrap().commands.clear();

    // Archived, then — before anything reached the server — trashed.
    queue(&it.store, file_into(message, MailboxRole::Archive));
    queue(&it.store, file_into(message, MailboxRole::Trash));
    let drained = it.engine.drain_outbox(&mut cancel, now()).await.unwrap();

    assert_eq!(drained.outbox_settled, 2, "{:?}", drained.needs_attention);
    assert_eq!(drained.still_queued, 0);
    assert_eq!(
        changes(&server),
        vec![
            "SELECT \"INBOX\"".to_owned(),
            "UID MOVE 10 \"Archive\"".to_owned(),
            // Where the first move put it, by the `COPYUID` it was answered with.
            "SELECT \"Archive\"".to_owned(),
            "UID MOVE 1 \"Trash\"".to_owned(),
        ]
    );
    assert_eq!(holds(&server, "Trash"), vec![1], "the second move happened");
    assert!(holds(&server, "Archive").is_empty());
    assert_eq!(
        it.store.remotes_of(message).unwrap(),
        vec![RemoteRef::Imap {
            mailbox: "Trash".to_owned(),
            uidvalidity: 60,
            uid: 1,
        }]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_flag_change_queued_behind_a_move_is_stored_on_the_moved_message() {
    let (port, server) = serve(Uidplus::Yes).await;
    let mut it = engine(port);
    let (_tx, mut cancel) = watch::channel(false);
    it.engine
        .sync(&mailbox("INBOX"), &mut cancel, now(), 200)
        .await
        .unwrap();
    let message = the_message(&it.store);
    server.lock().unwrap().commands.clear();

    queue(&it.store, file_into(message, MailboxRole::Archive));
    queue(
        &it.store,
        RemoteIntent::SetFlags {
            messages: vec![message],
            read: None,
            star: Some(Star::Starred),
        },
    );
    let drained = it.engine.drain_outbox(&mut cancel, now()).await.unwrap();

    assert_eq!(drained.outbox_settled, 2, "{:?}", drained.needs_attention);
    assert_eq!(
        changes(&server),
        vec![
            "SELECT \"INBOX\"".to_owned(),
            "UID MOVE 10 \"Archive\"".to_owned(),
            "SELECT \"Archive\"".to_owned(),
            "UID STORE 1 +FLAGS (\\Flagged)".to_owned(),
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_copyuid_the_second_move_waits_for_the_sync_that_finds_the_message() {
    let (port, server) = serve(Uidplus::No).await;
    let mut it = engine(port);
    let (_tx, mut cancel) = watch::channel(false);
    it.engine
        .sync(&mailbox("INBOX"), &mut cancel, now(), 200)
        .await
        .unwrap();
    let message = the_message(&it.store);
    server.lock().unwrap().commands.clear();

    queue(&it.store, file_into(message, MailboxRole::Archive));
    queue(&it.store, file_into(message, MailboxRole::Trash));
    let drained = it.engine.drain_outbox(&mut cancel, now()).await.unwrap();

    // The first went; the second has nowhere to go yet and is not sent to UID 10 in the inbox,
    // which the server would have answered OK and done nothing with.
    assert_eq!(drained.outbox_settled, 1, "{:?}", drained.needs_attention);
    assert_eq!(drained.still_queued, 1);
    assert!(
        drained.needs_attention.is_empty(),
        "{:?}",
        drained.needs_attention
    );
    assert_eq!(
        changes(&server),
        vec![
            "SELECT \"INBOX\"".to_owned(),
            "UID MOVE 10 \"Archive\"".to_owned(),
        ]
    );
    assert!(it.store.remotes_of(message).unwrap().is_empty());
    // Waiting cost it nothing: no attempt counted, no backoff.
    let waiting = it
        .store
        .outbox_due(ACCOUNT, now())
        .unwrap()
        .into_iter()
        .next()
        .expect("still queued");
    assert_eq!(waiting.attempts, 0);

    // The next pass: sync first, as `mailo sync` and a watch do, then drain.
    server.lock().unwrap().commands.clear();
    for path in ["INBOX", "Archive"] {
        it.engine
            .sync(&mailbox(path), &mut cancel, now(), 200)
            .await
            .unwrap();
    }
    assert_eq!(
        the_message(&it.store),
        message,
        "found in Archive as the same message, not a new one"
    );
    let drained = it.engine.drain_outbox(&mut cancel, now()).await.unwrap();
    assert_eq!(drained.outbox_settled, 1, "{:?}", drained.needs_attention);
    assert_eq!(drained.still_queued, 0);
    assert_eq!(
        changes(&server),
        vec![
            "SELECT \"Archive\"".to_owned(),
            "UID MOVE 1 \"Trash\"".to_owned(),
        ]
    );
    assert_eq!(holds(&server, "Trash"), vec![1]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_move_made_before_the_sync_finds_where_the_last_one_went_is_sent_after_it() {
    let (port, server) = serve(Uidplus::No).await;
    let mut it = engine(port);
    let (_tx, mut cancel) = watch::channel(false);
    it.engine
        .sync(&mailbox("INBOX"), &mut cancel, now(), 200)
        .await
        .unwrap();
    let message = the_message(&it.store);
    queue(&it.store, file_into(message, MailboxRole::Archive));
    let drained = it.engine.drain_outbox(&mut cancel, now()).await.unwrap();
    assert_eq!(drained.outbox_settled, 1, "{:?}", drained.needs_attention);
    assert!(it.store.remotes_of(message).unwrap().is_empty());

    // The user moves it again between passes. It has no address here now, and it is still mail
    // the server holds: queued, not kept to this client.
    queue(&it.store, file_into(message, MailboxRole::Trash));
    server.lock().unwrap().commands.clear();
    let drained = it.engine.drain_outbox(&mut cancel, now()).await.unwrap();
    assert_eq!(drained.outbox_settled, 0);
    assert_eq!(drained.still_queued, 1);
    assert!(changes(&server).is_empty(), "{:?}", changes(&server));

    for path in ["INBOX", "Archive"] {
        it.engine
            .sync(&mailbox(path), &mut cancel, now(), 200)
            .await
            .unwrap();
    }
    server.lock().unwrap().commands.clear();
    let drained = it.engine.drain_outbox(&mut cancel, now()).await.unwrap();
    assert_eq!(drained.outbox_settled, 1, "{:?}", drained.needs_attention);
    assert_eq!(
        changes(&server),
        vec![
            "SELECT \"Archive\"".to_owned(),
            "UID MOVE 1 \"Trash\"".to_owned(),
        ]
    );
    assert_eq!(holds(&server, "Trash"), vec![1]);
}

/// F154: a message held at two addresses — its copy in the inbox and one in Archive — is
/// starred at both, each UID in its own mailbox. Sent as one `UID STORE 10,7` in the inbox, the
/// 7 named a different message there, which was starred instead.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_operation_on_a_message_in_two_mailboxes_names_each_uid_in_its_own() {
    let (port, server) = serve(Uidplus::Yes).await;
    {
        let mut held = server.lock().unwrap();
        let other = RAW
            .replace("engine@example.test", "other@example.test")
            .replace("the engine", "something else");
        let inbox = held.mailboxes.get_mut("INBOX").unwrap();
        inbox.held.insert(7, other);
        let archive = held.mailboxes.get_mut("Archive").unwrap();
        archive.held.insert(7, RAW.to_owned());
        archive.next = 8;
    }
    let mut it = engine(port);
    let (_tx, mut cancel) = watch::channel(false);
    for path in ["INBOX", "Archive"] {
        it.engine
            .sync(&mailbox(path), &mut cancel, now(), 200)
            .await
            .unwrap();
    }
    let page = it
        .store
        .threads(
            &Query {
                filter: Filter::All,
                sort: Sort {
                    property: Property::Date,
                    dir: SortDir::Desc,
                },
                page: PageReq {
                    after: None,
                    limit: 10,
                },
            },
            now(),
        )
        .unwrap();
    let message = page
        .items
        .iter()
        .flat_map(|t| it.store.thread(t.id).unwrap().messages)
        .find(|m| it.store.remotes_of(*m).unwrap().len() == 2)
        .expect("the message held in both mailboxes");
    server.lock().unwrap().commands.clear();

    queue(
        &it.store,
        RemoteIntent::SetFlags {
            messages: vec![message],
            read: None,
            star: Some(Star::Starred),
        },
    );
    let drained = it.engine.drain_outbox(&mut cancel, now()).await.unwrap();

    assert_eq!(drained.outbox_settled, 1, "{:?}", drained.needs_attention);
    let mut sent = changes(&server);
    sent.sort();
    assert_eq!(
        sent,
        vec![
            "SELECT \"Archive\"".to_owned(),
            "SELECT \"INBOX\"".to_owned(),
            "UID STORE 10 +FLAGS (\\Flagged)".to_owned(),
            "UID STORE 7 +FLAGS (\\Flagged)".to_owned(),
        ],
        "one command per mailbox, never 10 and 7 together"
    );
}
