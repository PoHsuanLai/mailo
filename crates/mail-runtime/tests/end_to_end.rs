//! The whole stack over a real socket: engine, drive loop, transport, session, backend,
//! assembly, store.
//!
//! Every other test in this workspace stops at one seam or another — a transcript replaces the
//! socket, or a constructed `Message` replaces the parser. This one replaces nothing except the
//! server's identity: a real `TcpListener` speaking real POP3 to the real client, with real
//! RFC 5322 messages going in and a real SQLite database coming out.
//!
//! It is not a substitute for connecting to a server someone else operates. What it does rule
//! out is every failure that lives between `AccountEngine::sync` and the wire, which until now
//! nothing exercised together.

use chrono::{DateTime, TimeZone, Utc};
use mail_domain::*;
use mail_proto::backend::{Authenticate, Pop3Backend};
use mail_proto::{Pop3Command, Pop3Session};
use mail_runtime::{AccountEngine, MapSecrets, Secrets};
use mail_store::{SqliteStore, Store};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::sync::watch;

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));
const PASSWORD: &str = "s3cr3t";

fn now() -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000, 0).unwrap()
}

/// Three messages, deliberately awkward: one ordinary, one whose body contains a line starting
/// with a dot, and one with no `Message-ID` at all.
fn maildrop() -> Vec<(&'static str, String)> {
    vec![
        (
            "0000000166aaf64b",
            "From: Ada Lovelace <ada@example.test>\r\n\
             To: me@example.test\r\n\
             Subject: lunch on friday\r\n\
             Date: Tue, 14 Nov 2023 22:13:20 +0000\r\n\
             Message-ID: <first@example.test>\r\n\
             \r\n\
             Shall we say one o'clock?\r\n"
                .to_owned(),
        ),
        (
            "0000000266aaf64b",
            "From: Bob <bob@example.test>\r\n\
             Subject: dotted\r\n\
             Message-ID: <second@example.test>\r\n\
             \r\n\
             Here is a line:\r\n\
             .a line beginning with a dot\r\n\
             and the end.\r\n"
                .to_owned(),
        ),
        (
            "0000000366aaf64b",
            "From: anon@example.test\r\n\
             Subject: no identity\r\n\
             \r\n\
             a message with no Message-ID, which is common in the wild\r\n"
                .to_owned(),
        ),
    ]
}

/// A POP3 server, speaking the real protocol on a real socket.
///
/// Only as much of RFC 1939 as the client uses — but the bytes are genuine, including
/// dot-stuffing on the way out, which is where a body containing a leading dot gets corrupted
/// if either side gets it wrong.
async fn serve() -> u16 {
    serve_full().await.0
}

/// The server, plus a handle to how many of the three messages it still admits to holding.
///
/// Dropping one is how a message deleted in webmail looks from here.
async fn serve_full() -> (u16, Present) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let present: Present = Arc::new(std::sync::atomic::AtomicUsize::new(maildrop().len()));
    let showing = present.clone();
    tokio::spawn(async move {
        while let Ok((sock, _)) = listener.accept().await {
            tokio::spawn(session(sock, showing.clone()));
        }
    });
    (port, present)
}

/// How many messages remain in the maildrop. One per server, not a `static`: these tests run in
/// parallel in one binary.
type Present = Arc<std::sync::atomic::AtomicUsize>;

