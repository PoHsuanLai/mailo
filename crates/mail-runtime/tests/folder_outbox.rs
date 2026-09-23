//! Folder work through the outbox, over a real socket: planned and applied here, queued, sent
//! after a restart, and either confirmed or put back.
//!
//! The server is a few lines of loopback IMAP that records what it was asked and refuses the
//! folder names it is told to. Its replies are written from RFC 3501 §6.3, not recorded.

use chrono::{DateTime, TimeZone, Utc};
use mail_domain::folder::{FolderContents, FolderCtx, plan};
use mail_domain::*;
use mail_proto::backend::{Authenticate, ImapBackend};
use mail_proto::{ImapAuth, ImapCommand, ImapSession};
use mail_runtime::{AccountEngine, MapSecrets, Secrets};
use mail_store::{SqliteStore, Store};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::watch;

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));

fn now() -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000, 0).unwrap()
}

type Heard = Arc<Mutex<Vec<String>>>;

/// A server that answers `NO` to any `CREATE` naming `refuse`, and lists what it was made to.
async fn serve(refuse: &'static str) -> (u16, Heard) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let heard: Heard = Arc::new(Mutex::new(Vec::new()));
    let log = heard.clone();
    tokio::spawn(async move {
        while let Ok((mut sock, _)) = listener.accept().await {
            let log = log.clone();
            tokio::spawn(async move {
                let _ = sock
                    .write_all(b"* OK [CAPABILITY IMAP4rev1] ready\r\n")
                    .await;
                let mut buf = Vec::new();
                loop {
                    let mut chunk = [0u8; 4096];
                    let n = match sock.read(&mut chunk).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => n,
                    };
                    buf.extend_from_slice(&chunk[..n]);
                    while let Some(at) = buf.windows(2).position(|w| w == b"\r\n") {
                        let line = String::from_utf8_lossy(&buf[..at]).to_string();
                        buf.drain(..at + 2);
                        let (tag, rest) = line.split_once(' ').unwrap_or((&line, ""));
                        let verb = rest.split(' ').next().unwrap_or("").to_ascii_uppercase();
                        if verb != "LOGIN" {
                            log.lock().unwrap().push(rest.to_owned());
                        }
                        let reply = match verb.as_str() {
                            "CREATE" if rest.contains(refuse) => {
                                format!("{tag} NO [CANNOT] Mailbox name not allowed\r\n")
                            }
                            "LIST" => format!(
                                "* LIST (\\HasNoChildren) \"/\" \"INBOX\"\r\n\
                                 * LIST (\\HasNoChildren) \"/\" \"&ZeVnLIqe-\"\r\n\
                                 {tag} OK done\r\n"
                            ),
                            "LSUB" => format!(
                                "* LSUB () \"/\" \"INBOX\"\r\n\
                                 * LSUB () \"/\" \"&ZeVnLIqe-\"\r\n\
                                 {tag} OK done\r\n"
                            ),
                            _ => format!("{tag} OK done\r\n"),
                        };
                        if sock.write_all(reply.as_bytes()).await.is_err() {
                            return;
                        }
                    }
                }
            });
        }
    });
    (port, heard)
}

