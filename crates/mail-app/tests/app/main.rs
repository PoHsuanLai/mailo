//! Every integration test of `mail-app`, in one binary.
//!
//! Each `tests/*.rs` used to be a binary of its own, and each of them linked the whole
//! dependency graph again. A file here is a module, so a test's name gains its file's name
//! as a prefix: `cargo test -p mail-app --test app -- <file>::`.

// The helpers the tests share, declared once.
#[path = "../support/drive.rs"]
mod drive;
#[path = "../support/latency_inbox.rs"]
mod latency_inbox;
#[path = "../support/row_menu.rs"]
mod row_menu;
#[path = "../support/settle.rs"]
mod settle;
#[path = "../../../mail-mime/tests/smime_support/mod.rs"]
mod smime_support;
#[path = "../support/typing.rs"]
mod typing;

mod cli;
mod cli_watch_notify_offline;
mod compose;
mod folders;
mod frame_budget;
mod import_export;
mod invites;
mod lanes;
mod launcher;
mod native_account_pane;
mod native_add_account;
mod native_brand;
mod native_compose_align;
mod native_control_sizes;
mod native_copy;
mod native_desktop_appearance;
mod native_destroy;
mod native_drop;
mod native_emoji;
mod native_fetch;
mod native_follow_up;
mod native_frame;
mod native_harness;
mod native_join;
mod native_keyboard;
mod native_latency;
mod native_launcher;
mod native_layout;
mod native_marks;
mod native_mute;
mod native_original;
mod native_preview;
mod native_reader_scroll;
mod native_selection;
mod native_sender;
mod native_server_search;
mod native_spaces;
mod native_spelling;
mod native_views;
mod native_window;
mod pgp;
mod printing;
mod receipts;
mod rules;
mod screens;
mod searching;
mod send_later;
mod smime;
mod snoozing;
mod unsubscribe;

/// The tests that keep a binary of their own, each saying why at its top.
const OWN_BINARY: &[&str] = &[];

/// A new `tests/*.rs` is a new binary, linking everything again: it belongs here as a module.
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
        "a test binary of its own; fold it into tests/app/ as a module: {stray:?}"
    );
}
