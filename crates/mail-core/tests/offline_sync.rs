//! An account kept offline in full: a pass fetches the attachments a large message left on the
//! server, largest last, and an account not kept so leaves them there.
//!
//! One whole pass through `sync::drive`, against a scripted IMAP backend: two large messages
//! arrive as headers, then as their text with their attachments left behind (`plan.md` 9.6),
//! and then, only with the setting on, each attachment on its own. What is asserted is which
//! parts the pass asked the server for, in what order, and what the store says is held after.

use chrono::{DateTime, TimeZone, Utc};
use mail_core::offline::Keep;
use mail_core::sync::report::{AccountReport, PassEnd};
use mail_core::sync::{self, Configured};
use mail_domain::*;
use mail_proto::{Backend, IoReady, Progress, ProtoOutcome};
use mail_runtime::{AccountEngine, MapSecrets};
use mail_store::{SqliteStore, Store};
use std::sync::{Arc, Mutex};

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));

fn now() -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000, 0).unwrap()
}

fn inbox() -> MailboxRef {
    MailboxRef {
        account: ACCOUNT,
        path: "INBOX".to_owned(),
    }
}

fn at(uid: u32) -> RemoteRef {
    RemoteRef::Imap {
        mailbox: "INBOX".to_owned(),
        uidvalidity: 1,
        uid,
    }
}

/// The two messages: uid 7 carries a 9 MB PDF as "2" and a 2 MB archive as "3"; uid 8 a 3 MB
/// PDF as "2". Sizes are the server's figures; the bytes behind them are a few.
/// An attachment: its section, its type, and the server's figure for its size.
type Part = (&'static str, &'static str, u64);

const MESSAGES: [(u32, &[Part]); 2] = [
    (
        7,
        &[
            ("2", "application/pdf", 9_000_000),
            ("3", "application/zip", 2_000_000),
        ],
    ),
    (8, &[("2", "application/pdf", 3_000_000)]),
];

fn header(uid: u32) -> String {
    format!(
        "From: Ada <ada@example.test>\r\nTo: me@example.test\r\nSubject: Report {uid}\r\nMessage-ID: <report-{uid}@example.test>\r\nDate: Tue, 14 Nov 2023 22:13:{uid:02} +0000\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"mix\"\r\n\r\n"
    )
}

fn parts_of(uid: u32) -> &'static [Part] {
    MESSAGES
        .iter()
        .find(|(u, _)| *u == uid)
        .map(|(_, parts)| *parts)
        .unwrap_or_else(|| panic!("no message {uid}"))
}

const TEXT: &str = "The numbers are attached.";

fn tree(uid: u32) -> PartTree {
    let mut parts = vec![PartTree::Leaf {
        section: "1".to_owned(),
        mime: "text/plain".to_owned(),
        octets: TEXT.len() as u64,
        attachment: false,
    }];
    parts.extend(
        parts_of(uid)
            .iter()
            .map(|(section, mime, octets)| PartTree::Leaf {
                section: (*section).to_owned(),
                mime: (*mime).to_owned(),
                octets: *octets,
                attachment: true,
            }),
    );
    PartTree::Multipart {
        section: String::new(),
        subtype: "mixed".to_owned(),
        boundary: "mix".to_owned(),
        parts,
    }
}

fn section(uid: u32, name: &str) -> Vec<u8> {
    let bytes = match name {
        "HEADER" => header(uid),
        "1.MIME" => "Content-Type: text/plain; charset=utf-8\r\n\r\n".to_owned(),
        "1" => TEXT.to_owned(),
        other => {
            let (part, mime) = other.split_once('.').unwrap_or((other, ""));
            let (_, kind, _) = parts_of(uid)
                .iter()
                .find(|(s, _, _)| *s == part)
                .unwrap_or_else(|| panic!("message {uid} has no section {other}"));
            match mime {
                "MIME" => format!(
                    "Content-Type: {kind}; name=\"part{part}\"\r\nContent-Disposition: attachment; filename=\"part{part}\"\r\nContent-Transfer-Encoding: base64\r\n\r\n"
                ),
                // `%PDF-1.4`, whatever the part claims to be.
                _ => "JVBERi0xLjQ=".to_owned(),
            }
        }
    };
    bytes.into_bytes()
}

