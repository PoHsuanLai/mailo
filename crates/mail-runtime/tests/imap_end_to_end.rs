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

/// The mailbox's `UIDVALIDITY`, which a test can change under the client.
///
/// One per server rather than a `static`: these tests run in parallel in one binary, and a
/// shared one would have each test changing the mailbox out from under the others.
type Validity = Arc<AtomicUsize>;

/// How many messages the server still admits to having, from the front of the maildrop.
///
/// Dropping one is how a message deleted on another device looks from here.
type Present = Arc<AtomicUsize>;

/// Flags the server holds per UID, so a `UID STORE` can be observed on the next fetch.
///
/// Without this the server answered `OK stored` and then reported the original flags for ever,
/// which would let a round-trip test pass while nothing round-tripped.
type Flags = Arc<Mutex<std::collections::BTreeMap<u32, Vec<String>>>>;

/// Messages the server was asked to store with `APPEND`, as it received them.
type Appended = Arc<Mutex<Vec<String>>>;

/// Everything a test can change about the server while the client is talking to it.
///
/// One handle rather than four arguments: each of these grew in separately as a test needed it,
/// and by the fourth the session signature said nothing about what any of them were for.
#[derive(Clone)]
struct ServerState {
    validity: Validity,
    present: Present,
    flags: Flags,
    appended: Appended,
}

#[derive(Debug, Default)]
struct Seen {
    commands: Vec<String>,
}

type Shared = Arc<Mutex<Seen>>;

/// An IMAP server on a real socket. Only as much of RFC 3501 as the client uses, but the bytes
/// are genuine — including literals, which is where a parser that counts wrongly corrupts mail.
async fn serve(seen: Shared, fault: Fault) -> (u16, Validity) {
    let (port, validity, _present) = serve_full(seen, fault).await;
    (port, validity)
}

async fn serve_full(seen: Shared, fault: Fault) -> (u16, Validity, Present) {
    let (port, validity, present, _flags) = serve_flags(seen, fault).await;
    (port, validity, present)
}

async fn serve_flags(seen: Shared, fault: Fault) -> (u16, Validity, Present, Flags) {
    let (port, v, p, f, _a) = serve_appending(seen, fault).await;
    (port, v, p, f)
}

async fn serve_appending(seen: Shared, fault: Fault) -> (u16, Validity, Present, Flags, Appended) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let faulted = Arc::new(AtomicUsize::new(0));
    let validity: Validity = Arc::new(AtomicUsize::new(42));
    let present: Present = Arc::new(AtomicUsize::new(maildrop().len()));
    let flags: Flags = Arc::new(Mutex::new(
        maildrop()
            .iter()
            .map(|(uid, _)| (*uid, vec!["\\Seen".to_owned()]))
            .collect(),
    ));
    let appended: Appended = Arc::new(Mutex::new(Vec::new()));
    let state = ServerState {
        validity: validity.clone(),
        present: present.clone(),
        flags: flags.clone(),
        appended: appended.clone(),
    };
    tokio::spawn(async move {
        while let Ok((sock, _)) = listener.accept().await {
            tokio::spawn(session(
                sock,
                seen.clone(),
                fault,
                faulted.clone(),
                state.clone(),
            ));
        }
    });
    (port, validity, present, flags, appended)
}

