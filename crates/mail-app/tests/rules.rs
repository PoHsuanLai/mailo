//! `mailo rules`, `mailo vacation` and `mailo sieve` against a real store, and rules acting
//! inside a real sync pass against an IMAP server on a socket in this process.
//!
//! Nothing here reaches a ManageSieve server: the only accounts that would try are refused
//! before connecting, which is the behaviour under test. The push itself is tested against a
//! loopback server in `mail-runtime/tests/sieve.rs`.

use chrono::{DateTime, TimeZone, Utc};
use mail_app::{cli, sync};
use mail_domain::*;
use mail_proto::backend::{Authenticate, ImapBackend};
use mail_proto::{ImapAuth, ImapCommand, ImapSession};
use mail_runtime::{AccountEngine, MapSecrets};
use mail_store::{SqliteStore, Store};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

fn now() -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000, 0).unwrap()
}

fn run(store: &SqliteStore, words: &[&str]) -> Result<String, String> {
    let args: Vec<String> = words.iter().map(|w| (*w).to_owned()).collect();
    let command = cli::parse(&args)?;
    cli::run_with_clients(
        store,
        &command,
        now(),
        &mail_runtime::OAuthRegistry::default(),
    )
}

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));
const ME: &str = "me@example.test";

/// Write an account as `account add` would, without going near a credential.
fn configure(store: &SqliteStore, id: AccountId, plan: &AccountPlan, caps: &AccountCaps) {
    store
        .connection()
        .execute(
            "INSERT INTO accounts (id, address, plan, created_at) VALUES (?1, ?2, ?3, ?4)",
            [
                id.to_string(),
                plan.address.clone(),
                serde_json::to_string(plan).unwrap(),
                now().to_rfc3339(),
            ],
        )
        .unwrap();
    store.put_caps(id, caps, now()).unwrap();
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

/// A server with folders: archiving moves into `Archive`.
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
        move_ext: MoveExt::Supported,
        expunge: ExpungeMeans::Forbidden,
        top: Supported::Absent,
        pipelining: Supported::Absent,
        connections: ConnectionBudget { max: 1 },
        // Fresh, so a pass does not stop to re-read capabilities.
        observed_at: Utc::now(),
    }
}

fn store() -> (SqliteStore, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::in_memory(dir.path()).unwrap();
    configure(&store, ACCOUNT, &plan(1), &caps());
    (store, dir)
}

#[test]
fn a_rule_is_added_listed_switched_off_and_on_and_removed() {
    let (store, _dir) = store();
    let said = run(
        &store,
        &[
            "rules",
            "add",
            "Bills",
            "from:bank.example",
            "subject:statement",
            "--read",
            "--folder",
            "Money/Bills",
            "--stop",
        ],
    )
    .unwrap();
    assert!(
        said.contains("added rule \"Bills\" on me@example.test"),
        "{said}"
    );
    // A manual IMAP account may have ManageSieve, so the way to put it there is named.
    assert!(said.contains("mailo sieve push"), "{said}");
    run(
        &store,
        &["rules", "add", "News", "from:lists.example", "--archive"],
    )
    .unwrap();

    let listed = run(&store, &["rules"]).unwrap();
    assert_eq!(
        listed,
        "me@example.test:\n  \
         1. Bills: from:bank.example subject:statement → mark read, move to Money/Bills, stop\n  \
         2. News: from:lists.example → archive\n"
    );

    // The same name twice is refused, and says why.
    let again = run(&store, &["rules", "add", "News", "from:x", "--star"]).unwrap_err();
    assert!(again.contains("already a rule called \"News\""), "{again}");

    run(&store, &["rules", "disable", "Bills"]).unwrap();
    assert!(
        run(&store, &["rules", "list"])
            .unwrap()
            .contains("Bills (disabled)")
    );
    run(&store, &["rules", "enable", "Bills"]).unwrap();
    assert!(
        !run(&store, &["rules", "list"])
            .unwrap()
            .contains("disabled")
    );

    run(&store, &["rules", "remove", "Bills"]).unwrap();
    let left: Vec<String> = store
        .rules(ACCOUNT)
        .unwrap()
        .into_iter()
        .map(|r| r.name)
        .collect();
    assert_eq!(left, ["News"]);
    let missing = run(&store, &["rules", "remove", "Bills"]).unwrap_err();
    assert!(missing.contains("no rule \"Bills\""), "{missing}");
}

