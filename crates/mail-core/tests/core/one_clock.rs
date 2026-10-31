//! One writer for the hours of the day.
//!
//! The snooze menu wrote twelve hours while every other surface wrote twenty-four, each with a
//! chrono pattern of its own. Every time of day a person reads now goes through
//! `mail_logic::when::time_of_day`, which takes the person's clock; this keeps a surface from
//! writing an hour by itself again. The files below write or read hours for machines, not for
//! people, and say so.

use crate::support::sources;
use std::path::Path;

/// The files that may name an hour pattern, and why.
const ALLOWED: &[(&str, &str)] = &[
    ("mail-core/src/when.rs", "the one writer: time_of_day"),
    (
        "mail-reader/src/print.rs",
        "below mail-core, so it cannot call time_of_day; told the clock as a bool, and \
         mail-core's print tests hold the two to the same words",
    ),
    (
        "mail-core/src/snooze.rs",
        "reads a typed `2026-10-08 09:00`",
    ),
    (
        "mail-core/src/rules/server.rs",
        "reads a typed vacation instant",
    ),
    (
        "mail-ui/src/ui/rules/away.rs",
        "writes a kept instant back into a field that reads it again",
    ),
    (
        "mail-store/src/sqlite/accounts.rs",
        "reads SQLite's own datetime text",
    ),
    (
        "mail-proto/src/automatic_replies.rs",
        "Graph's `dateTimeTimeZone`, a wire format",
    ),
    (
        "mail-archive/src/mbox.rs",
        "the mbox `From ` line, a file format",
    ),
    ("mail-proto/src/imap.rs", "IMAP's date-time (RFC 3501)"),
    (
        "mail-proto/src/sieve/script.rs",
        "Sieve's date values (RFC 5260)",
    ),
];

/// chrono's hour patterns: `%H:%M` and every one that writes a twelve-hour clock or a whole
/// time. Each but the first must not run on into a letter or digit, so `%PDF` is not `%P`.
const PATTERNS: &[&str] = &[
    "%H:%M", "%I", "%-I", "%l", "%p", "%P", "%r", "%R", "%T", "%X", "%k",
];

/// Whether `line` names one of [`PATTERNS`].
fn names_an_hour(line: &str) -> bool {
    PATTERNS.iter().any(|pattern| {
        line.match_indices(pattern).any(|(at, _)| {
            *pattern == "%H:%M"
                || !line[at + pattern.len()..]
                    .chars()
                    .next()
                    .is_some_and(|next| next.is_ascii_alphanumeric())
        })
    })
}

#[test]
fn every_hour_a_person_reads_is_written_by_time_of_day() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut hits = Vec::new();
    for krate in std::fs::read_dir(&crates).unwrap() {
        let src = krate.unwrap().path().join("src");
        if !src.is_dir() {
            continue;
        }
        let mut files = Vec::new();
        sources(&src, &mut files);
        for file in files {
            let shown = file
                .strip_prefix(&crates)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if ALLOWED.iter().any(|(allowed, _)| *allowed == shown) {
                continue;
            }
            let text = std::fs::read_to_string(&file).unwrap();
            for (n, line) in text.lines().enumerate() {
                // A file's tests begin at its `#[cfg(test)]`; comments are not code.
                if line.starts_with("#[cfg(test)]") {
                    break;
                }
                if !line.trim_start().starts_with("//") && names_an_hour(line) {
                    hits.push(format!("{shown}:{}: {}", n + 1, line.trim()));
                }
            }
        }
    }
    assert!(
        hits.is_empty(),
        "an hour written by a pattern of its own; write it with \
         mail_logic::when::time_of_day and the person's Clock:\n{}",
        hits.join("\n")
    );
}

#[test]
fn the_allowed_files_still_name_an_hour() {
    // A file that no longer needs its exception loses it, so the list only shrinks.
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    for (file, why) in ALLOWED {
        let text = std::fs::read_to_string(crates.join(file)).unwrap();
        assert!(
            text.lines().any(names_an_hour),
            "{file} ({why}) names no hour pattern now: take it off the list"
        );
    }
}

#[test]
fn a_pattern_is_told_from_a_word_that_starts_like_one() {
    assert!(names_an_hour(r#"at.format("%-I:%M %p")"#));
    assert!(names_an_hour(r#"at.format("%a %H:%M")"#));
    assert!(!names_an_hour(r#"b"%PDF-1.7""#));
    assert!(!names_an_hour(r#"format!("{}%", done)"#));
}
