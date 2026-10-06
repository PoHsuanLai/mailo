//! Seeds the account `dev/scenarios` runs the real `mailo` against: one IMAP account on the local
//! fake server (`scripts/live-imapd.py`), in plaintext, holding its password in the scenario's
//! secrets directory (debug builds only, `MAILO_TEST_SECRETS_DIR`), and nothing else.
//!
//! `#[ignore]`d: it is a fixture generator for a script, not a test. `MAILO_SEED_DIR` names the
//! data home the binary will open (`$XDG_DATA_HOME`), `MAILO_SCENARIO_IMAP_PORT` the port the
//! fake server listens on. The plan is written by hand, as `sync_path.rs` does, because
//! `mailo account add` produces only TLS and a loopback server has no certificate a public root
//! would sign; the capabilities are the ones a pass records for a server that offers IDLE, so the
//! watch holds a connection open and hears new mail pushed rather than polling every five minutes.

use mail_domain::id::account_id_from_uuid;
use mail_domain::*;
use mail_runtime::{KeyringSecrets, Secrets};
use mail_store::SqliteStore;
use porter_core::SecretText;
use porter_core::{AccountId, Credential, SecretKey, SecretPurpose};

fn acct_account() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000c1"))
}

#[test]
#[ignore = "fixture generator: set MAILO_SEED_DIR, MAILO_SCENARIO_IMAP_PORT and MAILO_TEST_SECRETS_DIR"]
fn seed() {
    let base = std::path::PathBuf::from(
        std::env::var("MAILO_SEED_DIR").expect("MAILO_SEED_DIR names the data home"),
    )
    .join("mailo");
    let port: u16 = std::env::var("MAILO_SCENARIO_IMAP_PORT")
        .expect("MAILO_SCENARIO_IMAP_PORT names the fake IMAP server's port")
        .parse()
        .expect("a port number");
    assert!(
        std::env::var_os("MAILO_TEST_SECRETS_DIR").is_some(),
        "without MAILO_TEST_SECRETS_DIR the password would go to the real keyring"
    );
    std::fs::create_dir_all(base.join("blobs")).unwrap();
    let store = SqliteStore::open(base.join("mail.db"), base.join("blobs")).unwrap();
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
    let caps = AccountCaps {
        labels: ServerLabels::LocalOnly,
        threads: ServerThreads::Jwz,
        watch: WatchMode::Idle,
        archive: ArchiveMeans::LocalOnly,
        folders: FolderRoles(Vec::new()),
        condstore: Condstore::Absent,
        move_ext: MoveExt::Absent,
        expunge: ExpungeMeans::Forbidden,
        top: Supported::Absent,
        pipelining: Supported::Absent,
        connections: ConnectionBudget { max: 1 },
        observed_at: chrono::Utc::now(),
    };
    {
        let db = store.connection();
        db.execute(
            "INSERT OR REPLACE INTO accounts (id, address, plan, created_at)
             VALUES (?1, 'ada@example.test', ?2, datetime('now'))",
            rusqlite::params![
                acct_account().to_string(),
                serde_json::to_string(&plan).unwrap()
            ],
        )
        .unwrap();
        db.execute(
            "INSERT OR REPLACE INTO identities (id, account, from_name, from_email, is_default)
             VALUES (?1, ?2, NULL, 'ada@example.test', '\"default\"')",
            [
                IdentityId::generate().to_string(),
                acct_account().to_string(),
            ],
        )
        .unwrap();
        db.execute(
            "INSERT OR REPLACE INTO account_caps (account, caps, observed_at)
             VALUES (?1, ?2, datetime('now'))",
            rusqlite::params![
                acct_account().to_string(),
                serde_json::to_string(&caps).unwrap()
            ],
        )
        .unwrap();
    }
    KeyringSecrets
        .put(
            &SecretKey {
                account: acct_account(),
                purpose: SecretPurpose::IncomingPassword,
            },
            &Credential::Password(SecretText::new("s3cr3t-pass".to_owned())),
        )
        .unwrap();
}