async fn session(sock: tokio::net::TcpStream, present: Present) {
    let (read, mut write) = sock.into_split();
    let mut lines = BufReader::new(read).lines();
    let mut drop = maildrop();
    drop.truncate(present.load(std::sync::atomic::Ordering::SeqCst));

    let _ = write.write_all(b"+OK POP3 server ready\r\n").await;

    while let Ok(Some(line)) = lines.next_line().await {
        let line = line.trim_end().to_owned();
        let mut parts = line.split_whitespace();
        let verb = parts.next().unwrap_or("").to_ascii_uppercase();
        let arg = parts.next().unwrap_or("");

        let reply: String = match verb.as_str() {
            "CAPA" => "+OK\r\nTOP\r\nUIDL\r\nPIPELINING\r\nUSER\r\nSASL PLAIN\r\n.\r\n".to_owned(),
            "AUTH" | "USER" | "PASS" => "+OK\r\n".to_owned(),
            "STAT" => {
                let bytes: usize = drop.iter().map(|(_, m)| m.len()).sum();
                format!("+OK {} {bytes}\r\n", drop.len())
            }
            "UIDL" => {
                let mut out = String::from("+OK\r\n");
                for (i, (uidl, _)) in drop.iter().enumerate() {
                    out.push_str(&format!("{} {uidl}\r\n", i + 1));
                }
                out.push_str(".\r\n");
                out
            }
            "LIST" => {
                let mut out = String::from("+OK\r\n");
                for (i, (_, message)) in drop.iter().enumerate() {
                    out.push_str(&format!("{} {}\r\n", i + 1, message.len()));
                }
                out.push_str(".\r\n");
                out
            }
            "TOP" | "RETR" => {
                let index: usize = arg.parse().unwrap_or(0);
                match drop.get(index.wrapping_sub(1)) {
                    Some((_, message)) => {
                        // TOP n 0 is headers only; RETR is everything. Dot-stuff on the way out,
                        // which is the server's job and the thing the client must undo.
                        let body = if verb == "TOP" {
                            message.split("\r\n\r\n").next().unwrap_or("").to_owned() + "\r\n"
                        } else {
                            message.clone()
                        };
                        let stuffed: String = body
                            .split("\r\n")
                            .map(|l| {
                                if l.starts_with('.') {
                                    format!(".{l}")
                                } else {
                                    l.to_owned()
                                }
                            })
                            .collect::<Vec<_>>()
                            .join("\r\n");
                        format!("+OK\r\n{stuffed}\r\n.\r\n")
                    }
                    None => "-ERR no such message\r\n".to_owned(),
                }
            }
            "QUIT" => {
                let _ = write.write_all(b"+OK bye\r\n").await;
                return;
            }
            _ => "-ERR unknown command\r\n".to_owned(),
        };
        if write.write_all(reply.as_bytes()).await.is_err() {
            return;
        }
    }
}

