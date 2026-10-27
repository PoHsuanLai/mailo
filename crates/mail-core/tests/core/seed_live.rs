//! Seeds a real on-disk store for a live look at the window.
//!
//! `#[ignore]`d: it is not a test, it is a fixture generator for a human (or a probe script) to
//! run the binary against. `MAILO_SEED_DIR` names the data directory the binary will open.
//!
//! It seeds *mail*, and nothing else. A store in some other state — an account added but not yet
//! signed in, say — is made by running `mailo account add` against the directory, not by writing
//! the rows here. A hand-written one is a state the application cannot reach: it can leave
//! `account_caps` empty, which `account add` never does, and the window then rightly reports a
//! setup failure for what looks like the ordinary first-run state.

use crate::support::account;
use mail_domain::*;
use mail_runtime::{Arrival, absorb};
use mail_store::{SqliteStore, Store};

#[test]
#[ignore = "fixture generator: set MAILO_SEED_DIR and run with --ignored"]
fn seed() {
    let base = std::path::PathBuf::from(
        std::env::var("MAILO_SEED_DIR").expect("MAILO_SEED_DIR names the data directory"),
    )
    .join("mailo");
    std::fs::create_dir_all(base.join("blobs")).unwrap();
    let store = SqliteStore::open(base.join("mail.db"), base.join("blobs")).unwrap();
    mail_store::testing::seed_account(&store, account(), "me@example.test");

    let now = chrono::Utc::now();
    for (n, subject) in ["flight to taipei", "the invoice"].iter().enumerate() {
        let raw = format!(
            "From: ada@example.test\r\nTo: me@example.test\r\nSubject: {subject}\r\n\
             Date: Tue, 22 Sep 2026 09:00:00 +0800\r\n\
             Message-ID: <live{n}@example.test>\r\n\r\nbody of {subject}\r\n"
        );
        absorb(
            &store,
            account(),
            MailboxRef {
                account: account(),
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
            account(),
            Ingest {
                mailbox: MailboxRef {
                    account: account(),
                    path: "INBOX".to_owned(),
                },
                validity: UidValidity::Same,
                cursor: None,
                messages: vec![],
                flags: vec![],
                flag_colors: Vec::new(),
                labels: vec![],
                label_names: vec![(
                    RemoteRef::Pop {
                        uidl: "live0".to_owned(),
                    },
                    vec!["travel".to_owned()],
                )],
                provider_threads: Vec::new(),
                gone: vec![],
            },
        )
        .unwrap();
    eprintln!("seeded {}", base.display());
}