/// One message from `from`, held in the inbox with a server address, as a sync leaves it.
fn held(store: &SqliteStore, uid: u32, from: &str) -> MessageId {
    let key = format!("{uid}@example.test");
    let raw = store
        .blobs()
        .put(
            &store.connection(),
            format!("From: {from}\r\n\r\nhi\r\n").as_bytes(),
        )
        .unwrap();
    let message = Message {
        id: MessageId::generate(),
        thread: ThreadId::generate(),
        account: ACCOUNT,
        key: MessageKey::Rfc(key.clone()),
        date: now() - chrono::Duration::days(i64::from(uid)),
        from: Address {
            name: None,
            email: from.to_owned(),
        },
        reply_to: vec![],
        to: vec![],
        cc: vec![],
        bcc: vec![],
        subject: format!("message {uid}"),
        in_reply_to: None,
        references: vec![],
        rfc_message_id: Some(key.clone()),
        read: ReadState::Unread,
        star: Star::Unstarred,
        mailbox: MailboxRole::Inbox,
        labels: vec![],
        body: Body::Present {
            text: Some("hi".to_owned()),
            raw,
        },
        attachments: vec![],
    };
    let id = message.id;
    store
        .ingest(
            ACCOUNT,
            Ingest {
                mailbox: MailboxRef {
                    account: ACCOUNT,
                    path: "INBOX".to_owned(),
                },
                validity: UidValidity::Same,
                cursor: None,
                messages: vec![Fetched {
                    remote: RemoteRef::Imap {
                        mailbox: "INBOX".to_owned(),
                        uidvalidity: 42,
                        uid,
                    },
                    key: MessageKey::Rfc(key),
                    raw,
                    message,
                }],
                flags: vec![],
                labels: vec![],
                label_names: vec![],
                gone: vec![],
            },
        )
        .unwrap();
    id
}

#[test]
fn rules_run_reaches_the_mail_already_here_and_queues_what_the_user_would() {
    let (store, _dir) = store();
    let news = held(&store, 1, "news@lists.example");
    let friend = held(&store, 2, "ada@example.test");
    run(
        &store,
        &[
            "rules",
            "add",
            "News",
            "from:lists.example",
            "--read",
            "--archive",
        ],
    )
    .unwrap();

    let said = run(&store, &["rules", "run", "News", "--batch", "1"]).unwrap();
    assert!(
        said.contains("2 message(s) looked at, 1 matched, 2 change(s) queued"),
        "{said}"
    );
    let news = store.message(news).unwrap();
    assert_eq!(
        (news.mailbox, news.read),
        (MailboxRole::Archive, ReadState::Read)
    );
    let friend = store.message(friend).unwrap();
    assert_eq!(
        (friend.mailbox, friend.read),
        (MailboxRole::Inbox, ReadState::Unread)
    );
    let ops: Vec<ProtoOp> = store
        .outbox_due(ACCOUNT, now() + chrono::Duration::days(1))
        .unwrap()
        .into_iter()
        .map(|e| e.op)
        .collect();
    let remote = RemoteRef::Imap {
        mailbox: "INBOX".to_owned(),
        uidvalidity: 42,
        uid: 1,
    };
    assert_eq!(
        ops,
        vec![
            ProtoOp::SetFlags {
                remotes: vec![remote.clone()],
                read: Some(ReadState::Read),
                star: None,
            },
            ProtoOp::SetMailbox {
                remotes: vec![remote],
                role: MailboxRole::Archive,
            },
        ]
    );
}

