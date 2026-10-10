//! Every integration test of `mail-runtime`, in one binary.
//!
//! Each `tests/*.rs` used to be a binary of its own, and each of them linked the whole
//! dependency graph again. A file here is a module, so a test's name gains its file's name
//! as a prefix: `cargo test -p mail-runtime --test runtime -- <file>::`.

// The helpers the tests share, declared once.
#[path = "../../../mail-mime/tests/bimi_support/mod.rs"]
mod bimi_support;
#[path = "../jmap_fake/mod.rs"]
mod jmap_fake;
#[path = "../support/relay.rs"]
mod relay;

mod assemble;
mod bimi;
mod carddav;
mod cjk_search;
mod drive;
mod end_to_end;
mod folder_outbox;
mod folders;
mod gmail_labels;
mod graph_read;
mod graph_send;
mod imap_end_to_end;
mod jmap;
mod large_messages;
mod link_bus;
mod live_imap;
mod live_pop3;
mod live_probe;
mod live_smtp;
mod lookup;
mod part_flood_ingest;
mod pgp_learn;
mod queued_moves;
mod reparse;
mod send_later;
mod sent_copy;
mod server_search;
mod sieve;
mod sign_in;
mod submission_end_to_end;
mod tokens;
mod two_accounts;
mod unsubscribe;
mod wkd;

/// The tests that keep a binary of their own, each saying why at its top.
const OWN_BINARY: &[&str] = &["tls_provider.rs"];

/// A new `tests/*.rs` is a new binary, linking everything again: it belongs here as a module.
#[test]
fn every_integration_test_is_in_this_binary() {
    let tests = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let mut stray: Vec<String> = std::fs::read_dir(&tests)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            if path.is_dir() {
                path.join("main.rs").exists()
                    && path.file_name().is_some_and(|name| name != "runtime")
            } else {
                path.extension().is_some_and(|ext| ext == "rs")
                    && path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .is_none_or(|name| !OWN_BINARY.contains(&name))
            }
        })
        .map(|path| path.display().to_string())
        .collect();
    stray.sort();
    assert!(
        stray.is_empty(),
        "a test binary of its own; fold it into tests/runtime/ as a module: {stray:?}"
    );
}