async fn session(
    mut sock: tokio::net::TcpStream,
    seen: Shared,
    fault: Fault,
    faulted: Arc<AtomicUsize>,
    state: ServerState,
) {
    let ServerState {
        validity,
        present,
        flags,
        appended,
    } = state;
    if sock
        .write_all(b"* OK [CAPABILITY IMAP4rev1] server ready\r\n")
        .await
        .is_err()
    {
        return;
    }

    let mut drop = maildrop();
    drop.truncate(present.load(Ordering::SeqCst));
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
                     * LIST (\\HasNoChildren \\Drafts) \"/\" \"Drafts\"\r\n\
                     {tag} OK done\r\n"
                )
            } else if upper.starts_with("LSUB") {
                format!("* LSUB () \"/\" \"INBOX\"\r\n{tag} OK done\r\n")
            } else if upper.starts_with("ENABLE") {
                format!("* ENABLED QRESYNC\r\n{tag} OK enabled\r\n")
            } else if upper.starts_with("SELECT") || upper.starts_with("EXAMINE") {
                let validity = validity.load(Ordering::SeqCst);
                // Asked to resynchronise, a QRESYNC server says what went: here, whatever has
                // been taken out of the maildrop, and then a range far above any UID it ever
                // issued, because servers do send those and only held UIDs may be acted on.
                let vanished = if upper.contains("(QRESYNC") {
                    let gone: Vec<String> = maildrop()
                        .iter()
                        .map(|(uid, _)| *uid)
                        .filter(|uid| !drop.iter().any(|(held, _)| held == uid))
                        .map(|uid| uid.to_string())
                        .chain(["200:4294967295".to_owned()])
                        .collect();
                    format!("* VANISHED (EARLIER) {}\r\n", gone.join(","))
                } else {
                    String::new()
                };
                format!(
                    "* 2 EXISTS\r\n\
                     * OK [UIDVALIDITY {validity}] uids valid\r\n\
                     * OK [UIDNEXT 103] next\r\n\
                     * OK [HIGHESTMODSEQ 7788] modseq\r\n\
                     {vanished}\
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
                    envelopes(&drop, &tag, &flags)
                }
            } else if upper.starts_with("UID MOVE") || upper.starts_with("UID COPY") {
                format!("{tag} OK done\r\n")
            } else if upper.starts_with("UID SEARCH") {
                let uids: Vec<String> = drop.iter().map(|(uid, _)| uid.to_string()).collect();
                format!("* SEARCH {}\r\n{tag} OK done\r\n", uids.join(" "))
            } else if upper.starts_with("UID STORE") {
                apply_store(&rest, &flags);
                format!("{tag} OK stored\r\n")
            } else if upper.starts_with("LOGOUT") {
                let _ = sock
                    .write_all(format!("* BYE\r\n{tag} OK done\r\n").as_bytes())
                    .await;
                return;
            } else if upper.starts_with("APPEND") {
                // `{n}` then the bytes. The count is authoritative: read exactly that many,
                // which is what a client desynchronising the connection gets wrong.
                let want: usize = rest
                    .rsplit_once('{')
                    .and_then(|(_, n)| n.trim_end_matches('}').parse().ok())
                    .unwrap_or(0);
                let _ = sock.write_all(b"+ ready for literal\r\n").await;
                while buf.len() < want + 2 {
                    let mut chunk = [0u8; 4096];
                    match sock.read(&mut chunk).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => buf.extend_from_slice(&chunk[..n]),
                    }
                }
                let body = buf.drain(..want).collect::<Vec<u8>>();
                appended
                    .lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&body).to_string());
                // The trailing CRLF after the literal.
                if buf.starts_with(b"\r\n") {
                    buf.drain(..2);
                }
                format!("{tag} OK [APPENDUID 42 103] done\r\n")
            } else if upper.starts_with("IDLE") {
                // `+ idling`, then something to report. A real server would park here until it
                // had news; this one has news immediately, which is the case worth testing —
                // the parking itself is the client's cancellation problem, covered separately.
                //
                // The tag is remembered, because `DONE` arrives with no tag of its own and must
                // be answered with the IDLE's. Replying with anything else leaves the client
                // waiting for a completion that never comes, which is what this fixture did on
                // its first run.
                idle_tag = Some(tag.clone());
                let _ = sock.write_all(b"+ idling\r\n").await;
                let _ = sock.write_all(b"* 3 EXISTS\r\n").await;
                continue;
            } else if tag.eq_ignore_ascii_case("DONE") {
                match idle_tag.take() {
                    Some(idle) => format!("{idle} OK idle done\r\n"),
                    None => "* BAD DONE without IDLE\r\n".to_owned(),
                }
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
/// Apply `UID STORE <set> (+|-)FLAGS (\Seen \Flagged)` to what the server holds.
fn apply_store(command: &str, flags: &Flags) {
    let upper = command.to_uppercase();
    let adding = !upper.contains("-FLAGS");
    let Some(open) = command.find('(') else {
        return;
    };
    let Some(close) = command.rfind(')') else {
        return;
    };
    let named: Vec<String> = command[open + 1..close]
        .split_whitespace()
        .map(str::to_owned)
        .collect();

    let uids: Vec<u32> = upper
        .strip_prefix("UID STORE ")
        .and_then(|rest| rest.split(' ').next())
        .map(|set| set.split(',').filter_map(|n| n.parse().ok()).collect())
        .unwrap_or_default();

    let mut held = flags.lock().unwrap();
    for uid in uids {
        let entry = held.entry(uid).or_default();
        for flag in &named {
            let present = entry.iter().any(|f| f.eq_ignore_ascii_case(flag));
            match (adding, present) {
                (true, false) => entry.push(flag.clone()),
                (false, true) => entry.retain(|f| !f.eq_ignore_ascii_case(flag)),
                _ => {}
            }
        }
    }
}

fn envelopes(drop: &[(u32, String)], tag: &str, flags: &Flags) -> String {
    let mut out = String::new();
    for (seq, (uid, raw)) in drop.iter().enumerate() {
        let subject = raw
            .lines()
            .find_map(|l| l.strip_prefix("Subject: "))
            .unwrap_or("");
        let from_email = if *uid == 101 { "ada" } else { "bob" };
        let held = flags
            .lock()
            .unwrap()
            .get(uid)
            .cloned()
            .unwrap_or_default()
            .join(" ");
        out.push_str(&format!(
            "* {} FETCH (UID {uid} FLAGS ({held}) INTERNALDATE \"14-Nov-2023 22:13:20 +0000\" \
             ENVELOPE (\"Tue, 14 Nov 2023 22:13:20 +0000\" \"{subject}\" \
             ((NIL NIL \"{from_email}\" \"example.test\")) NIL NIL \
             ((NIL NIL \"me\" \"example.test\")) NIL NIL NIL NIL))\r\n",
            seq + 1
        ));
    }
    out.push_str(&format!("{tag} OK done\r\n"));
    out
}

/// Which UIDs a `UID FETCH <set>` names, in the order a server answers them: mailbox order.
///
/// Not the order the set was written in. RFC 3501 does not promise request order, and Gmail
/// answers `UID FETCH 9,3` with 3 first. This fake used to answer in request order, which is
/// how a client that paired replies with requests by position passed every test here and then
/// scrambled a real mailbox the day it started asking newest-first.
fn wanted(upper: &str, drop: &[(u32, String)]) -> Vec<u32> {
    let set = upper
        .strip_prefix("UID FETCH ")
        .and_then(|rest| rest.split(' ').next())
        .unwrap_or("");
    if set.contains(':') {
        return drop.iter().map(|(uid, _)| *uid).collect();
    }
    let mut uids: Vec<u32> = set.split(',').filter_map(|n| n.parse().ok()).collect();
    uids.sort_unstable();
    uids
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
    caps_with(Condstore::Absent)
}

fn caps_with(condstore: Condstore) -> AccountCaps {
    AccountCaps {
        labels: ServerLabels::LocalOnly,
        threads: ServerThreads::Jwz,
        watch: WatchMode::Poll {
            every: std::time::Duration::from_secs(300),
        },
        archive: ArchiveMeans::MoveToFolder("Archive".to_owned()),
        folders: FolderRoles::default(),
        condstore,
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
    engine_with(port, dir, caps())
}

fn engine_with(port: u16, dir: tempfile::TempDir, caps: AccountCaps) -> Fixture {
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
        caps,
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
    let (port, _validity) = serve(seen.clone(), Fault::None).await;
    let mut it = engine(port, tempfile::tempdir().unwrap());
    let (_tx, mut cancel) = watch::channel(false);

    let report = it
        .engine
        .sync(&inbox(), &mut cancel, now(), 200)
        .await
        .expect("a sync over IMAP");
    // The fixture holds exactly two, and `> 0` is true of a pass that fetched the same header
    // eight times as well as of one that did its job — which is the shape F95 hid behind.
    assert_eq!(report.headers_fetched, 2, "{report:?}");

    let bodies = it
        .engine
        .fetch_bodies(&inbox(), &mut cancel, now(), 100)
        .await
        .expect("bodies");
    assert_eq!(bodies.bodies_fetched, 2, "{bodies:?}");

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
    let (port, _validity) = serve(seen.clone(), Fault::None).await;
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
    let (port, _validity) = serve(seen, Fault::None).await;
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
                    // Exact bytes, not `contains`. This asserted only that the tag-shaped line
                    // survived, and it did — while the `)` closing the FETCH response was being
                    // appended to every message fetched over IMAP. A substring assertion cannot
                    // see something *added*, which is how that shipped.
                    let expected = maildrop()
                        .into_iter()
                        .find(|(uid, _)| *uid == 102)
                        .map(|(_, raw)| raw)
                        .expect("the fixture has uid 102");
                    assert_eq!(
                        text, expected,
                        "the stored message is not byte-for-byte what the server sent"
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
    let (port, _validity) = serve(seen, Fault::DropDuringFetch).await;
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_condstore_server_gets_a_changedsince_sweep_after_the_first_pass() {
    // The whole chain, which had three links and was missing two: the SELECT response carries
    // HIGHESTMODSEQ, the ingest records it in the cursor, `sync_state` keeps it, and the next
    // flags sweep reads it back and asks only for what changed. Until this test the cursor was
    // written by every sync and read by nothing, so the sweep refetched every flag for ever.
    let seen: Shared = Arc::new(Mutex::new(Seen::default()));
    let (port, _validity) = serve(seen.clone(), Fault::None).await;
    let mut it = engine_with(
        port,
        tempfile::tempdir().unwrap(),
        caps_with(Condstore::Supported),
    );
    let (_tx, mut cancel) = watch::channel(false);

    // First pass: nothing is known, so this must be a full sweep.
    it.engine
        .sync(&inbox(), &mut cancel, now(), 200)
        .await
        .unwrap();

    // The cursor should now carry what the server said.
    match it.store.cursor(&inbox()).unwrap() {
        Some(SyncCursor::Imap {
            modseq,
            uidvalidity,
            ..
        }) => {
            assert_eq!(modseq, Some(7788), "HIGHESTMODSEQ was not recorded");
            assert_eq!(uidvalidity, 42);
        }
        other => panic!("no usable cursor after a sync: {other:?}"),
    }

    seen.lock().unwrap().commands.clear();
    it.engine
        .sweep(&inbox(), &mut cancel, now())
        .await
        .expect("a sweep");

    let seen = seen.lock().unwrap();
    assert!(
        seen.commands
            .iter()
            .any(|c| c.to_uppercase().contains("CHANGEDSINCE 7788")),
        "the sweep refetched every flag instead of asking what changed: {:?}",
        seen.commands
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_server_without_condstore_is_never_sent_changedsince() {
    // CHANGEDSINCE to a server that does not advertise CONDSTORE is a protocol error, not a
    // graceful degradation — so the capability gate has to hold even though the cursor is there.
    let seen: Shared = Arc::new(Mutex::new(Seen::default()));
    let (port, _validity) = serve(seen.clone(), Fault::None).await;
    let mut it = engine(port, tempfile::tempdir().unwrap());
    let (_tx, mut cancel) = watch::channel(false);

    it.engine
        .sync(&inbox(), &mut cancel, now(), 200)
        .await
        .unwrap();
    // The modseq really is on record; only the capability says not to use it.
    assert!(matches!(
        it.store.cursor(&inbox()).unwrap(),
        Some(SyncCursor::Imap {
            modseq: Some(7788),
            ..
        })
    ));

    seen.lock().unwrap().commands.clear();
    it.engine
        .sweep(&inbox(), &mut cancel, now())
        .await
        .expect("a sweep");

    let seen = seen.lock().unwrap();
    assert!(
        !seen
            .commands
            .iter()
            .any(|c| c.to_uppercase().contains("CHANGEDSINCE")),
        "CHANGEDSINCE sent to a server that never offered CONDSTORE: {:?}",
        seen.commands
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_mailbox_recreated_on_the_server_invalidates_the_uids_we_stored() {
    // `plan.md` phase 5 names "UIDVALIDITY reset handling". The store has implemented it since
    // it was written — `UidValidity::Reset` drops every `remote_map` row for the mailbox — and
    // the backend reported `Same` unconditionally, so it could never fire.
    //
    // A server that restores a mailbox from backup, migrates it, or has a folder deleted and
    // remade with the same name MUST change UIDVALIDITY. Every UID we hold then names a
    // different message, or none. Keeping them is not a stale cache: it is marking the wrong
    // mail read and attaching bodies to the wrong headers.
    let seen: Shared = Arc::new(Mutex::new(Seen::default()));
    let (port, validity) = serve(seen, Fault::None).await;
    let dir = tempfile::tempdir().unwrap();
    let (_tx, mut cancel) = watch::channel(false);

    let dir = {
        let mut it = engine(port, dir);
        it.engine
            .sync(&inbox(), &mut cancel, now(), 200)
            .await
            .unwrap();
        assert_eq!(remote_rows(&it.store), 2, "the first sync mapped both UIDs");
        it._dir
    };

    // The mailbox is recreated on the server.
    validity.store(99, Ordering::SeqCst);

    let mut it = engine(port, dir);
    it.engine
        .sync(&inbox(), &mut cancel, now(), 200)
        .await
        .unwrap();

    match it.store.cursor(&inbox()).unwrap() {
        Some(SyncCursor::Imap { uidvalidity, .. }) => assert_eq!(uidvalidity, 99),
        other => panic!("{other:?}"),
    }
    // The old rows are gone and the mailbox has been mapped afresh, rather than two generations
    // of UID sitting on top of each other.
    assert_eq!(
        remote_rows(&it.store),
        2,
        "stale remote_map rows survived a UIDVALIDITY change"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unchanged_uidvalidity_does_not_throw_the_mailbox_away() {
    // The other half. Resetting on every sync would refetch the whole mailbox for ever, which
    // is the expensive way to be wrong and just as silent.
    let seen: Shared = Arc::new(Mutex::new(Seen::default()));
    let (port, _validity) = serve(seen, Fault::None).await;
    let dir = tempfile::tempdir().unwrap();
    let (_tx, mut cancel) = watch::channel(false);

    let dir = {
        let mut it = engine(port, dir);
        it.engine
            .sync(&inbox(), &mut cancel, now(), 200)
            .await
            .unwrap();
        it.engine
            .fetch_bodies(&inbox(), &mut cancel, now(), 100)
            .await
            .unwrap();
        it._dir
    };

    let mut it = engine(port, dir);
    it.engine
        .sync(&inbox(), &mut cancel, now(), 200)
        .await
        .unwrap();
    assert_eq!(count(&it.store), 2, "a second sync duplicated the mailbox");
    assert_eq!(remote_rows(&it.store), 2);
}

/// How many `remote_map` rows exist, which is what a reset clears.
fn remote_rows(store: &SqliteStore) -> i64 {
    store
        .connection()
        .query_row("SELECT count(*) FROM remote_map", [], |r| r.get(0))
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_message_deleted_elsewhere_disappears_here_too() {
    // The third of the three sync intervals. Gmail has no QRESYNC and IDLE reports new mail
    // only, so the full listing diffed against `remote_map` is the only way to learn that
    // something was deleted on another device — RFC 7162 says as much outright.
    //
    // `Job::Listing` used to share the envelope arm, which parses FETCH responses. `UID SEARCH`
    // answers `* SEARCH 101 102`, so nothing parsed, `gone` was always empty, and every sweep
    // concluded that nothing had disappeared.
    let seen: Shared = Arc::new(Mutex::new(Seen::default()));
    let (port, _validity, present) = serve_full(seen, Fault::None).await;
    let mut it = engine(port, tempfile::tempdir().unwrap());
    let (_tx, mut cancel) = watch::channel(false);

    it.engine
        .sync(&inbox(), &mut cancel, now(), 200)
        .await
        .unwrap();
    assert_eq!(count(&it.store), 2, "both messages arrived");

    // One is deleted on another device.
    present.store(1, Ordering::SeqCst);

    // The expunge interval is the longest of the three, so a sweep at `now()` would not be due.
    let later = now() + chrono::TimeDelta::try_hours(2).unwrap();
    it.engine.sweep(&inbox(), &mut cancel, later).await.unwrap();

    assert_eq!(
        count(&it.store),
        1,
        "a message deleted on the server is still here"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_sweep_that_finds_everything_still_there_deletes_nothing() {
    // The dangerous direction. `gone` drives deletion, so a listing this code fails to parse
    // must never read as "the mailbox is empty".
    let seen: Shared = Arc::new(Mutex::new(Seen::default()));
    let (port, _validity, _present) = serve_full(seen, Fault::None).await;
    let mut it = engine(port, tempfile::tempdir().unwrap());
    let (_tx, mut cancel) = watch::channel(false);

    it.engine
        .sync(&inbox(), &mut cancel, now(), 200)
        .await
        .unwrap();
    let before = count(&it.store);

    let later = now() + chrono::TimeDelta::try_hours(2).unwrap();
    it.engine.sweep(&inbox(), &mut cancel, later).await.unwrap();

    assert_eq!(count(&it.store), before, "a sweep deleted live mail");
}

/// With QRESYNC the server names what was expunged: one `SELECT`, and no listing to diff.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_qresync_server_says_what_was_deleted_elsewhere() {
    let seen: Shared = Arc::new(Mutex::new(Seen::default()));
    let (port, _validity, present) = serve_full(seen.clone(), Fault::None).await;
    let mut it = engine_with(
        port,
        tempfile::tempdir().unwrap(),
        caps_with(Condstore::Qresync),
    );
    let (_tx, mut cancel) = watch::channel(false);

    it.engine
        .sync(&inbox(), &mut cancel, now(), 200)
        .await
        .unwrap();
    assert_eq!(count(&it.store), 2);

    present.store(1, Ordering::SeqCst);
    seen.lock().unwrap().commands.clear();
    let later = now() + chrono::TimeDelta::try_hours(2).unwrap();
    it.engine.sweep(&inbox(), &mut cancel, later).await.unwrap();

    assert_eq!(
        count(&it.store),
        1,
        "the message the server named is still here"
    );
    let commands = seen.lock().unwrap().commands.clone();
    assert!(
        commands.iter().any(|c| c == "ENABLE QRESYNC"),
        "{commands:?}"
    );
    assert!(
        commands
            .iter()
            .any(|c| c == "EXAMINE \"INBOX\" (QRESYNC (42 7788))"),
        "{commands:?}"
    );
    assert!(
        !commands.iter().any(|c| c.starts_with("UID SEARCH")),
        "a QRESYNC server was still asked to list every UID: {commands:?}"
    );
}

/// The dangerous direction again, for QRESYNC: a resync that lists nothing is not an empty
/// mailbox, and a vanished range covering UIDs never held removes nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_qresync_sweep_that_names_nothing_held_deletes_nothing() {
    let seen: Shared = Arc::new(Mutex::new(Seen::default()));
    let (port, _validity, _present) = serve_full(seen, Fault::None).await;
    let mut it = engine_with(
        port,
        tempfile::tempdir().unwrap(),
        caps_with(Condstore::Qresync),
    );
    let (_tx, mut cancel) = watch::channel(false);

    it.engine
        .sync(&inbox(), &mut cancel, now(), 200)
        .await
        .unwrap();
    let later = now() + chrono::TimeDelta::try_hours(2).unwrap();
    it.engine.sweep(&inbox(), &mut cancel, later).await.unwrap();

    assert_eq!(count(&it.store), 2, "a resync deleted live mail");
}

/// A user's action, all the way to the server and back.
///
/// The optimistic-apply and reconciliation rules are tested in `mail-store` against synthetic
/// `Ingest`s. This drives the same rules through the whole chain — `Op::apply`, the outbox,
/// `resolve_intent`, the backend, a real socket, `Settle`, and the next sync's reconciliation —
/// which is the only way to find a seam that lies between two correct halves.
mod round_trip {
    use super::*;

    fn thread_of(store: &SqliteStore) -> (ThreadId, Vec<Message>) {
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
        let id = page.items.first().expect("a thread").id;
        let loaded = store.thread(id).unwrap();
        let messages = loaded
            .messages
            .iter()
            .filter_map(|m| store.message(*m).ok())
            .collect();
        (id, messages)
    }

    /// Star the newest thread locally, exactly as a hover button does.
    fn star(store: &SqliteStore, caps: &AccountCaps) -> ThreadId {
        let (id, messages) = thread_of(store);
        let loaded = store.thread(id).unwrap();
        let applied = Op::SetStar(Star::Starred).apply(
            &Target::Threads(vec![id]),
            &loaded,
            &messages,
            caps,
            now(),
        );
        store.apply(ACCOUNT, &applied.forward).unwrap();
        if let Some(intent) = applied.remote {
            store
                .enqueue(ACCOUNT, intent, &applied.inverse, now())
                .unwrap();
        }
        id
    }

    fn is_starred(store: &SqliteStore, thread: ThreadId) -> bool {
        store
            .thread(thread)
            .unwrap()
            .messages
            .iter()
            .filter_map(|m| store.message(*m).ok())
            .any(|m| m.star == Star::Starred)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn starring_reaches_the_server_and_survives_the_next_sync() {
        let seen: Shared = Arc::new(Mutex::new(Seen::default()));
        let (port, _validity, _present, flags) = serve_flags(seen.clone(), Fault::None).await;
        let mut it = engine(port, tempfile::tempdir().unwrap());
        let (_tx, mut cancel) = watch::channel(false);

        it.engine
            .sync(&inbox(), &mut cancel, now(), 200)
            .await
            .unwrap();
        let thread = star(&it.store, &caps());
        assert!(is_starred(&it.store, thread), "the optimistic apply");

        // The outbox carries it to the server.
        let report = it.engine.drain_outbox(&mut cancel, now()).await.unwrap();
        assert_eq!(report.outbox_settled, 1, "{report:?}");
        assert!(
            flags
                .lock()
                .unwrap()
                .values()
                .any(|f| f.iter().any(|flag| flag == "\\Flagged")),
            "the server was never told: {:?}",
            seen.lock().unwrap().commands
        );

        // And the next sync, which now hears \Flagged back, leaves it starred.
        it.engine
            .sync(&inbox(), &mut cancel, now(), 200)
            .await
            .unwrap();
        assert!(
            is_starred(&it.store, thread),
            "server truth un-starred what the user starred"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_star_not_yet_sent_survives_server_truth_that_contradicts_it() {
        // The reconciliation rule, over a socket. The user stars a message; the outbox has not
        // drained yet; a sync arrives carrying the server's older opinion. Writing that opinion
        // straight in is how a star flips back under the user's cursor one poll after they set
        // it — the bug the whole pending_changes mechanism exists to prevent.
        let seen: Shared = Arc::new(Mutex::new(Seen::default()));
        let (port, _validity, _present, _flags) = serve_flags(seen, Fault::None).await;
        let mut it = engine(port, tempfile::tempdir().unwrap());
        let (_tx, mut cancel) = watch::channel(false);

        it.engine
            .sync(&inbox(), &mut cancel, now(), 200)
            .await
            .unwrap();
        let thread = star(&it.store, &caps());

        // No drain. The server still says unstarred.
        it.engine
            .sync(&inbox(), &mut cancel, now(), 200)
            .await
            .unwrap();

        assert!(
            is_starred(&it.store, thread),
            "an undelivered local change was overwritten by server truth"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_settled_change_stops_being_re_layered() {
        // The other half: once the server agrees, the pending row must go. A pending change that
        // outlived its confirmation would re-apply itself over every future ingest, so the user
        // could never un-star the message again.
        let seen: Shared = Arc::new(Mutex::new(Seen::default()));
        let (port, _validity, _present, _flags) = serve_flags(seen, Fault::None).await;
        let mut it = engine(port, tempfile::tempdir().unwrap());
        let (_tx, mut cancel) = watch::channel(false);

        it.engine
            .sync(&inbox(), &mut cancel, now(), 200)
            .await
            .unwrap();
        star(&it.store, &caps());
        it.engine.drain_outbox(&mut cancel, now()).await.unwrap();

        let pending: i64 = it
            .store
            .connection()
            .query_row("SELECT count(*) FROM pending_changes", [], |r| r.get(0))
            .unwrap();
        assert_eq!(pending, 0, "a confirmed change is still pending");

        let queued = it.store.outbox_due(ACCOUNT, now()).unwrap();
        assert!(queued.is_empty(), "a settled operation is still queued");
    }
}

/// Archiving, and the two commands that must never appear while doing it.
///
/// `CONVENTIONS.md` forbids `\Deleted` + `EXPUNGE`: Gmail routes expunging through a per-account
/// setting that may be `deleteForever` and cannot be read over IMAP, so a client that completes
/// a move that way can permanently destroy mail on an account whose owner never agreed to it.
mod archiving {
    use super::*;

    fn caps_archiving(move_ext: MoveExt) -> AccountCaps {
        AccountCaps {
            archive: ArchiveMeans::MoveToFolder("Archive".to_owned()),
            move_ext,
            ..caps()
        }
    }

    /// Archive the newest thread, exactly as the hover button does.
    async fn archive(it: &mut Fixture, caps: &AccountCaps, cancel: &mut mail_runtime::Cancel) {
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
        let id = page.items.first().expect("a thread").id;
        let loaded = it.store.thread(id).unwrap();
        let messages: Vec<Message> = loaded
            .messages
            .iter()
            .filter_map(|m| it.store.message(*m).ok())
            .collect();
        let applied =
            Op::Archive.apply(&Target::Threads(vec![id]), &loaded, &messages, caps, now());
        it.store.apply(ACCOUNT, &applied.forward).unwrap();
        if let Some(intent) = applied.remote {
            it.store
                .enqueue(ACCOUNT, intent, &applied.inverse, now())
                .unwrap();
        }
        it.engine.drain_outbox(cancel, now()).await.unwrap();
    }

    fn commands(seen: &Shared) -> Vec<String> {
        seen.lock().unwrap().commands.clone()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_server_with_move_gets_uid_move() {
        // The capability existed, was checked, and led to an empty block — so this path copied
        // and left the original where it was, whatever the server supported.
        let seen: Shared = Arc::new(Mutex::new(Seen::default()));
        let (port, _v, _p, _f) = serve_flags(seen.clone(), Fault::None).await;
        let mut it = engine_with(
            port,
            tempfile::tempdir().unwrap(),
            caps_archiving(MoveExt::Supported),
        );
        let (_tx, mut cancel) = watch::channel(false);

        it.engine
            .sync(&inbox(), &mut cancel, now(), 200)
            .await
            .unwrap();
        seen.lock().unwrap().commands.clear();
        archive(&mut it, &caps_archiving(MoveExt::Supported), &mut cancel).await;

        let sent = commands(&seen);
        assert!(
            sent.iter()
                .any(|c| c.to_uppercase().starts_with("UID MOVE")),
            "a MOVE-capable server was not sent UID MOVE: {sent:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_server_without_move_copies_and_stops() {
        let seen: Shared = Arc::new(Mutex::new(Seen::default()));
        let (port, _v, _p, _f) = serve_flags(seen.clone(), Fault::None).await;
        let mut it = engine_with(
            port,
            tempfile::tempdir().unwrap(),
            caps_archiving(MoveExt::Absent),
        );
        let (_tx, mut cancel) = watch::channel(false);

        it.engine
            .sync(&inbox(), &mut cancel, now(), 200)
            .await
            .unwrap();
        seen.lock().unwrap().commands.clear();
        archive(&mut it, &caps_archiving(MoveExt::Absent), &mut cancel).await;

        let sent = commands(&seen);
        assert!(
            sent.iter()
                .any(|c| c.to_uppercase().starts_with("UID COPY")),
            "{sent:?}"
        );
        assert!(
            !sent
                .iter()
                .any(|c| c.to_uppercase().starts_with("UID MOVE")),
            "MOVE was sent to a server that never offered it: {sent:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn nothing_this_client_sends_can_destroy_mail() {
        // The property, asserted over every command of a full working session rather than of one
        // operation: sync, archive with MOVE, archive without it, flags, sweep. `\Deleted` and
        // `EXPUNGE` must appear nowhere at all.
        for move_ext in [MoveExt::Supported, MoveExt::Absent] {
            let seen: Shared = Arc::new(Mutex::new(Seen::default()));
            let (port, _v, _p, _f) = serve_flags(seen.clone(), Fault::None).await;
            let caps = caps_archiving(move_ext);
            let mut it = engine_with(port, tempfile::tempdir().unwrap(), caps.clone());
            let (_tx, mut cancel) = watch::channel(false);

            it.engine
                .sync(&inbox(), &mut cancel, now(), 200)
                .await
                .unwrap();
            archive(&mut it, &caps, &mut cancel).await;
            let later = now() + chrono::TimeDelta::try_hours(2).unwrap();
            it.engine.sweep(&inbox(), &mut cancel, later).await.unwrap();

            for command in commands(&seen) {
                let upper = command.to_uppercase();
                // One backslash, not two. This read `"\\\\DELETED"` until the APPEND tests
                // showed the same over-escaping elsewhere — two literal backslashes, which no
                // command contains, so the assertion could never fire. A safety check that
                // cannot fail is not a safety check.
                assert!(
                    !upper.contains("\\DELETED"),
                    "a \\Deleted flag was sent: {command}"
                );
                assert!(
                    !upper.starts_with("EXPUNGE") && !upper.starts_with("UID EXPUNGE"),
                    "an EXPUNGE was sent: {command}"
                );
            }
        }
    }
}

/// Capabilities, which were stored once as a guess and never checked against the server.
mod discovery {
    use super::*;

    fn stored_caps(store: &SqliteStore) -> AccountCaps {
        let text: String = store
            .connection()
            .query_row(
                "SELECT caps FROM account_caps WHERE account = ?1",
                [ACCOUNT.to_string()],
                |r| r.get(0),
            )
            .expect("capabilities were written");
        serde_json::from_str(&text).expect("stored capabilities decode")
    }

    /// An engine whose stored capabilities claim the server supports nothing.
    fn pessimistic(port: u16) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let it = engine_with(port, dir, caps());
        it.store
            .put_caps(
                ACCOUNT,
                &caps(),
                now() - chrono::TimeDelta::try_days(7).unwrap(),
            )
            .unwrap();
        it
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn asking_the_server_replaces_the_guess() {
        // The fake server advertises CONDSTORE-less IMAP4rev1 plus UIDPLUS and MOVE. The stored
        // expectation says MOVE is absent, and nothing would ever have corrected it.
        let seen: Shared = Arc::new(Mutex::new(Seen::default()));
        let (port, _v, _p, _f) = serve_flags(seen.clone(), Fault::None).await;
        let mut it = pessimistic(port);
        let (_tx, mut cancel) = watch::channel(false);

        assert_eq!(caps().move_ext, MoveExt::Absent, "precondition: the guess");

        let found = it.engine.refresh_caps(&mut cancel, now()).await.unwrap();
        assert_eq!(
            found.move_ext,
            MoveExt::Supported,
            "MOVE was advertised and not noticed: {:?}",
            seen.lock().unwrap().commands
        );
        // And written down, so a restart does not go back to guessing.
        assert_eq!(stored_caps(&it.store).move_ext, MoveExt::Supported);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn capabilities_are_asked_for_after_authenticating() {
        // Gmail's pre-auth list omits CONDSTORE, MOVE and X-GM-EXT-1. Believing the first answer
        // reports a far less capable server than it is, so the walk asks twice and keeps the
        // later reply.
        let seen: Shared = Arc::new(Mutex::new(Seen::default()));
        let (port, _v, _p, _f) = serve_flags(seen.clone(), Fault::None).await;
        let mut it = pessimistic(port);
        let (_tx, mut cancel) = watch::channel(false);

        it.engine.refresh_caps(&mut cancel, now()).await.unwrap();

        let sent = seen.lock().unwrap().commands.clone();
        let login = sent.iter().position(|c| c == "LOGIN").expect("logged in");
        let after = sent
            .iter()
            .skip(login)
            .filter(|c| c.to_uppercase().starts_with("CAPABILITY"))
            .count();
        assert!(
            after >= 1,
            "capabilities were never asked for after authenticating: {sent:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_special_use_folders_are_discovered() {
        // The server's LIST reply marks Sent with \Sent. Without this walk FolderRoles stayed
        // empty for every account, so archiving and filing targeted a path nobody confirmed.
        let seen: Shared = Arc::new(Mutex::new(Seen::default()));
        let (port, _v, _p, _f) = serve_flags(seen, Fault::None).await;
        let mut it = pessimistic(port);
        let (_tx, mut cancel) = watch::channel(false);

        let found = it.engine.refresh_caps(&mut cancel, now()).await.unwrap();
        assert!(
            found
                .folders
                .0
                .iter()
                .any(|(path, role)| path == "Sent" && *role == MailboxRole::Sent),
            "the \\Sent folder was not recognised: {:?}",
            found.folders
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn fresh_capabilities_are_not_re_asked_every_pass() {
        // Re-reading them on every sync would be a wasted round trip every few minutes.
        let seen: Shared = Arc::new(Mutex::new(Seen::default()));
        let (port, _v, _p, _f) = serve_flags(seen, Fault::None).await;
        let mut it = engine_with(port, tempfile::tempdir().unwrap(), caps());
        let (_tx, mut cancel) = watch::channel(false);

        // `caps()` is observed at `now()`, so nothing is stale.
        assert!(!it.engine.caps_are_stale(now()));
        // A day later it is.
        assert!(
            it.engine
                .caps_are_stale(now() + chrono::TimeDelta::try_days(2).unwrap())
        );

        it.engine.refresh_caps(&mut cancel, now()).await.unwrap();
    }
}

/// Waiting for the server to say something, which nothing had ever asked it to do.
///
/// `ProtoOp::Watch` and `ProtoOutcome::Woken` were both implemented and neither was reachable:
/// no caller sent the op, no handler matched the outcome. So IDLE was never used on any server
/// that offered it, and new mail appeared only when something ran a sync — up to a full poll
/// interval after it arrived.
mod watching {
    use super::*;

    fn caps_idle() -> AccountCaps {
        AccountCaps {
            watch: WatchMode::Idle,
            ..caps()
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_server_offering_idle_is_waited_on() {
        let seen: Shared = Arc::new(Mutex::new(Seen::default()));
        let (port, _v, _p, _f) = serve_flags(seen.clone(), Fault::None).await;
        let mut it = engine_with(port, tempfile::tempdir().unwrap(), caps_idle());
        let (_tx, mut cancel) = watch::channel(false);

        // Bounded, because the failure this guards is a wait that never ends: with the news
        // check removed the session parks for ever, and a test that hangs blocks a run instead
        // of reporting. Five seconds is far longer than a loopback round trip.
        let woken = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            it.engine.watch(&inbox(), &mut cancel),
        )
        .await
        .expect("IDLE never ended: the server signalled and the session kept parking")
        .unwrap();
        assert!(woken, "the server signalled and nothing noticed");

        let sent = seen.lock().unwrap().commands.clone();
        assert!(
            sent.iter().any(|c| c.to_uppercase().starts_with("IDLE")),
            "IDLE was never sent: {sent:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_server_without_idle_says_so_rather_than_sleeping() {
        // `WatchMode::Poll` is not a worse kind of watching, it is the absence of watching.
        // Sleeping in here to imitate it would hide that from whoever schedules around it.
        let seen: Shared = Arc::new(Mutex::new(Seen::default()));
        let (port, _v, _p, _f) = serve_flags(seen.clone(), Fault::None).await;
        let mut it = engine_with(port, tempfile::tempdir().unwrap(), caps());
        let (_tx, mut cancel) = watch::channel(false);

        assert!(
            matches!(caps().watch, WatchMode::Poll { .. }),
            "precondition"
        );
        let woken = it.engine.watch(&inbox(), &mut cancel).await.unwrap();
        assert!(!woken, "a polling account claimed to have been woken");
        assert!(
            seen.lock().unwrap().commands.is_empty(),
            "a connection was opened for a server with no IDLE"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_parked_watch_can_be_cancelled() {
        // The reason `IoReady::Interrupt` exists. An IDLE with no traffic parks for as long as
        // the server allows, so without a way in from outside, quitting would block on a socket
        // that is behaving perfectly. This server never speaks, which is the parking case.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let _ = sock
                        .write_all(b"* OK [CAPABILITY IMAP4rev1] ready\r\n")
                        .await;
                    // Answer the login and the select, then go quiet for ever.
                    let mut buf = [0u8; 4096];
                    let mut replied = 0;
                    while let Ok(n) = sock.read(&mut buf).await {
                        if n == 0 {
                            return;
                        }
                        replied += 1;
                        let reply = match replied {
                            1 => "a001 OK logged in\r\n".to_owned(),
                            2 => "* 2 EXISTS\r\na002 OK [READ-ONLY] done\r\n".to_owned(),
                            // IDLE: acknowledge, then never speak again.
                            _ => "+ idling\r\n".to_owned(),
                        };
                        if sock.write_all(reply.as_bytes()).await.is_err() {
                            return;
                        }
                    }
                });
            }
        });

        let mut it = engine_with(port, tempfile::tempdir().unwrap(), caps_idle());
        let (tx, mut cancel) = watch::channel(false);
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
            let _ = tx.send(true);
        });

        let outcome = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            it.engine.watch(&inbox(), &mut cancel),
        )
        .await;
        assert!(
            outcome.is_ok(),
            "a parked IDLE ignored the interrupt and had to be timed out"
        );
    }
}

/// Uploading a draft, which `ProtoOp::Append` was written for and nothing called.
mod appending {
    use super::*;

    fn draft() -> Draft {
        Draft {
            id: DraftId::generate(),
            account: ACCOUNT,
            identity: IdentityId::generate(),
            to: vec![Address {
                name: None,
                email: "ada@example.test".to_owned(),
            }],
            cc: vec![],
            bcc: vec![],
            subject: "half a thought".to_owned(),
            in_reply_to: None,
            forward_of: None,
            text: "to be continued".to_owned(),
            html: None,
            attachments: vec![],
            receipt: ReceiptRequest::Unrequested,
            openpgp: OpenPgp::None,
            state: SendState::Editing,
            updated: now(),
        }
    }

    const RAW: &[u8] = b"From: me@example.test\r\n\
Subject: half a thought\r\n\
\r\n\
to be continued\r\n\
.a line that starts with a dot\r\n";

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_draft_reaches_the_servers_drafts_folder() {
        let seen: Shared = Arc::new(Mutex::new(Seen::default()));
        let (port, _v, _p, _f, appended) = serve_appending(seen.clone(), Fault::None).await;
        let mut it = engine_with(port, tempfile::tempdir().unwrap(), caps());
        let (_tx, mut cancel) = watch::channel(false);

        // The Drafts path comes from the server, not from a guess, so discovery runs first.
        it.engine.refresh_caps(&mut cancel, now()).await.unwrap();

        let uploaded = it
            .engine
            .upload_draft(&draft(), RAW.to_vec(), &mut cancel)
            .await
            .expect("the append succeeds");
        assert!(
            uploaded,
            "the server named a Drafts folder and nothing used it"
        );

        let stored = appended.lock().unwrap().clone();
        assert_eq!(
            stored.len(),
            1,
            "nothing was appended: {:?}",
            seen.lock().unwrap().commands
        );
        assert!(stored[0].contains("half a thought"), "{}", stored[0]);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_literal_arrives_byte_for_byte() {
        // A literal is framed by a byte count, not by a terminator, so a body containing a line
        // that begins with a dot — or a CRLF, or anything else — must survive untouched. A count
        // that disagrees with what follows desynchronises the connection entirely.
        let seen: Shared = Arc::new(Mutex::new(Seen::default()));
        let (port, _v, _p, _f, appended) = serve_appending(seen, Fault::None).await;
        let mut it = engine_with(port, tempfile::tempdir().unwrap(), caps());
        let (_tx, mut cancel) = watch::channel(false);
        it.engine.refresh_caps(&mut cancel, now()).await.unwrap();

        it.engine
            .upload_draft(&draft(), RAW.to_vec(), &mut cancel)
            .await
            .unwrap();

        let stored = appended.lock().unwrap().clone();
        assert_eq!(
            stored[0].as_bytes(),
            RAW,
            "the literal was altered in transit"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_draft_is_marked_as_a_draft_and_already_read() {
        // Without \\Draft the message shows up as ordinary mail in the folder; without \\Seen the
        // user's own unfinished note arrives as unread mail on every device they own.
        let seen: Shared = Arc::new(Mutex::new(Seen::default()));
        let (port, _v, _p, _f, _a) = serve_appending(seen.clone(), Fault::None).await;
        let mut it = engine_with(port, tempfile::tempdir().unwrap(), caps());
        let (_tx, mut cancel) = watch::channel(false);
        it.engine.refresh_caps(&mut cancel, now()).await.unwrap();

        it.engine
            .upload_draft(&draft(), RAW.to_vec(), &mut cancel)
            .await
            .unwrap();

        let append = seen
            .lock()
            .unwrap()
            .commands
            .iter()
            .find(|c| c.to_uppercase().starts_with("APPEND"))
            .cloned()
            .expect("an APPEND was sent");
        assert!(append.contains("\\Draft"), "{append}");
        assert!(append.contains("\\Seen"), "{append}");
        assert!(append.contains("Drafts"), "{append}");
    }

    /// An import's upload: queued in the outbox, sent by the drain with its own flags and date,
    /// and kept here at the address `APPENDUID` gave it, so a second import finds it held.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_queued_upload_is_sent_and_kept_at_the_address_the_server_gave() {
        let seen: Shared = Arc::new(Mutex::new(Seen::default()));
        let (port, _v, _p, _f, appended) = serve_appending(seen.clone(), Fault::None).await;
        let mut it = engine_with(port, tempfile::tempdir().unwrap(), caps());
        let (_tx, mut cancel) = watch::channel(false);

        const IMPORTED: &[u8] = b"From: old@example.test\r\n\
Message-ID: <imported-1@example.test>\r\n\
Subject: from the archive\r\n\
\r\n\
kept for years\r\n";
        let raw = it
            .store
            .blobs()
            .put(&it.store.connection(), IMPORTED)
            .unwrap();
        let mailbox = MailboxRef {
            account: ACCOUNT,
            path: "Archive".to_owned(),
        };
        it.store
            .enqueue(
                ACCOUNT,
                RemoteIntent::Append {
                    mailbox: mailbox.clone(),
                    flags: vec![SystemFlag::Seen],
                    date: Some(Utc.timestamp_opt(1_600_000_000, 0).unwrap()),
                    raw,
                },
                &Patch {
                    id: ChangeId::generate(),
                    changes: vec![],
                },
                now(),
            )
            .unwrap()
            .expect("queued");

        let report = it.engine.drain_outbox(&mut cancel, now()).await.unwrap();
        assert_eq!(report.appended, 1, "{report:?}");
        assert_eq!(appended.lock().unwrap()[0].as_bytes(), IMPORTED);
        let line = seen
            .lock()
            .unwrap()
            .commands
            .iter()
            .find(|c| c.to_uppercase().starts_with("APPEND"))
            .cloned()
            .expect("an APPEND was sent");
        assert!(line.contains("(\\Seen)"), "{line}");
        assert!(line.contains("\"13-Sep-2020 12:26:40 +0000\""), "{line}");

        let key = MessageKey::Rfc("imported-1@example.test".to_owned());
        assert!(it.store.holds(ACCOUNT, &key).unwrap());
        assert_eq!(
            it.store.remote_refs(&mailbox).unwrap(),
            vec![RemoteRef::Imap {
                mailbox: "Archive".to_owned(),
                uidvalidity: 42,
                uid: 103,
            }]
        );
        assert!(it.store.outbox_due(ACCOUNT, now()).unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_account_with_no_drafts_folder_says_so_rather_than_guessing() {
        // Uploading into a path nobody confirmed is how a message lands somewhere the user will
        // never look. `caps()` has an empty FolderRoles and no discovery has run.
        let seen: Shared = Arc::new(Mutex::new(Seen::default()));
        let (port, _v, _p, _f, appended) = serve_appending(seen, Fault::None).await;
        let mut it = engine_with(port, tempfile::tempdir().unwrap(), caps());
        let (_tx, mut cancel) = watch::channel(false);

        let uploaded = it
            .engine
            .upload_draft(&draft(), RAW.to_vec(), &mut cancel)
            .await
            .expect("no folder is not an error");
        assert!(!uploaded, "a folder was invented");
        assert!(
            appended.lock().unwrap().is_empty(),
            "something was uploaded anyway"
        );
    }
}
