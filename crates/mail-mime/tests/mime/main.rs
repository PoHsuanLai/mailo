//! Every integration test of `mail-mime`, in one binary.
//!
//! Each `tests/*.rs` used to be a binary of its own, and each of them linked the whole
//! dependency graph again. A file here is a module, so a test's name gains its file's name
//! as a prefix: `cargo test -p mail-mime --test mime -- <file>::`.

// The helpers the tests share, declared once.
#[path = "../bimi_support/mod.rs"]
mod bimi_support;
#[path = "../block/mod.rs"]
mod block;
#[path = "../smime_support/mod.rs"]
mod smime_support;

mod auth;
mod bimi;
mod block_adversarial;
mod block_heaviness;
mod block_html;
mod block_proptest;
mod block_quote;
mod block_text;
mod block_tracking;
mod build;
mod imip;
mod inline;
mod mdn;
mod openpgp;
mod parse;
mod print;
mod raw_headers;
mod reconstruct;
mod sanitize;
mod sanitize_adversarial;
mod sanitize_unrendered;
mod smime;
mod smime_openssl;
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
                path.join("main.rs").exists() && path.file_name().is_some_and(|name| name != "mime")
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
        "a test binary of its own; fold it into tests/mime/ as a module: {stray:?}"
    );
}
