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

#[test]
fn a_row_says_its_size_when_and_where_it_came_from() {
    let origin = Origin {
        thread: ThreadId::generate(),
        subject: "  Minutes of the board ".to_owned(),
    };
    let said = detail(&entry(Some(origin)), true, at(1));
    let when = crate::ui::menus::when_words(at(0), at(1), &chrono::Local);
    assert_eq!(
        said,
        format!(
            "{} \u{b7} {when} \u{b7} Minutes of the board",
            mail_core::attach::human_size(2048)
        )
    );
}

#[test]
fn a_file_of_the_window_s_own_names_no_message() {
    let said = detail(&entry(None), true, at(1));
    assert_eq!(said.matches('\u{b7}').count(), 1, "{said}");
}

#[test]
fn a_file_no_longer_there_says_so() {
    assert_eq!(detail(&entry(None), false, at(1)), "Moved or deleted");
}

#[test]
fn a_picture_has_the_picture_icon_and_anything_else_a_page() {
    assert_eq!(icon_of("Photo.JPG"), Icon::Image);
    assert_eq!(icon_of("minutes.pdf"), Icon::File);
    assert_eq!(icon_of("README"), Icon::File);
}