fn account_plan(port: u16) -> AccountPlan {
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
        archive: ArchiveMeans::LocalOnly,
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

/// The store on disk under `dir`, so a second call is the same database after a restart.
fn open(dir: &tempfile::TempDir) -> Arc<SqliteStore> {
    let store = Arc::new(SqliteStore::open(dir.path().join("mail.db"), dir.path()).unwrap());
    store
        .connection()
        .execute(
            "INSERT OR IGNORE INTO accounts (id, address, plan, created_at)
             VALUES (?1, 'me@example.test', '{}', datetime('now'))",
            [ACCOUNT.to_string()],
        )
        .unwrap();
    store
}

fn engine(port: u16, store: Arc<SqliteStore>) -> AccountEngine<ImapBackend> {
    let secrets = MapSecrets::default();
    let password = Credential::Password("s3cr3t".to_owned());
    secrets
        .put(
            &SecretKey {
                account: ACCOUNT,
                purpose: SecretPurpose::IncomingPassword,
            },
            &password,
        )
        .unwrap();
    let auth = ImapAuth {
        username: "me@example.test".to_owned(),
        credential: password,
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
        account_plan(port),
        backend,
        store,
        Arc::new(secrets),
    )
}

/// What `mail_app::folder::change` does: plan, apply here, queue for the server.
fn request(store: &SqliteStore, port: u16, work: FolderWork) {
    let folders = store.folders(ACCOUNT).unwrap();
    let applied = plan(
        &work,
        &FolderCtx {
            account: ACCOUNT,
            incoming: &account_plan(port).incoming,
            caps: &caps(),
            folders: &folders,
            labels: &[],
            contents: &FolderContents::default(),
        },
    )
    .unwrap();
    store.apply(ACCOUNT, &applied.forward).unwrap();
    store
        .enqueue(ACCOUNT, applied.remote.unwrap(), &applied.inverse, now())
        .unwrap()
        .expect("folder work is always queued");
}

fn paths(store: &SqliteStore) -> Vec<String> {
    store
        .folders(ACCOUNT)
        .unwrap()
        .into_iter()
        .map(|f| f.path)
        .collect()
}

/// Made offline, sent after a restart, in modified UTF-7, and still there once the server's own
/// listing replaces what was held.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_folder_made_offline_reaches_the_server_after_a_restart() {
    let (port, heard) = serve("never").await;
    let dir = tempfile::tempdir().unwrap();
    {
        let store = open(&dir);
        request(
            &store,
            port,
            FolderWork::Create {
                path: "日本語".to_owned(),
            },
        );
        assert_eq!(
            paths(&store),
            ["日本語"],
            "there at once, before any server"
        );
    }

    // The process ends; a new one opens the same database and drains what was left.
    let store = open(&dir);
    let mut engine = engine(port, store.clone());
    let (_tx, mut cancel) = watch::channel(false);
    let report = engine.drain_outbox(&mut cancel, now()).await.unwrap();
    assert_eq!(report.outbox_settled, 1, "{report:?}");
    assert_eq!(
        heard.lock().unwrap().as_slice(),
        ["CREATE \"&ZeVnLIqe-\"", "SUBSCRIBE \"&ZeVnLIqe-\""]
    );
    assert!(store.outbox_due(ACCOUNT, now()).unwrap().is_empty());

    let listed = engine.refresh_folders(&mut cancel, now()).await.unwrap();
    let names: Vec<(&str, Subscription)> = listed
        .iter()
        .map(|f| (f.path.as_str(), f.subscription))
        .collect();
    assert_eq!(
        names,
        [
            ("INBOX", Subscription::Subscribed),
            ("日本語", Subscription::Subscribed)
        ]
    );
}

/// A permanent `NO` takes the folder back out, and says so.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_folder_the_server_refuses_is_taken_back() {
    let (port, heard) = serve("Forbidden").await;
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir);
    request(
        &store,
        port,
        FolderWork::Create {
            path: "Forbidden".to_owned(),
        },
    );
    assert_eq!(paths(&store), ["Forbidden"]);

    let mut engine = engine(port, store.clone());
    let (_tx, mut cancel) = watch::channel(false);
    let report = engine.drain_outbox(&mut cancel, now()).await.unwrap();

    assert_eq!(paths(&store), Vec::<String>::new(), "the undo ran");
    assert!(store.outbox_due(ACCOUNT, now()).unwrap().is_empty());
    assert!(
        report
            .needs_attention
            .iter()
            .any(|n| n.contains("not allowed")),
        "{report:?}"
    );
    // Only the CREATE: nothing is subscribed to a folder the server would not make.
    assert_eq!(heard.lock().unwrap().as_slice(), ["CREATE \"Forbidden\""]);
}
