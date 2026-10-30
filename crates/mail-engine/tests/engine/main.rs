//! Every integration test of `mail-engine`, in one binary.
//!
//! These drive an `AccountEngine` (or the scheduler over it) against servers in this process: the
//! IMAP, POP3, SMTP and Graph conversations end to end, through `mail-runtime`'s drivers. A file
//! here is a module, so a test's name gains its file's name as a prefix:
//! `cargo nextest run -p mail-engine -E 'test(imap_end_to_end::)'`.

// The helpers the tests share, declared once.
mod fixture;

mod end_to_end;
mod folder_outbox;
mod graph_read;
mod graph_send;
mod imap_end_to_end;
mod large_messages;
mod queued_moves;
mod send_later;
mod sent_copy;
mod server_drafts;
mod server_search;
mod silent_server;
mod submission_end_to_end;
mod tokens;

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
                    && path.file_name().is_some_and(|name| name != "engine")
            } else {
                path.extension().is_some_and(|ext| ext == "rs")
            }
        })
        .map(|path| path.display().to_string())
        .collect();
    stray.sort();
    assert!(
        stray.is_empty(),
        "a test binary of its own; fold it into tests/engine/ as a module: {stray:?}"
    );
}
