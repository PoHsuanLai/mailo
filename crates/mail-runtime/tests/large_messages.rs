//! A large IMAP message arrives as its text, and its attachment waits on the server.
//!
//! The pieces each have their own tests — the backend turns `BODYSTRUCTURE` into a tree,
//! `mail-mime` rebuilds a message from sections, the store records a part as held. This is the
//! engine putting them together: which sections it asks for, what it stores, when it falls back
//! to fetching the message whole, and the download when someone opens the attachment.
//!
//! The backend is scripted rather than a server: what is asserted is the engine's decisions, and
//! every wire format involved is tested where it is parsed.

use chrono::{DateTime, TimeZone, Utc};
use mail_domain::*;
use mail_proto::{Backend, IoReady, Progress, ProtoError, ProtoOutcome};
use mail_runtime::{AccountEngine, Arrival, MapSecrets};
use mail_store::{SqliteStore, Store};
use std::sync::{Arc, Mutex};
use tokio::sync::watch;

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));

fn now() -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000, 0).unwrap()
}

const HEADER: &str = "From: Ada <ada@example.test>\r\nTo: me@example.test\r\nSubject: The report\r\nMessage-ID: <report@example.test>\r\nDate: Tue, 14 Nov 2023 22:13:20 +0000\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"mix\"\r\n\r\n";
const TEXT_MIME: &str = "Content-Type: text/plain; charset=utf-8\r\n\r\n";
const TEXT: &str = "The numbers are attached.";
const PDF_MIME: &str = "Content-Type: application/pdf; name=\"report.pdf\"\r\nContent-Disposition: attachment; filename=\"report.pdf\"\r\nContent-Transfer-Encoding: base64\r\n\r\n";
/// `%PDF-1.4` in base64. The server's figure below says nine megabytes, which is the point.
const PDF: &str = "JVBERi0xLjQ=";

fn remote() -> RemoteRef {
    RemoteRef::Imap {
        mailbox: "INBOX".to_owned(),
        uidvalidity: 1,
        uid: 7,
    }
}

fn inbox() -> MailboxRef {
    MailboxRef {
        account: ACCOUNT,
        path: "INBOX".to_owned(),
    }
}

fn tree() -> PartTree {
    PartTree::Multipart {
        section: String::new(),
        subtype: "mixed".to_owned(),
        boundary: "mix".to_owned(),
        parts: vec![
            PartTree::Leaf {
                section: "1".to_owned(),
                mime: "text/plain".to_owned(),
                octets: TEXT.len() as u64,
                attachment: false,
            },
            PartTree::Leaf {
                section: "2".to_owned(),
                mime: "application/pdf".to_owned(),
                octets: 9_000_000,
                attachment: true,
            },
        ],
    }
}

fn section(name: &str) -> Vec<u8> {
    match name {
        "HEADER" => HEADER,
        "1.MIME" => TEXT_MIME,
        "1" => TEXT,
        "2.MIME" => PDF_MIME,
        "2" => PDF,
        other => panic!("asked for a section the message does not have: {other}"),
    }
    .as_bytes()
    .to_vec()
}

fn whole() -> Vec<u8> {
    format!("{HEADER}--mix\r\n{TEXT_MIME}{TEXT}\r\n--mix\r\n{PDF_MIME}{PDF}\r\n--mix--\r\n")
        .into_bytes()
}

/// What the engine asked for, in order.
type Asked = Arc<Mutex<Vec<String>>>;

struct Scripted {
    caps: AccountCaps,
    asked: Asked,
    /// F112: a server that fails producing a structure.
    structure_fails: bool,
}

impl Backend for Scripted {
    fn begin(&mut self, op: ProtoOp) -> Progress<ProtoOutcome> {
        match op {
            ProtoOp::FetchStructure { .. } => {
                self.asked.lock().unwrap().push("structure".to_owned());
                if self.structure_fails {
                    return Progress::Failed(ProtoError::Malformed("server crashed".to_owned()));
                }
                Progress::Done(ProtoOutcome::Structures(vec![(remote(), tree())]))
            }
            ProtoOp::FetchSections { remote, sections } => {
                self.asked
                    .lock()
                    .unwrap()
                    .push(format!("sections {}", sections.join(" ")));
                let parts = sections.iter().map(|s| (s.clone(), section(s))).collect();
                Progress::Done(ProtoOutcome::Sections { remote, parts })
            }
            ProtoOp::FetchBody { remotes } => {
                self.asked.lock().unwrap().push("whole".to_owned());
                Progress::Done(ProtoOutcome::Fetched {
                    items: remotes.into_iter().map(|r| (r, whole())).collect(),
                    flags: Vec::new(),
                })
            }
            other => panic!("not scripted: {other:?}"),
        }
    }