#[test]
fn server_side_rules_and_vacation_are_refused_where_the_provider_has_no_managesieve() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::in_memory(dir.path()).unwrap();
    let gmail = presets::preset_for("someone@gmail.com", now()).expect("gmail preset");
    configure(&store, ACCOUNT, &gmail.plan, &gmail.expected_caps);

    let body = dir.path().join("away.txt");
    std::fs::write(&body, "Back on Monday.").unwrap();
    let refused = run(
        &store,
        &[
            "vacation",
            "on",
            "--subject",
            "Away",
            "--body-file",
            body.to_str().unwrap(),
        ],
    )
    .unwrap_err();
    assert!(
        refused.contains("Google offers no ManageSieve"),
        "{refused}"
    );
    // Refused before anything was kept: a reply nobody will send is not recorded as on.
    assert_eq!(store.vacation(ACCOUNT).unwrap(), None);

    for words in [&["sieve", "push"][..], &["sieve", "status"][..]] {
        let refused = run(&store, words).unwrap_err();
        assert!(refused.contains("rules run in this client"), "{refused}");
    }
    // Rules themselves are still kept, and said to run here.
    let said = run(
        &store,
        &["rules", "add", "News", "from:lists.example", "--archive"],
    )
    .unwrap();
    assert!(!said.contains("sieve push"), "{said}");
}

// ---------------------------------------------------------------------------------------------
// Rules inside a sync pass.
// ---------------------------------------------------------------------------------------------

/// UID, raw bytes.
type Maildrop = Arc<Mutex<Vec<(u32, String)>>>;
/// Every command line the server received.
type Heard = Arc<Mutex<Vec<String>>>;

fn raw(uid: u32, from: &str, subject: &str) -> String {
    format!(
        "From: {from}\r\nTo: {ME}\r\nSubject: {subject}\r\nDate: {}\r\n\
         Message-ID: <{uid}@example.test>\r\n\r\nbody of {uid}\r\n",
        now().to_rfc2822()
    )
}

/// Enough of an IMAP server for one pass: select, survey, fetch, store, move, list.
async fn serve(drop: Maildrop, heard: Heard) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((sock, _)) = listener.accept().await {
            tokio::spawn(session(sock, drop.clone(), heard.clone()));
        }
    });
    port
}

async fn session(mut sock: tokio::net::TcpStream, drop: Maildrop, heard: Heard) {
    if sock
        .write_all(b"* OK [CAPABILITY IMAP4rev1] ready\r\n")
        .await
        .is_err()
    {
        return;
    }
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
            heard.lock().unwrap().push(line.clone());
            let (tag, rest) = line.split_once(' ').unwrap_or(("*", ""));
            let upper = rest.to_uppercase();
            let held = drop.lock().unwrap().clone();
            let reply = if upper.starts_with("SELECT") || upper.starts_with("EXAMINE") {
                let next = held.iter().map(|(uid, _)| uid + 1).max().unwrap_or(1);
                format!(
                    "* {} EXISTS\r\n* OK [UIDVALIDITY 42] ok\r\n* OK [UIDNEXT {next}] ok\r\n\
                     {tag} OK done\r\n",
                    held.len()
                )
            } else if upper.starts_with("UID FETCH") {
                let set = upper["UID FETCH ".len()..]
                    .split(' ')
                    .next()
                    .unwrap_or("")
                    .to_owned();
                let named: Vec<u32> = set.split(',').filter_map(|n| n.parse().ok()).collect();
                let mut out = String::new();
                for (seq, (uid, raw)) in held.iter().enumerate() {
                    if !set.contains(':') && !named.contains(uid) {
                        continue;
                    }
                    let item = if upper.contains("BODY.PEEK[HEADER]") {
                        let head =
                            raw.split("\r\n\r\n").next().unwrap_or("").to_owned() + "\r\n\r\n";
                        format!("FLAGS () BODY[HEADER] {{{}}}\r\n{head}", head.len())
                    } else if upper.contains("BODY.PEEK[]") {
                        format!("BODY[] {{{}}}\r\n{raw}", raw.len())
                    } else {
                        format!("FLAGS () RFC822.SIZE {}", raw.len())
                    };
                    out.push_str(&format!("* {} FETCH (UID {uid} {item})\r\n", seq + 1));
                }
                out + &format!("{tag} OK done\r\n")
            } else if upper.starts_with("UID SEARCH") {
                let uids: Vec<String> = held.iter().map(|(uid, _)| uid.to_string()).collect();
                format!("* SEARCH {}\r\n{tag} OK done\r\n", uids.join(" "))
            } else if upper.starts_with("LIST") {
                format!(
                    "* LIST () \"/\" \"INBOX\"\r\n* LIST (\\Archive) \"/\" \"Archive\"\r\n{tag} OK done\r\n"
                )
            } else if upper.starts_with("LSUB") {
                format!("* LSUB () \"/\" \"INBOX\"\r\n{tag} OK done\r\n")
            } else if upper.starts_with("LOGOUT") {
                let _ = sock
                    .write_all(format!("* BYE\r\n{tag} OK done\r\n").as_bytes())
                    .await;
                return;
            } else {
                // LOGIN, CAPABILITY, NOOP, UID STORE, UID MOVE: all simply done.
                format!("{tag} OK done\r\n")
            };
            if sock.write_all(reply.as_bytes()).await.is_err() {
                return;
            }
        }
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

