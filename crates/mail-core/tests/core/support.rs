//! What more than one of these tests starts from: the account, the clock, a store, a server's
//! capabilities and its password.

use mail_domain::{
    AccountCaps, ArchiveMeans, Condstore, ConnectionBudget, ExpungeMeans, FolderRoles, MoveExt,
    ServerLabels, ServerThreads, Supported, WatchMode,
};
use mail_secrets::AccountSecrets;
use mail_store::SqliteStore;
use porter_core::{Credential, SecretKey, SecretPurpose, SecretText};
use porter_secrets::MemorySecrets;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub use mail_domain::testing::{account, now};

/// A store in memory holding [`account`] at `me@example.test` and nothing else.
pub fn store() -> SqliteStore {
    let store = mail_store::testing::in_memory();
    mail_store::testing::seed_account(&store, account(), "me@example.test");
    store
}

/// A plain IMAP server's capabilities: no labels, threads, extensions or pipelining, one
/// connection, polled every five minutes. Read at [`now`], so a pass at [`now`] finds them fresh
/// and does not ask the server again.
pub fn caps() -> AccountCaps {
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

/// [`account`]'s incoming password, kept in `secrets`.
pub fn with_password(secrets: &MemorySecrets, password: &str) {
    mail_secrets::block_on(secrets.put(
        &SecretKey {
            account: account(),
            purpose: SecretPurpose::IncomingPassword,
        },
        &Credential::Password(SecretText::new(password.to_owned())),
    ))
    .unwrap();
}

/// Every source file of the program under `dir`: its tests (a `tests/` directory, `tests.rs`,
/// `*_tests.rs`) left out.
pub fn sources(dir: &Path, into: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if path.is_dir() {
            if name != "tests" {
                sources(&path, into);
            }
        } else if name.ends_with(".rs") && name != "tests.rs" && !name.ends_with("_tests.rs") {
            into.push(path);
        }
    }
}
