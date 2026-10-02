//! Seeds a real on-disk store for a live look at the window.
//!
//! `#[ignore]`d: it is not a test, it is a fixture generator for a human (or a probe script) to
//! run the binary against. `MAILO_SEED_DIR` names the data directory the binary will open.
//!
//! It seeds *mail*, and nothing else. A store in some other state — an account added but not yet
//! signed in, say — is made by running `mailo account add` against the directory, not by writing
//! the rows here. A hand-written one of those cost a false finding within five minutes: it left
//! `account_caps` empty, which `account add` never does, and the window then reported "Something
//! has gone wrong with setup" for what looked like the ordinary first-run state. The guard was
//! right and the fixture was lying. See `CONVENTIONS.md`, "An assertion that was already true".

use mail_domain::*;
use mail_runtime::{Arrival, absorb};
use mail_store::{SqliteStore, Store};

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));

#[test]
#[ignore = "fixture generator: set MAILO_SEED_DIR and run with --ignored"]
fn seed() {
    let base = std::path::PathBuf::from(
        std::env::var("MAILO_SEED_DIR").expect("MAILO_SEED_DIR names the data directory"),
    )
    .join("mailo");
    std::fs::create_dir_all(base.join("blobs")).unwrap();
    let store = SqliteStore::open(base.join("mail.db"), base.join("blobs")).unwrap();
    store
        .connection()
        .execute(
            "INSERT OR IGNORE INTO accounts (id, address, plan, created_at)
             VALUES (?1, 'me@example.test', '{}', datetime('now'))",
            [ACCOUNT.to_string()],
        )
        .unwrap();

    let now = chrono::Utc::now();
    for (n, subject) in ["flight to taipei", "the invoice"].iter().enumerate() {
        let raw = format!(
            "From: ada@example.test\r\nTo: me@example.test\r\nSubject: {subject}\r\n\
             Date: Tue, 22 Sep 2026 09:00:00 +0800\r\n\
             Message-ID: <live{n}@example.test>\r\n\r\nbody of {subject}\r\n"
        );
        absorb(
            &store,
            ACCOUNT,
            MailboxRef {
                account: ACCOUNT,
                path: "INBOX".to_owned(),
            },
            Some(SyncCursor::Pop),
            vec![Arrival {
                remote: RemoteRef::Pop {
                    uidl: format!("live{n}"),
                },
                raw: raw.into_bytes(),
            }],
            false,
            now,
        )
        .unwrap();
    }
    // One of them labelled, the way a Gmail sync reports it.
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
                messages: vec![],
                flags: vec![],
                labels: vec![],
                label_names: vec![(
                    RemoteRef::Pop {
                        uidl: "live0".to_owned(),
                    },
                    vec!["travel".to_owned()],
                )],
                gone: vec![],
            },
        )
        .unwrap();
    eprintln!("seeded {}", base.display());
}
