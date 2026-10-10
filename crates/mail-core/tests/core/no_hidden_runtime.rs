//! The libraries start no runtime and read no environment.
//!
//! `scripts/check-boundary.sh` holds the same line for the gate; this is the copy a plain
//! `cargo test -p mail-core` runs. The application owns the one tokio runtime (`mail-app`'s
//! `edge`); `mail-core`, `mail-runtime` and `mail-store` give back futures, and `mail-core` is
//! handed the environment ([`mail_core::Environment`]) instead of reading it.

use std::path::{Path, PathBuf};

/// Every non-test `.rs` file under `dir`: not `tests.rs`, `*_tests.rs` or a `tests` directory.
fn sources(dir: &Path, into: &mut Vec<PathBuf>) {
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

/// `file:line: text` for each code line of the library crate `krate` that holds one of `needles`:
/// comments are not code, and a file's tests begin at its `#[cfg(test)]`.
fn found(krate: &str, needles: &[&str], except: &[&str]) -> Vec<String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(krate)
        .join("src");
    let mut files = Vec::new();
    sources(&root, &mut files);
    let mut out = Vec::new();
    for file in files {
        let shown = file
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if except.contains(&shown.as_str()) {
            continue;
        }
        let text = std::fs::read_to_string(&file).unwrap();
        for (n, line) in text.lines().enumerate() {
            if line.starts_with("#[cfg(test)]") {
                break;
            }
            if line.trim_start().starts_with("//") {
                continue;
            }
            if needles.iter().any(|needle| line.contains(needle)) {
                out.push(format!("{krate}/src/{shown}:{}: {}", n + 1, line.trim()));
            }
        }
    }
    out
}

#[test]
fn no_library_builds_or_blocks_on_a_runtime_of_its_own() {
    let runtime = [
        "new_current_thread",
        "new_multi_thread",
        "Runtime::new(",
        "OnceLock<tokio::runtime::Runtime>",
    ];
    let mut hits = Vec::new();
    for krate in ["mail-core", "mail-runtime", "mail-store"] {
        hits.extend(found(krate, &runtime, &[]));
    }
    hits.extend(found("mail-core", &["block_on("], &[]));
    assert!(
        hits.is_empty(),
        "a library waits on a future itself instead of returning it; the application's edge \
         (crates/mail-app/src/edge.rs) is where that is done:\n{}",
        hits.join("\n")
    );
}

#[test]
fn mail_core_reads_no_environment_variable() {
    let hits = found(
        "mail-core",
        &["env::var(", "env::var_os(", "env::current_exe("],
        // The notification adapter relaunches the program that showed the notification.
        &["notify/desktop.rs"],
    );
    assert!(
        hits.is_empty(),
        "mail-core is handed an Environment (mail_core::Environment) and reads none:\n{}",
        hits.join("\n")
    );
}

#[test]
fn the_link_to_accountd_is_not_a_global() {
    let hits = found(
        "mail-runtime",
        &["link::install(", "link::current(", "static CURRENT"],
        &[],
    );
    assert!(
        hits.is_empty(),
        "the link is handed to mail_core::Mail::new; a process-wide copy is the application's \
         (mail-app's edge):\n{}",
        hits.join("\n")
    );
}
