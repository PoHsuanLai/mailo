//! The list's own words: each row's second line and its icon.

use super::Origin;
use super::log::Entry;
use super::panel::{detail, icon_of};
use chrono::{DateTime, Utc};
use ds::prelude::Icon;
use mail_domain::ThreadId;
use std::path::PathBuf;

fn at(hours: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(1_700_000_000, 0).unwrap_or_else(|| panic!("a valid timestamp"))
        + chrono::Duration::hours(hours)
}

fn entry(origin: Option<Origin>) -> Entry {
    Entry {
        path: PathBuf::from("/saves/minutes.pdf"),
        at: at(0),
        bytes: 2048,
        origin,
    }
}

/// A row's second line: its size, when, and the message it came from; a file the window saved
/// of its own names no message; and a file no longer there says so.
#[test]
fn a_rows_second_line() {
    let origin = Origin {
        thread: ThreadId::generate(),
        subject: "  Minutes of the board ".to_owned(),
    };
    let size = mail_core::attach::human_size(2048);
    let when = crate::ui::menus::when_words(at(0), at(1), &chrono::Local);
    let cases = [
        (
            "from a message",
            Some(origin),
            true,
            format!("{size} \u{b7} {when} \u{b7} Minutes of the board"),
        ),
        (
            "the window's own",
            None,
            true,
            format!("{size} \u{b7} {when}"),
        ),
        (
            "no longer there",
            None,
            false,
            "Moved or deleted".to_owned(),
        ),
    ];
    for (name, origin, there, want) in cases {
        assert_eq!(detail(&entry(origin), there, at(1)), want, "{name}");
    }
}

#[test]
fn a_picture_has_the_picture_icon_and_anything_else_a_page() {
    assert_eq!(icon_of("Photo.JPG"), Icon::Image);
    assert_eq!(icon_of("minutes.pdf"), Icon::File);
    assert_eq!(icon_of("README"), Icon::File);
}
