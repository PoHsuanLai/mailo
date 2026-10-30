//! Every integration test of `mail-cli`, in one binary: the command line driven as `main` drives it,
//! over a store in a scratch directory. The window's tests are `mail-app`'s.
//!
//! A file here is a module, so a test's name gains its file's name as a prefix:
//! `cargo nextest run -p mail-cli -E 'test(cli::)'`.

// The helpers the tests share, declared once.
#[path = "../support/cli_mail.rs"]
mod cli_mail;
#[path = "../support/sealed.rs"]
mod sealed;
use mail_crypto::testing::smime as smime_support;
// The account the window's tests use too (`mail-app/tests/support/store.rs`).
#[path = "../support/store.rs"]
mod store;

mod cli;
mod cli_watch_notify_offline;
mod folders;
mod import_export;
mod invites;
mod pgp;
mod printing;
mod receipts;
mod rules;
mod send_later;
mod smime;
mod unsubscribe;

#[test]
fn every_integration_test_is_in_this_binary() {
    let tests = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let mut stray: Vec<String> = std::fs::read_dir(&tests)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            if path.is_dir() {
                path.join("main.rs").exists() && path.file_name().is_some_and(|name| name != "app")
            } else {
                path.extension().is_some_and(|ext| ext == "rs")
            }
        })
        .map(|path| path.display().to_string())
        .collect();
    stray.sort();
    assert!(
        stray.is_empty(),
        "a test binary of its own; fold it into tests/app/ as a module: {stray:?}"
    );
}
