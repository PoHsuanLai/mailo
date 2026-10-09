//! Every integration test of `mail-store`, in one binary.
//!
//! Each `tests/*.rs` used to be a binary of its own, and each of them linked the whole
//! dependency graph again. A file here is a module, so a test's name gains its file's name
//! as a prefix: `cargo test -p mail-store --test store -- <file>::`.

mod accounts;
mod concurrency;
mod contacts;
mod destroy;
mod drafts;
mod fold_table;
mod folders;
mod follow_up;
mod found;
mod graph_remote;
mod groups;
mod held;
mod import;
mod in_folder;
mod jmap;
mod labels;
mod offline;
mod parity;
mod pgp_keys;
mod phase3_milestone;
mod queued_moves;
mod reconciliation;
mod remove_account;
mod rules;
mod search;
mod send_later;
mod smime_certs;
mod sqlite_capabilities;
mod templates;
mod two_writers;
mod upgrade;
mod views;

/// The tests that keep a binary of their own, each saying why at its top.
const OWN_BINARY: &[&str] = &["scale.rs"];

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
                    && path.file_name().is_some_and(|name| name != "store")
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
        "a test binary of its own; fold it into tests/store/ as a module: {stray:?}"
    );
}