fn uid(remote: &RemoteRef) -> u32 {
    match remote {
        RemoteRef::Imap { uid, .. } => *uid,
        other => panic!("IMAP only here: {other:?}"),
    }
}

/// Every attachment fetched on its own, as `uid:section`, in the order asked.
type Asked = Arc<Mutex<Vec<String>>>;

struct Scripted {
    caps: AccountCaps,
    parts: Asked,
}

impl Backend for Scripted {
    fn begin(&mut self, op: ProtoOp) -> Progress<ProtoOutcome> {
        Progress::Done(match op {
            ProtoOp::FetchEnvelopes { mailbox, .. } => ProtoOutcome::Ingested(Box::new(Ingest {
                mailbox,
                validity: UidValidity::Same,
                cursor: None,
                messages: Vec::new(),
                flags: Vec::new(),
                labels: Vec::new(),
                label_names: Vec::new(),
                gone: Vec::new(),
            })),
            ProtoOp::FetchHeaders { remotes } => ProtoOutcome::Fetched {
                items: remotes
                    .into_iter()
                    .map(|r| {
                        let raw = header(uid(&r)).into_bytes();
                        (r, raw)
                    })
                    .collect(),
                flags: Vec::new(),
            },
            ProtoOp::FetchStructure { remotes } => ProtoOutcome::Structures(
                remotes
                    .into_iter()
                    .map(|r| (r.clone(), tree(uid(&r))))
                    .collect(),
            ),
            ProtoOp::FetchSections { remote, sections } => {
                let n = uid(&remote);
                // A part on its own is its header and its content and nothing else; the first
                // fetch of a large message always asks for the message's own header too.
                if let [mime, content] = sections.as_slice()
                    && *mime == format!("{content}.MIME")
                {
                    self.parts.lock().unwrap().push(format!("{n}:{content}"));
                }
                let parts = sections
                    .iter()
                    .map(|s| (s.clone(), section(n, s)))
                    .collect();
                ProtoOutcome::Sections { remote, parts }
            }
            ProtoOp::FetchBody { remotes } => panic!("fetched whole, not by part: {remotes:?}"),
            _ => ProtoOutcome::Applied,
        })
    }

    fn feed(&mut self, _: IoReady) -> Progress<ProtoOutcome> {
        unreachable!("every op above is answered without I/O")
    }

    fn caps(&self) -> &AccountCaps {
        &self.caps
    }

    /// The server's listing: both messages, well over the size above which a message is
    /// fetched by its parts.
    fn surveyed(&self) -> Vec<(RemoteRef, u64)> {
        vec![(at(7), 11_000_500), (at(8), 3_000_500)]
    }
}

/// A port that accepts and says nothing: a pass connects before it asks the backend anything.
fn somewhere_to_connect() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        let mut held = Vec::new();
        for sock in listener.incoming().flatten() {
            held.push(sock);
        }
    });
    port
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
        top: Supported::Absent,
        pipelining: Supported::Absent,
        connections: ConnectionBudget { max: 1 },
        // Fresh, so the pass does not ask the server for them again.
        observed_at: now(),
    }
}

struct Passed {
    report: AccountReport,
    parts: Vec<String>,
    store: Arc<SqliteStore>,
    _dir: tempfile::TempDir,
}

