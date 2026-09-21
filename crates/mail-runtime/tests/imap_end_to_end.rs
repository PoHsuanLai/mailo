//! IMAP over a real socket: engine, drive loop, transport, session, backend, assembly, store.
//!
//! `plan.md` phase 5 is done when "the same CLI works through the IMAP backend, and killing the
//! app mid-sync and restarting produces no duplicates". Proving that against Gmail needs an
//! OAuth client registration nobody can supply from a source tree. Proving it against *an* IMAP
//! server does not — and the parts most likely to be wrong are not Google-specific. This is the
//! first time the IMAP backend has spoken to anything but a transcript.
//!
//! The server authenticates with `LOGIN`, because that is what every IMAP server except Gmail
//! does, and until the commit that added this file the backend could not use it: it named
//! `AUTHENTICATE XOAUTH2` at eleven call sites of its own.

use chrono::{DateTime, TimeZone, Utc};
use mail_domain::*;
use mail_proto::backend::{Authenticate, ImapBackend};
use mail_proto::{ImapAuth, ImapCommand, ImapSession};
use mail_runtime::{AccountEngine, MapSecrets, Secrets};
use mail_store::{SqliteStore, Store};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::watch;

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));
const PASSWORD: &str = "s3cr3t";

fn now() -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000, 0).unwrap()
}

/// Two messages, with the awkwardness that matters on this protocol: a literal in the body, and
/// a subject that needs encoded-word decoding.
fn maildrop() -> Vec<(u32, String)> {
    vec![
        (
            101,
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
            102,
            "From: Bob <bob@example.test>\r\n\
             To: me@example.test\r\n\
             Subject: =?utf-8?B?ZMOpasOgIHZ1?=\r\n\
             Date: Tue, 14 Nov 2023 23:13:20 +0000\r\n\
             Message-ID: <second@example.test>\r\n\
             \r\n\
             a body containing a line that looks like a tag:\r\n\
             A1 OK not really\r\n"
                .to_owned(),
        ),
    ]
}

/// How the server should behave when a fetch arrives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fault {
    /// Answer everything.
    None,
    /// Drop the connection partway through the first body fetch, once.
    DropDuringFetch,
}

#[derive(Debug, Default)]
struct Seen {
    commands: Vec<String>,
}

type Shared = Arc<Mutex<Seen>>;

/// An IMAP server on a real socket. Only as much of RFC 3501 as the client uses, but the bytes
/// are genuine — including literals, which is where a parser that counts wrongly corrupts mail.
async fn serve(seen: Shared, fault: Fault) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let faulted = Arc::new(AtomicUsize::new(0));
    tokio::spawn(async move {
        while let Ok((sock, _)) = listener.accept().await {
            tokio::spawn(session(sock, seen.clone(), fault, faulted.clone()));
        }
    });
    port
}

