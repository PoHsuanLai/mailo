//! `sync::fetch_body_with`: one message's body, on demand, for a reader that opened it before a
//! sync reached it.
//!
//! The IMAP server is a scripted one on loopback, in this process, so the test is fast and needs
//! no daemon: it answers just enough of the protocol for one `UID FETCH`.

use chrono::{DateTime, TimeZone, Utc};
use mail_core::sync;
use mail_domain::id::account_id_from_uuid;
use mail_domain::*;
use mail_runtime::{AccountSecrets, ClientRegistry};
use mail_store::SqliteStore;
use porter_core::SecretText;
use porter_core::{AccountId, Credential, SecretKey, SecretPurpose};
use porter_secrets::MemorySecrets;
use std::io::{BufRead as _, BufReader, Write as _};
use std::net::TcpListener;
use std::sync::Arc;
use std::time::Duration;

fn acct_account() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000b1"))
}

const RAW: &[u8] = b"From: a@example.test\r\nSubject: hello\r\nMessage-ID: <b1@example.test>\r\n\r\nthe body text\r\n";

fn now() -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000, 0).unwrap()
}

fn caps() -> AccountCaps {
    AccountCaps {
        labels: ServerLabels::LocalOnly,
        threads: ServerThreads::Jwz,
        watch: WatchMode::Poll {
            every: Duration::from_secs(300),
        },
        archive: ArchiveMeans::LocalOnly,
        folders: FolderRoles(Vec::new()),
        condstore: Condstore::Absent,
        move_ext: MoveExt::Absent,
        expunge: ExpungeMeans::Forbidden,
        top: Supported::Absent,
        pipelining: Supported::Absent,
        connections: ConnectionBudget { max: 1 },
        observed_at: now(),
    }
}

/// An IMAP account on `port` holding one message, headers only, and its id.
fn account_with_header_only_message(port: u16) -> (Arc<SqliteStore>, MessageId, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::in_memory(dir.path()).unwrap());
    let plan = AccountPlan {
        address: "ada@example.test".to_owned(),
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
    {
        let db = store.connection();
        db.execute(
            "INSERT INTO accounts (id, address, plan, created_at)
             VALUES (?1, 'ada@example.test', ?2, datetime('now'))",
            rusqlite::params![
                acct_account().to_string(),
                serde_json::to_string(&plan).unwrap()
            ],
        )
        .unwrap();
        db.execute(
            "INSERT INTO account_caps (account, caps, observed_at) VALUES (?1, ?2, ?3)",
            rusqlite::params![
                acct_account().to_string(),
                serde_json::to_string(&caps()).unwrap(),
                now().to_rfc3339()
            ],
        )
        .unwrap();
    }
    mail_runtime::absorb(
        &store,
        acct_account(),
        MailboxRef {
            account: acct_account(),
            path: "INBOX".to_owned(),
        },
        None,
        vec![mail_runtime::Arrival {
            remote: RemoteRef::Imap {
                mailbox: "INBOX".to_owned(),
                uidvalidity: 7,
                uid: 5,
            },
            raw: RAW.to_vec(),
        }],
        true,
        now(),
    )
    .unwrap();
    let id: String = store
        .connection()
        .query_row("SELECT id FROM messages", [], |r| r.get(0))
        .unwrap();
    let id = MessageId::from_uuid(id.parse().unwrap());
    (store, id, dir)
}

/// Answers one connection: every command succeeds, `SELECT` reports validity 7, and
/// `UID FETCH` returns [`RAW`] as UID 5.
fn serve_one() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        let Ok((stream, _)) = listener.accept() else {
            return;
        };
        let mut out = stream.try_clone().unwrap();
        out.write_all(b"* OK ready\r\n").unwrap();
        for line in BufReader::new(stream).lines() {
            let Ok(line) = line else { return };
            let (tag, command) = line.split_once(' ').unwrap_or((&line, ""));
            let command = command.to_ascii_uppercase();
            let reply: Vec<u8> = if command.starts_with("CAPABILITY") {
                format!("* CAPABILITY IMAP4rev1\r\n{tag} OK done\r\n").into_bytes()
            } else if command.starts_with("SELECT") || command.starts_with("EXAMINE") {
                format!("* 1 EXISTS\r\n* OK [UIDVALIDITY 7] ok\r\n{tag} OK [READ-ONLY] done\r\n")
                    .into_bytes()
            } else if command.starts_with("UID FETCH") {
                let mut r = format!("* 1 FETCH (UID 5 BODY[] {{{}}}\r\n", RAW.len()).into_bytes();
                r.extend_from_slice(RAW);
                r.extend_from_slice(format!(")\r\n{tag} OK done\r\n").as_bytes());
                r
            } else {
                format!("{tag} OK done\r\n").into_bytes()
            };
            if out.write_all(&reply).is_err() {
                return;
            }
        }
    });
    port
}

/// The body text the store holds for `id`, if any.
fn held_body(store: &SqliteStore, id: MessageId) -> Option<String> {
    let raw: Option<String> = store
        .connection()
        .query_row(
            "SELECT body_text FROM messages WHERE id = ?1",
            [id.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    raw
}

fn with_password(secrets: &MemorySecrets) {
    mail_runtime::block_on(secrets.put(
        &SecretKey {
            account: acct_account(),
            purpose: SecretPurpose::IncomingPassword,
        },
        &Credential::Password(SecretText::new("s3cr3t-pass".to_owned())),
    ))
    .unwrap();
}

#[test]
fn a_body_fetched_on_demand_is_stored_for_the_reader() {
    let (store, id, _dir) = account_with_header_only_message(serve_one());
    assert!(held_body(&store, id).is_none(), "seeded with a body");
    let secrets = MemorySecrets::default();
    with_password(&secrets);

    sync::fetch_body_with(
        store.clone(),
        Arc::new(secrets),
        &ClientRegistry::default(),
        id,
        now(),
    )
    .expect("a reachable server with a good credential delivers the body");

    let body = held_body(&store, id).expect("the body was stored");
    assert!(body.contains("the body text"), "{body:?}");
}

#[test]
fn a_missing_credential_asks_for_a_new_sign_in() {
    let (store, id, _dir) = account_with_header_only_message(1);
    let (retry, why) = sync::fetch_body_with(
        store,
        Arc::new(MemorySecrets::default()),
        &ClientRegistry::default(),
        id,
        now(),
    )
    .unwrap_err();
    assert_eq!(retry, Retry::NeedsReauth, "{why}");
    assert!(why.contains("mailo account add"), "{why}");
}

#[test]
fn an_unreachable_server_can_be_tried_again_later() {
    // Port 1 refuses.
    let (store, id, _dir) = account_with_header_only_message(1);
    let secrets = MemorySecrets::default();
    with_password(&secrets);
    let (retry, why) = sync::fetch_body_with(
        store.clone(),
        Arc::new(secrets),
        &ClientRegistry::default(),
        id,
        now(),
    )
    .unwrap_err();
    assert!(matches!(retry, Retry::After(_)), "{retry:?}: {why}");
    assert!(held_body(&store, id).is_none());
}