async fn one_pass(port: u16, store: &Arc<SqliteStore>) -> mail_runtime::SyncReport {
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
    sync::drive(
        &mut engine,
        &account,
        &inbox,
        &mut cancel,
        Utc::now(),
        sync::Mode::Once,
        sync::Announce::Quietly,
    )
    .await
    .unwrap()
}

fn commands_like(heard: &Heard, what: &str) -> Vec<String> {
    heard
        .lock()
        .unwrap()
        .iter()
        .filter_map(|l| l.split_once(' ').map(|(_, rest)| rest.to_owned()))
        .filter(|rest| rest.starts_with(what))
        .collect()
}

#[tokio::test]
async fn a_rule_acts_on_mail_the_pass_it_arrives_and_the_server_hears_it_once() {
    let drop: Maildrop = Arc::new(Mutex::new(vec![(
        1,
        raw(1, "Old <news@lists.example>", "before the rule"),
    )]));
    let heard: Heard = Arc::default();
    let port = serve(drop.clone(), heard.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::open(dir.path().join("mail.db"), dir.path()).unwrap());
    configure(&store, ACCOUNT, &plan(port), &caps());

    // A pass before any rule: the old newsletter is simply fetched.
    one_pass(port, &store).await;
    run(
        &store,
        &[
            "rules",
            "add",
            "News",
            "from:lists.example",
            "--read",
            "--archive",
        ],
    )
    .unwrap();

    // New mail: one the rule matches, one it does not.
    drop.lock().unwrap().extend([
        (2, raw(2, "News <news@lists.example>", "this week")),
        (3, raw(3, "Ada <ada@example.test>", "lunch")),
    ]);
    let report = one_pass(port, &store).await;
    assert_eq!(report.ruled.len(), 1, "{:?}", report.ruled);
    assert_eq!(report.ruled[0].1, vec!["News".to_owned()]);

    // Told the server as a person archiving and reading it would have: in this same pass, and
    // about the new message only — never the one that was there before the rule.
    assert_eq!(
        commands_like(&heard, "UID STORE"),
        ["UID STORE 2 +FLAGS (\\Seen)"]
    );
    assert_eq!(
        commands_like(&heard, "UID MOVE"),
        ["UID MOVE 2 \"Archive\""]
    );

    // A later pass finds nothing new and asks nothing more.
    let report = one_pass(port, &store).await;
    assert!(report.ruled.is_empty());
    assert_eq!(commands_like(&heard, "UID STORE").len(), 1);
    assert_eq!(commands_like(&heard, "UID MOVE").len(), 1);
}