    fn feed(&mut self, _: IoReady) -> Progress<ProtoOutcome> {
        unreachable!("every op above is answered without I/O")
    }

    fn caps(&self) -> &AccountCaps {
        &self.caps
    }

    /// The envelope walk's answer: the message, and its size on the server.
    fn surveyed(&self) -> Vec<(RemoteRef, u64)> {
        vec![(remote(), 9_000_500)]
    }
}

/// Somewhere for the engine to connect before it asks the backend anything. Nothing is read or
/// written: the backend answers every op without I/O.
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

struct Fixture {
    engine: AccountEngine<Scripted>,
    store: Arc<SqliteStore>,
    asked: Asked,
    _dir: tempfile::TempDir,
}

fn fixture(structure_fails: bool) -> Fixture {
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
    // What the header pass left: the message listed, with no body yet.
    mail_runtime::absorb(
        &store,
        ACCOUNT,
        inbox(),
        None,
        vec![Arrival {
            remote: remote(),
            raw: HEADER.as_bytes().to_vec(),
        }],
        true,
        now(),
    )
    .unwrap();

    let caps = AccountCaps {
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
        observed_at: now(),
    };
    let plan = AccountPlan {
        address: "me@example.test".to_owned(),
        incoming: Incoming::Imap {
            host: "127.0.0.1".to_owned(),
            port: somewhere_to_connect(),
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
    let asked: Asked = Arc::default();
    let backend = Scripted {
        caps,
        asked: asked.clone(),
        structure_fails,
    };
    let engine = AccountEngine::new(
        ACCOUNT,
        plan,
        backend,
        store.clone(),
        Arc::new(MapSecrets::default()),
    );
    Fixture {
        engine,
        store,
        asked,
        _dir: dir,
    }
}

fn the_message(store: &SqliteStore) -> Message {
    let id: String = store
        .connection()
        .query_row("SELECT id FROM messages", [], |r| r.get(0))
        .unwrap();
    store
        .message(MessageId::from_uuid(id.parse().unwrap()))
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_large_message_arrives_without_its_attachment() {
    let mut it = fixture(false);
    let (_tx, mut cancel) = watch::channel(false);

    let report = it
        .engine
        .fetch_bodies(&inbox(), &mut cancel, now(), 10)
        .await
        .unwrap();
    assert_eq!(report.bodies_fetched, 1, "{:?}", report.needs_attention);

    assert_eq!(
        *it.asked.lock().unwrap(),
        ["structure", "sections HEADER 1.MIME 1 2.MIME"],
        "the attachment's content is the one thing not asked for, and nothing is fetched whole"
    );
    let message = the_message(&it.store);
    assert_eq!(message.body.text(), Some(TEXT));
    assert_eq!(message.attachments.len(), 1);
    assert_eq!(message.attachments[0].name, "report.pdf");
    assert_eq!(
        message.attachments[0].content,
        PartContent::Remote {
            section: "2".to_owned()
        }
    );
    assert_eq!(message.attachments[0].size, 9_000_000);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn opening_the_attachment_downloads_and_decodes_it() {
    let mut it = fixture(false);
    let (_tx, mut cancel) = watch::channel(false);
    it.engine
        .fetch_bodies(&inbox(), &mut cancel, now(), 10)
        .await
        .unwrap();
    it.asked.lock().unwrap().clear();
    let id = the_message(&it.store).id;

    let blob = it.engine.fetch_part(id, "2", &mut cancel).await.unwrap();

    assert_eq!(*it.asked.lock().unwrap(), ["sections 2.MIME 2"]);
    let bytes = it.store.blobs().get(&it.store.connection(), blob).unwrap();
    assert_eq!(
        bytes, b"%PDF-1.4",
        "stored decoded, not as the base64 that was fetched"
    );
    let message = the_message(&it.store);
    assert_eq!(message.attachments[0].content, PartContent::Held(blob));
    assert_eq!(
        message.attachments[0].size, 8,
        "the real size replaces the server's figure"
    );
}

/// F112: a server that fails to produce a structure. The message still arrives, whole.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_server_that_cannot_describe_the_message_sends_it_whole() {
    let mut it = fixture(true);
    let (_tx, mut cancel) = watch::channel(false);

    let report = it
        .engine
        .fetch_bodies(&inbox(), &mut cancel, now(), 10)
        .await
        .unwrap();

    assert_eq!(report.bodies_fetched, 1, "{:?}", report.needs_attention);
    assert_eq!(*it.asked.lock().unwrap(), ["structure", "whole"]);
    let message = the_message(&it.store);
    assert_eq!(message.attachments[0].name, "report.pdf");
    assert!(message.attachments[0].blob().is_some(), "held, as before");
}