async fn session(
    mut sock: tokio::net::TcpStream,
    seen: Shared,
    fault: Fault,
    faulted: Arc<AtomicUsize>,
) {
    if sock
        .write_all(b"* OK [CAPABILITY IMAP4rev1] server ready\r\n")
        .await
        .is_err()
    {
        return;
    }

    let drop = maildrop();
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

            let mut parts = line.splitn(2, ' ');
            let tag = parts.next().unwrap_or("*").to_owned();
            let rest = parts.next().unwrap_or("").to_owned();
            let upper = rest.to_uppercase();

            // Recorded without arguments for LOGIN: the argument is the password.
            seen.lock()
                .unwrap()
                .commands
                .push(if upper.starts_with("LOGIN") {
                    "LOGIN".to_owned()
                } else {
                    rest.clone()
                });

            let reply = if upper.starts_with("CAPABILITY") {
                format!("* CAPABILITY IMAP4rev1 UIDPLUS MOVE\r\n{tag} OK done\r\n")
            } else if upper.starts_with("LOGIN") {
                format!("{tag} OK logged in\r\n")
            } else if upper.starts_with("LIST") {
                format!(
                    "* LIST (\\HasNoChildren) \"/\" \"INBOX\"\r\n\
                     * LIST (\\HasNoChildren \\Sent) \"/\" \"Sent\"\r\n\
                     {tag} OK done\r\n"
                )
            } else if upper.starts_with("SELECT") || upper.starts_with("EXAMINE") {
                format!(
                    "* 2 EXISTS\r\n\
                     * OK [UIDVALIDITY 42] uids valid\r\n\
                     * OK [UIDNEXT 103] next\r\n\
                     {tag} OK [READ-ONLY] done\r\n"
                )
            } else if upper.starts_with("UID FETCH") {
                if upper.contains("BODY.PEEK[]") {
                    if fault == Fault::DropDuringFetch
                        && faulted.fetch_add(1, Ordering::SeqCst) == 0
                    {
                        // Half of one message, then the socket dies. This is the crash phase 5
                        // asks about, from the side the client cannot distinguish from a train
                        // going into a tunnel.
                        let _ = sock
                            .write_all(b"* 1 FETCH (UID 101 BODY[] {40}\r\nFrom: Ada Lovelace <ad")
                            .await;
                        return;
                    }
                    bodies(&drop, &upper, &tag)
                } else if upper.contains("BODY.PEEK[HEADER]") {
                    headers(&drop, &upper, &tag)
                } else {
                    envelopes(&drop, &tag)
                }
            } else if upper.starts_with("UID STORE") {
                format!("{tag} OK stored\r\n")
            } else if upper.starts_with("LOGOUT") {
                let _ = sock
                    .write_all(format!("* BYE\r\n{tag} OK done\r\n").as_bytes())
                    .await;
                return;
            } else if upper.starts_with("NOOP") {
                format!("{tag} OK done\r\n")
            } else {
                format!("{tag} BAD unknown command\r\n")
            };

            if sock.write_all(reply.as_bytes()).await.is_err() {
                return;
            }
        }
    }
}

/// `UID FETCH` of envelopes: enough for the client to learn what exists.
fn envelopes(drop: &[(u32, String)], tag: &str) -> String {
    let mut out = String::new();
    for (seq, (uid, raw)) in drop.iter().enumerate() {
        let subject = raw
            .lines()
            .find_map(|l| l.strip_prefix("Subject: "))
            .unwrap_or("");
        let from_email = if *uid == 101 { "ada" } else { "bob" };
        out.push_str(&format!(
            "* {} FETCH (UID {uid} FLAGS (\\Seen) INTERNALDATE \"14-Nov-2023 22:13:20 +0000\" \
             ENVELOPE (\"Tue, 14 Nov 2023 22:13:20 +0000\" \"{subject}\" \
             ((NIL NIL \"{from_email}\" \"example.test\")) NIL NIL \
             ((NIL NIL \"me\" \"example.test\")) NIL NIL NIL NIL))\r\n",
            seq + 1
        ));
    }
    out.push_str(&format!("{tag} OK done\r\n"));
    out
}

/// Which UIDs a `UID FETCH <set>` names. Only the forms this client emits.
fn wanted(upper: &str, drop: &[(u32, String)]) -> Vec<u32> {
    let set = upper
        .strip_prefix("UID FETCH ")
        .and_then(|rest| rest.split(' ').next())
        .unwrap_or("");
    if set.contains(':') {
        return drop.iter().map(|(uid, _)| *uid).collect();
    }
    set.split(',').filter_map(|n| n.parse().ok()).collect()
}