fn plan(port: u16) -> AccountPlan {
    AccountPlan {
        address: "me@example.test".to_owned(),
        incoming: Incoming::Pop3 {
            host: "127.0.0.1".to_owned(),
            port,
            // Plaintext because this is loopback to a server in this process; TLS is exercised
            // by the transport's own tests and would only be testing rustls here.
            tls: Tls::Plaintext,
            leave: LeaveOnServer::Keep,
        },
        outgoing: Outgoing::Smtp {
            host: "127.0.0.1".to_owned(),
            port: 0,
            tls: Tls::Plaintext,
        },
        auth: AuthPlan::Password {
            username: Username::LocalPart,
            sasl: vec![SaslMech::Plain],
        },
        identities: Vec::new(),
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
        // Measured: this server advertises both, and TOP is what keeps the pass non-destructive.
        top: Supported::Yes,
        pipelining: Supported::Yes,
        connections: ConnectionBudget { max: 1 },
        observed_at: now(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_whole_sync_over_a_real_socket_lands_mail_in_the_store() {
    let port = serve().await;

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

    let backend = Pop3Backend::new(
        ACCOUNT,
        caps(),
        Box::new(|auth, commands| {
            let mut all = Vec::new();
            if auth == Authenticate::First {
                all.push(Pop3Command::AuthPlain);
            }
            all.extend(commands);
            Pop3Session::new("me", PASSWORD, all)
        }),
    );

    let mut engine = AccountEngine::new(
        ACCOUNT,
        plan(port),
        backend,
        store.clone(),
        Arc::new(secrets),
    );
    let mailbox = MailboxRef {
        account: ACCOUNT,
        path: "INBOX".to_owned(),
    };
    let (_tx, mut cancel) = watch::channel(false);

    // --- the header pass ----------------------------------------------------------------
    let headers = engine
        .sync(&mailbox, &mut cancel, now(), 50)
        .await
        .expect("a sync pass should succeed");
    assert_eq!(headers.headers_fetched, 3, "{:?}", headers.needs_attention);

    let query = |filter: Filter| Query {
        filter,
        sort: Sort {
            property: Property::Date,
            dir: SortDir::Desc,
        },
        page: PageReq {
            after: None,
            limit: 50,
        },
    };

    let listed = store.threads(&query(Filter::All), now()).unwrap();
    assert_eq!(listed.items.len(), 3, "three messages should be listable");
    assert!(
        listed.items.iter().any(|t| t.subject == "lunch on friday"),
        "{:?}",
        listed.items.iter().map(|t| &t.subject).collect::<Vec<_>>()
    );

    // Listable from headers alone, with no body — the reason Body::Absent exists.
    assert_eq!(
        store.unfetched(ACCOUNT, 10).unwrap().len(),
        3,
        "all three still need bodies"
    );

    // --- the body pass ------------------------------------------------------------------
    let bodies = engine
        .fetch_bodies(&mailbox, &mut cancel, now(), 50)
        .await
        .expect("fetching bodies should succeed");
    assert_eq!(bodies.bodies_fetched, 3, "{:?}", bodies.needs_attention);
    assert_eq!(
        store.unfetched(ACCOUNT, 10).unwrap().len(),
        0,
        "nothing should still be missing a body"
    );

    // --- the body actually survived the wire --------------------------------------------
    let found = store
        .threads(
            &query(Filter::Text(TextMatch::Contains("clock".into()))),
            now(),
        )
        .unwrap();
    assert_eq!(
        found.items.len(),
        1,
        "body text must reach the search index"
    );

    // Dot-stuffing: the server sent "..a line...", and the client must have removed exactly one
    // dot. Getting this wrong corrupts any message whose body has a line starting with a dot.
    let dotted = store
        .threads(
            &query(Filter::Subject(TextMatch::Contains("dotted".into()))),
            now(),
        )
        .unwrap();
    let thread = store.thread(dotted.items[0].id).unwrap();
    let message = store.message(thread.messages[0]).unwrap();
    let text = message.body.text().expect("the body was fetched");
    assert!(
        text.contains("\n.a line beginning with a dot"),
        "dot-stuffing was not undone correctly: {text:?}"
    );
    assert!(
        !text.contains("..a line"),
        "one dot too many survived: {text:?}"
    );

    // --- a second sync must not duplicate anything --------------------------------------
    engine
        .sync(&mailbox, &mut cancel, now(), 50)
        .await
        .expect("a second pass should succeed");
    let again = store.threads(&query(Filter::All), now()).unwrap();
    assert_eq!(
        again.items.len(),
        3,
        "re-syncing must re-map, not re-create: {:?}",
        again.items.iter().map(|t| &t.subject).collect::<Vec<_>>()
    );
    let messages: i64 = store
        .connection()
        .query_row("SELECT count(*) FROM messages", [], |r| r.get(0))
        .unwrap();
    assert_eq!(messages, 3);
}

/// What a POP3 account does on its *second* pass, which nothing here had ever exercised.
///
/// One sync hides a whole class of defect: everything idempotent is trivially idempotent when
/// it runs once.
///
/// These do **not** reproduce F52, and I checked rather than assumed — they pass with migration
/// 0002 removed. The duplicate `remote_map` rows needed a repeated *ingest of the same message*,
/// and a second POP3 pass does not do that: `unfetched` is empty once the bodies are stored, so
/// no header ingest runs and nothing rewrites the mapping. F52 reached this protocol by a route
/// these tests do not take, and `mail-store`'s `remote_map_identity` tests cover it directly.
mod repeated_passes {
    use super::*;

    struct Fixture {
        store: Arc<SqliteStore>,
        engine: AccountEngine<Pop3Backend>,
        _dir: tempfile::TempDir,
    }

    fn fixture(port: u16) -> Fixture {
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

        let backend = Pop3Backend::new(
            ACCOUNT,
            caps(),
            Box::new(|auth, commands| {
                let mut all = Vec::new();
                if auth == Authenticate::First {
                    all.push(Pop3Command::AuthPlain);
                }
                all.extend(commands);
                Pop3Session::new("me", PASSWORD, all)
            }),
        );
        let engine = AccountEngine::new(
            ACCOUNT,
            plan(port),
            backend,
            store.clone(),
            Arc::new(secrets),
        );
        Fixture {
            store,
            engine,
            _dir: dir,
        }
    }

    fn inbox() -> MailboxRef {
        MailboxRef {
            account: ACCOUNT,
            path: "INBOX".to_owned(),
        }
    }

    fn messages(store: &SqliteStore) -> u64 {
        store.count(&Filter::All, now()).unwrap()
    }

    fn remote_rows(store: &SqliteStore) -> i64 {
        store
            .connection()
            .query_row("SELECT count(*) FROM remote_map", [], |r| r.get(0))
            .unwrap()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn syncing_three_times_stores_each_message_once() {
        let port = serve().await;
        let mut it = fixture(port);
        let (_tx, mut cancel) = watch::channel(false);

        for _ in 0..3 {
            it.engine
                .sync(&inbox(), &mut cancel, now(), 50)
                .await
                .unwrap();
            it.engine
                .fetch_bodies(&inbox(), &mut cancel, now(), 50)
                .await
                .unwrap();
        }

        assert_eq!(messages(&it.store), 3, "a repeated sync duplicated mail");
        assert_eq!(
            remote_rows(&it.store),
            3,
            "one message in one maildrop is one row"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_header_stored_once_is_not_fetched_again() {
        // The survey is everything on the server, which answers "what exists" and not "what
        // should I fetch". Using it directly re-downloaded every header in the maildrop on every
        // pass — on POP3 that is one `TOP` per message per poll, against a campus server, for
        // mail already on disk. The earlier repeat-pass test asserted nothing was *duplicated*,
        // which stayed true the whole time.
        let port = serve().await;
        let mut it = fixture(port);
        let (_tx, mut cancel) = watch::channel(false);

        let first = it
            .engine
            .sync(&inbox(), &mut cancel, now(), 50)
            .await
            .unwrap();
        assert_eq!(first.headers_fetched, 3, "the first pass fetches all three");

        let second = it
            .engine
            .sync(&inbox(), &mut cancel, now(), 50)
            .await
            .unwrap();
        assert_eq!(
            second.headers_fetched, 0,
            "a second pass re-fetched headers already stored"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_body_fetched_once_is_not_fetched_again() {
        // `unfetched` drives the body pass. If a stored body did not clear it, every sync would
        // redownload the whole maildrop — which on POP3 also means RETR, which sets \Seen.
        let port = serve().await;
        let mut it = fixture(port);
        let (_tx, mut cancel) = watch::channel(false);

        it.engine
            .sync(&inbox(), &mut cancel, now(), 50)
            .await
            .unwrap();
        let first = it
            .engine
            .fetch_bodies(&inbox(), &mut cancel, now(), 50)
            .await
            .unwrap();
        assert!(first.bodies_fetched > 0, "{first:?}");

        let second = it
            .engine
            .fetch_bodies(&inbox(), &mut cancel, now(), 50)
            .await
            .unwrap();
        assert_eq!(
            second.bodies_fetched, 0,
            "bodies were downloaded a second time"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_message_deleted_in_webmail_disappears_here_too() {
        // POP3's expunge diff is UIDL: the complete list of what the server still holds, and
        // anything in remote_map missing from it was deleted elsewhere. The runtime does that
        // diff, so this exercises the same code the IMAP sweep uses.
        let (port, present) = serve_full().await;
        let mut it = fixture(port);
        let (_tx, mut cancel) = watch::channel(false);

        it.engine
            .sync(&inbox(), &mut cancel, now(), 50)
            .await
            .unwrap();
        assert_eq!(messages(&it.store), 3);

        // Two of the three are deleted from the maildrop.
        present.store(1, std::sync::atomic::Ordering::SeqCst);

        // The expunge interval is the longest of the three, so a sweep now would not be due.
        let later = now() + chrono::TimeDelta::try_hours(2).unwrap();
        it.engine.sweep(&inbox(), &mut cancel, later).await.unwrap();

        assert_eq!(
            messages(&it.store),
            1,
            "messages deleted on the server are still here"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_sweep_with_the_maildrop_intact_deletes_nothing() {
        // The dangerous direction: `gone` drives deletion, so a UIDL listing this code fails to
        // read must never look like an empty maildrop.
        let (port, _present) = serve_full().await;
        let mut it = fixture(port);
        let (_tx, mut cancel) = watch::channel(false);

        it.engine
            .sync(&inbox(), &mut cancel, now(), 50)
            .await
            .unwrap();
        let before = messages(&it.store);

        let later = now() + chrono::TimeDelta::try_hours(2).unwrap();
        it.engine.sweep(&inbox(), &mut cancel, later).await.unwrap();

        assert_eq!(messages(&it.store), before, "a sweep deleted live mail");
    }
}
