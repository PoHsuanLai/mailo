//! `sync::run`, which is the function the binary calls and the one nothing could run.
//!
//! Every layer below this has tests — the session, the backend, the engine, the store — and the
//! assembly had none, because it reached for `KeyringSecrets` directly and a test cannot have a
//! keyring. What is worth checking here *is* the assembly: that a stored plan becomes a working
//! connection, that an account without a credential is skipped with a reason rather than
//! stopping the run, and that what the user is told matches what happened.
//!
//! The IMAP half is `#[ignore]`d because it needs `scripts/live-imapd.py`; everything that does
//! not need a server runs normally.

use chrono::{DateTime, TimeZone, Utc};
use mail_domain::*;
use mail_runtime::{MapSecrets, Secrets};
use mail_store::SqliteStore;
use std::sync::Arc;

#[allow(dead_code)]
#[path = "../src/account.rs"]
mod account;
#[allow(dead_code)]
#[path = "../src/cli.rs"]
mod cli;
#[allow(dead_code)]
#[path = "../src/compose.rs"]
mod compose;
#[path = "../src/sync.rs"]
mod sync;

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));

fn now() -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000, 0).unwrap()
}

/// A store with one IMAP account pointed at `port`, in plaintext.
///
/// The plan is written directly rather than through `account add`, which only produces
/// `Tls::Implicit` — deliberately, since there is no configuration that sends a password in
/// clear. A loopback test server has no certificate a public root would sign, so this is the one
/// place that shape is constructed by hand.
fn configured(port: u16, caps: AccountCaps) -> (Arc<SqliteStore>, tempfile::TempDir) {
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
            rusqlite::params![ACCOUNT.to_string(), serde_json::to_string(&plan).unwrap()],
        )
        .unwrap();
        db.execute(
            "INSERT INTO account_caps (account, caps, observed_at) VALUES (?1, ?2, ?3)",
            rusqlite::params![
                ACCOUNT.to_string(),
                serde_json::to_string(&caps).unwrap(),
                now().to_rfc3339()
            ],
        )
        .unwrap();
    }
    (store, dir)
}

fn caps() -> AccountCaps {
    AccountCaps {
        labels: ServerLabels::LocalOnly,
        threads: ServerThreads::Jwz,
        watch: WatchMode::Poll {
            every: std::time::Duration::from_secs(300),
        },
        archive: ArchiveMeans::LocalOnly,
        folders: FolderRoles(Vec::new()),
        condstore: Condstore::Absent,
        move_ext: MoveExt::Absent,
        expunge: ExpungeMeans::Forbidden,
        top: Supported::Absent,
        pipelining: Supported::Absent,
        connections: ConnectionBudget { max: 1 },
        // Fresh, so the pass does not try to re-read capabilities from a server that may not be
        // there. Staleness is covered in `imap_end_to_end.rs`.
        observed_at: now(),
    }
}

fn with_password(store_secrets: &MapSecrets, password: &str) {
    store_secrets
        .put(
            &SecretKey {
                account: ACCOUNT,
                purpose: SecretPurpose::IncomingPassword,
            },
            &Credential::Password(password.to_owned()),
        )
        .unwrap();
}

#[test]
fn an_account_with_no_credential_is_skipped_with_a_reason() {
    // One account needing attention must not stop the others fetching mail, and the reason has
    // to name the command that fixes it — this is the first thing a new user sees.
    let (store, _dir) = configured(1, caps());
    let out = sync::run_with(store, Arc::new(MapSecrets::default()), now())
        .expect("a missing credential is not a failure of the run");

    assert!(out.contains("ada@example.test"), "{out}");
    assert!(out.contains("no credential stored"), "{out}");
    assert!(
        out.contains("mailo account add"),
        "the way out is named: {out}"
    );
}

#[test]
fn an_unreachable_server_is_reported_per_account_not_thrown() {
    // Port 1 refuses. The pass should say so against that account and return.
    let (store, _dir) = configured(1, caps());
    let secrets = MapSecrets::default();
    with_password(&secrets, "s3cr3t-pass");

    let out = sync::run_with(store, Arc::new(secrets), now())
        .expect("an unreachable server is reported, not returned as an error");
    assert!(out.contains("ada@example.test"), "{out}");
    assert!(
        out.to_lowercase().contains("connect") || out.to_lowercase().contains("refused"),
        "the reason should reach the user: {out}"
    );
}

#[test]
fn no_accounts_explains_how_to_add_one() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::in_memory(dir.path()).unwrap());
    let out = sync::run_with(store, Arc::new(MapSecrets::default()), now()).unwrap();
    assert!(out.contains("mailo account add"), "{out}");
}

#[tokio::test]
#[ignore = "needs a local Twisted IMAP4 server; run deliberately with --ignored"]
async fn a_whole_pass_against_a_real_server_lands_mail_and_says_what_it_did() {
    // The assembly, end to end: stored plan, stored capabilities, a credential, a real server,
    // and a line of output a person reads.
    let (store, _dir) = configured(11143, caps());
    let secrets = MapSecrets::default();
    with_password(&secrets, "s3cr3t-pass");

    let store_for_pass = store.clone();
    let out = tokio::task::spawn_blocking(move || {
        sync::run_with(store_for_pass, Arc::new(secrets), now())
    })
    .await
    .expect("the pass did not panic")
    .expect("a reachable server with a good credential syncs");

    eprintln!("{out}");
    assert!(out.contains("ada@example.test"), "{out}");
    assert!(
        !out.contains("needs attention") && !out.contains("protocol:"),
        "a clean pass should report neither trouble nor a protocol error: {out}"
    );
    assert!(
        out.contains("headers") && out.contains("bodies"),
        "the line a user reads should say what was fetched: {out}"
    );
    // The fixture serves two messages; they must be in the store afterwards.
    let count: i64 = store
        .connection()
        .query_row("SELECT count(*) FROM messages", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 2, "the pass reported success and stored nothing");
}