fn headers(drop: &[(u32, String)], upper: &str, tag: &str) -> String {
    let mut out = String::new();
    for uid in wanted(upper, drop) {
        let Some((_, raw)) = drop.iter().find(|(u, _)| *u == uid) else {
            continue;
        };
        let head = raw.split("\r\n\r\n").next().unwrap_or("").to_owned() + "\r\n\r\n";
        out.push_str(&format!(
            "* 1 FETCH (UID {uid} FLAGS (\\Seen) BODY[HEADER] {{{}}}\r\n{head})\r\n",
            head.len()
        ));
    }
    out.push_str(&format!("{tag} OK done\r\n"));
    out
}

fn bodies(drop: &[(u32, String)], upper: &str, tag: &str) -> String {
    let mut out = String::new();
    for uid in wanted(upper, drop) {
        let Some((_, raw)) = drop.iter().find(|(u, _)| *u == uid) else {
            continue;
        };
        // A literal, with a byte count the client must honour — including the body line that
        // begins with what looks like a tagged response.
        out.push_str(&format!(
            "* 1 FETCH (UID {uid} BODY[] {{{}}}\r\n{raw})\r\n",
            raw.len()
        ));
    }
    out.push_str(&format!("{tag} OK done\r\n"));
    out
}

fn plan(port: u16) -> AccountPlan {
    AccountPlan {
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
        folders: FolderRoles::default(),
        condstore: Condstore::Absent,
        move_ext: MoveExt::Absent,
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

/// An engine wired to the server on `port`, sharing `dir` so a restart sees the same database.
fn engine(port: u16, dir: tempfile::TempDir) -> Fixture {
    let store = Arc::new(SqliteStore::open(dir.path().join("mail.db"), dir.path()).unwrap());
    store
        .connection()
        .execute(
            "INSERT OR IGNORE INTO accounts (id, address, plan, created_at)
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
                // LOGIN, not XOAUTH2. The backend no longer decides this.
                all.push(ImapCommand::Login);
            }
            all.extend(commands);
            ImapSession::new(auth.clone(), all)
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

fn count(store: &SqliteStore) -> u64 {
    store.count(&Filter::All, now()).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_whole_sync_over_a_real_socket_lands_mail_in_the_store() {
    let seen: Shared = Arc::new(Mutex::new(Seen::default()));
    let port = serve(seen.clone(), Fault::None).await;
    let mut it = engine(port, tempfile::tempdir().unwrap());
    let (_tx, mut cancel) = watch::channel(false);

    let report = it
        .engine
        .sync(&inbox(), &mut cancel, now(), 200)
        .await
        .expect("a sync over IMAP");
    assert!(report.headers_fetched > 0, "{report:?}");

    let bodies = it
        .engine
        .fetch_bodies(&inbox(), &mut cancel, now(), 100)
        .await
        .expect("bodies");
    assert!(bodies.bodies_fetched > 0, "{bodies:?}");

    assert_eq!(count(&it.store), 2, "both messages should have landed");

    // It authenticated with LOGIN, which is the whole point of the factory owning the mechanism.
    let seen = seen.lock().unwrap();
    assert!(
        seen.commands.iter().any(|c| c == "LOGIN"),
        "never logged in: {:?}",
        seen.commands
    );
    assert!(
        !seen.commands.iter().any(|c| c.contains("XOAUTH2")),
        "a password account tried XOAUTH2: {:?}",
        seen.commands
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn headers_are_peeked_so_a_sync_does_not_mark_the_mailbox_read() {
    // BODY.PEEK[HEADER], never BODY[HEADER]. Getting this wrong marks every message in the
    // user's mailbox read, on every other device, as a side effect of listing them.
    let seen: Shared = Arc::new(Mutex::new(Seen::default()));
    let port = serve(seen.clone(), Fault::None).await;
    let mut it = engine(port, tempfile::tempdir().unwrap());
    let (_tx, mut cancel) = watch::channel(false);

    it.engine
        .sync(&inbox(), &mut cancel, now(), 200)
        .await
        .unwrap();
    it.engine
        .fetch_bodies(&inbox(), &mut cancel, now(), 100)
        .await
        .unwrap();

    let seen = seen.lock().unwrap();
    let fetches: Vec<&String> = seen
        .commands
        .iter()
        .filter(|c| c.to_uppercase().starts_with("UID FETCH"))
        .collect();
    assert!(!fetches.is_empty(), "nothing was fetched");
    for fetch in &fetches {
        let upper = fetch.to_uppercase();
        assert!(
            !upper.contains("BODY[") || upper.contains("BODY.PEEK["),
            "a non-peeking fetch would set \\Seen: {fetch}"
        );
    }
    // And the mailbox was opened read-only for the survey.
    assert!(
        seen.commands
            .iter()
            .any(|c| c.to_uppercase().starts_with("EXAMINE")),
        "the survey should EXAMINE, not SELECT: {:?}",
        seen.commands
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_literal_body_survives_a_line_that_looks_like_a_tagged_response() {
    // The second message's body contains "A1 OK not really". A parser that scans for a tagged
    // response instead of honouring the literal's byte count truncates the message there.
    let seen: Shared = Arc::new(Mutex::new(Seen::default()));
    let port = serve(seen, Fault::None).await;
    let mut it = engine(port, tempfile::tempdir().unwrap());
    let (_tx, mut cancel) = watch::channel(false);

    it.engine
        .sync(&inbox(), &mut cancel, now(), 200)
        .await
        .unwrap();
    it.engine
        .fetch_bodies(&inbox(), &mut cancel, now(), 100)
        .await
        .unwrap();

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
                    limit: 50,
                },
            },
            now(),
        )
        .unwrap();
    let mut found = false;
    for summary in page.items {
        let thread = it.store.thread(summary.id).unwrap();
        for id in thread.messages {
            let message = it.store.message(id).unwrap();
            if let Body::Present { raw, .. } = message.body {
                let bytes = it.store.blobs().get(&it.store.connection(), raw).unwrap();
                let text = String::from_utf8_lossy(&bytes);
                if text.contains("looks like a tag") {
                    assert!(
                        text.contains("A1 OK not really"),
                        "the body was truncated at the tag-shaped line:\n{text}"
                    );
                    found = true;
                }
            }
        }
    }
    assert!(found, "the message with the tag-shaped body never arrived");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn killing_the_app_mid_sync_and_restarting_produces_no_duplicates() {
    // `plan.md` phase 5's second clause, which needs no credentials to answer. The server drops
    // the connection in the middle of a body literal, exactly as a lost network does; the engine
    // is rebuilt against the same database, and the mailbox must contain each message once.
    let seen: Shared = Arc::new(Mutex::new(Seen::default()));
    let port = serve(seen, Fault::DropDuringFetch).await;
    let dir = tempfile::tempdir().unwrap();

    let after_crash = {
        let mut it = engine(port, dir);
        let (_tx, mut cancel) = watch::channel(false);
        it.engine
            .sync(&inbox(), &mut cancel, now(), 200)
            .await
            .unwrap();
        // This one dies partway through. A failure here is the expected outcome, not a problem.
        let _ = it
            .engine
            .fetch_bodies(&inbox(), &mut cancel, now(), 100)
            .await;
        let seen_now = count(&it.store);
        // Hand the directory back so the restart opens the same database.
        (seen_now, it._dir)
    };
    let (before, dir) = after_crash;
    assert!(before > 0, "the first pass stored nothing to duplicate");

    // Restart: a new engine, a new connection, the same database on disk.
    let mut it = engine(port, dir);
    let (_tx, mut cancel) = watch::channel(false);
    it.engine
        .sync(&inbox(), &mut cancel, now(), 200)
        .await
        .unwrap();
    it.engine
        .fetch_bodies(&inbox(), &mut cancel, now(), 100)
        .await
        .expect("the second pass completes");

    assert_eq!(
        count(&it.store),
        2,
        "restarting after a mid-sync death duplicated messages"
    );
}
