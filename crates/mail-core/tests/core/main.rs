//! Every integration test of `mail-core`, in one binary.
//!
//! Each `tests/*.rs` used to be a binary of its own, and each of them linked the whole
//! dependency graph again. A file here is a module, so a test's name gains its file's name
//! as a prefix: `cargo test -p mail-core --test core -- <file>::`.

mod attachments;
mod blocking;
mod fetch_body;
mod folder_sync;
mod held_while_linked;
mod ipc;
mod live_presets;
mod live_watch;
mod notifications;
mod offline_sync;
mod search_scale;
mod seed_live;
mod sender_standing;
mod sender_trust;
mod sync_path;

/// The tests that keep a binary of their own, each saying why at its top.
const OWN_BINARY: &[&str] = &["seed_scenario.rs"];

/// A new `tests/*.rs` is a new binary, linking everything again: it belongs here as a module.
#[test]
fn every_integration_test_is_in_this_binary() {
    let tests = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let mut stray: Vec<String> = std::fs::read_dir(&tests)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            if path.is_dir() {
                path.join("main.rs").exists() && path.file_name().is_some_and(|name| name != "core")
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
        "a test binary of its own; fold it into tests/core/ as a module: {stray:?}"
    );
}
