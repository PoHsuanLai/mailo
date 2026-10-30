//! Every integration test of `mail-crypto`, in one binary.
//!
//! A file here is a module, so a test's name gains its file's name as a prefix:
//! `cargo test -p mailo-crypto --test crypto -- <file>::`.

// The helpers the tests share, declared once.
#[path = "../support/mod.rs"]
mod support;

mod openpgp;
mod smime;
mod smime_openssl;
mod vmc;

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
                    && path.file_name().is_some_and(|name| name != "crypto")
            } else {
                path.extension().is_some_and(|ext| ext == "rs")
            }
        })
        .map(|path| path.display().to_string())
        .collect();
    stray.sort();
    assert!(
        stray.is_empty(),
        "a test binary of its own; fold it into tests/crypto/ as a module: {stray:?}"
    );
}
