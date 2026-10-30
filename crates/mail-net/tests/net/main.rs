//! Every integration test of `mail-net`, in one binary.
//!
//! A file here is a module, so a test's name gains its file's name as a prefix:
//! `cargo nextest run -p mail-net -E 'test(wkd::)'`.

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
                path.join("main.rs").exists() && path.file_name().is_some_and(|name| name != "net")
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
        "a test binary of its own; fold it into tests/net/ as a module: {stray:?}"
    );
}