/// One first-sync pass of the account, kept as `keep`.
async fn one_pass(keep: Keep) -> Passed {
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
    let port = somewhere_to_connect();
    let plan = AccountPlan {
        address: "me@example.test".to_owned(),
        incoming: Incoming::Imap {
            host: "127.0.0.1".to_owned(),
            port,
            tls: Tls::Plaintext,
        },
        outgoing: Outgoing::Smtp {
            host: "127.0.0.1".to_owned(),
            port,
            tls: Tls::Plaintext,
        },
        auth: AuthPlan::Password {
            username: Username::SameAsAddress,
            sasl: vec![SaslMech::Plain],
        },
        identities: Vec::new(),
    };
    let parts: Asked = Arc::default();
    let mut engine = AccountEngine::new(
        ACCOUNT,
        plan.clone(),
        Scripted {
            caps: caps(),
            parts: parts.clone(),
        },
        store.clone(),
        Arc::new(MapSecrets::default()),
    );
    let account = Configured {
        id: ACCOUNT,
        address: "me@example.test".to_owned(),
        plan,
        caps: caps(),
        keep,
    };
    let (_tx, mut cancel) = tokio::sync::watch::channel(false);
    let report = match sync::drive(
        &mut engine,
        &account,
        &[inbox()],
        &mut cancel,
        now(),
        sync::Mode::Once,
        sync::Announce::Quietly,
        None,
    )
    .await
    {
        PassEnd::Finished(report) => report,
        other => panic!("the pass did not run to its end: {other:?}"),
    };
    let parts = parts.lock().unwrap().clone();
    Passed {
        report,
        parts,
        store,
        _dir: dir,
    }
}

fn messages(store: &SqliteStore) -> Vec<Message> {
    let ids: Vec<String> = {
        let db = store.connection();
        let mut stmt = db.prepare("SELECT id FROM messages ORDER BY date").unwrap();
        stmt.query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    ids.iter()
        .map(|id| {
            store
                .message(MessageId::from_uuid(id.parse().unwrap()))
                .unwrap()
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn kept_offline_every_part_is_fetched_largest_last() {
    let passed = one_pass(Keep::Everything).await;

    assert_eq!(
        passed.report.counts.bodies_fetched, 2,
        "{:?}",
        passed.report.trouble
    );
    assert_eq!(
        passed.parts,
        ["7:3", "8:2", "7:2"],
        "every attachment, the 2 MB one first and the 9 MB one last"
    );
    assert_eq!(passed.report.counts.parts_fetched, 3);
    let offline = passed.store.offline(ACCOUNT).unwrap();
    assert_eq!(
        (offline.messages, offline.held, offline.parts_remote),
        (2, 2, 0),
        "both messages are here in full"
    );
    // Only the parts are held: the stored message is still the one rebuilt from its parts
    // (F165), which export and forward tell from the whole one by its bytes.
    for message in messages(&passed.store) {
        let raw = message.body.raw().expect("a body");
        let bytes = passed
            .store
            .blobs()
            .get(&passed.store.connection(), raw)
            .unwrap();
        assert!(mail_mime::left_on_server(&bytes), "{}", message.subject);
        assert!(
            message.attachments.iter().all(|a| a.blob().is_some()),
            "{:?}",
            message.attachments
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn not_kept_offline_no_large_part_is_fetched() {
    let passed = one_pass(Keep::Bodies).await;

    assert_eq!(
        passed.report.counts.bodies_fetched, 2,
        "{:?}",
        passed.report.trouble
    );
    assert_eq!(passed.parts, Vec::<String>::new());
    assert_eq!(passed.report.counts.parts_fetched, 0);
    let offline = passed.store.offline(ACCOUNT).unwrap();
    assert_eq!(
        (offline.messages, offline.held, offline.parts_remote),
        (2, 0, 3),
        "the text is here and the attachments wait on the server"
    );
    assert_eq!(offline.remote_bytes, 14_000_000);
}

#[test]
fn the_offline_command_sets_one_account_and_says_where_each_stands() {
    let config = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let store = SqliteStore::in_memory(data.path()).unwrap();
    let accounts = [(ACCOUNT, "me@example.test".to_owned())];
    assert_eq!(
        mail_core::offline::load(config.path()).of(ACCOUNT),
        Keep::Bodies
    );
    let said = mail_core::offline::command(
        Some(config.path()),
        &store,
        &accounts,
        Some("ME@example.test"),
        Some(Keep::Everything),
    )
    .unwrap();
    assert_eq!(
        said,
        "me@example.test: all mail kept offline; 0 of 0 messages offline\n"
    );
    assert_eq!(
        mail_core::offline::load(config.path()).of(ACCOUNT),
        Keep::Everything
    );
    let nobody = mail_core::offline::command(
        Some(config.path()),
        &store,
        &accounts,
        Some("nobody@example.test"),
        None,
    );
    assert!(nobody.is_err(), "{nobody:?}");
}
