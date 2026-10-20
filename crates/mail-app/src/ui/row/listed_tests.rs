//! What a row says, its menu and what a click on it opens, for each kind of row.

use super::listed::{Listed, Opens};
use super::menu::{self, Pick};
use crate::ui::copy;
use crate::ui::list_search::RowHit;
use crate::ui::selection::Click;
use chrono::{DateTime, TimeZone, Utc};
use ds::components::app::thread_height::ThreadLines;
use ds::components::menus::item::item::MenuItem;
use ds::prelude::*;
use mail_domain::*;

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 19, 12, 0, 0).unwrap()
}

fn summary(snippet: &str, read: ReadState) -> ThreadSummary {
    ThreadSummary {
        subject: "Offsite plans".into(),
        snippet: snippet.into(),
        from: Address {
            name: Some("Ada Lovelace".into()),
            email: "ada@example.test".into(),
        },
        last_date: now(),
        read,
        ..mail_domain::testing::summary()
    }
}

fn draft(subject: &str, to: &[&str], state: SendState) -> Draft {
    let identity = mail_domain::testing::identity();
    Draft {
        subject: subject.into(),
        to: to
            .iter()
            .map(|email| Address {
                name: None,
                email: (*email).into(),
            })
            .collect(),
        state,
        ..Draft::blank(&identity, now())
    }
}

fn picks(items: &[MenuItem<Pick>]) -> Vec<Pick> {
    items
        .iter()
        .filter_map(|item| match item {
            MenuItem::Item { value, .. } => Some(*value),
            _ => None,
        })
        .collect()
}

/// The lines a row draws: a snippet's line only when snippets are shown and there is one to
/// show; a draft's state on its third line; a held message's time where the time is, and no
/// third line.
#[test]
fn each_row_draws_the_lines_it_has() {
    let at = Utc.with_ymd_and_hms(2026, 10, 20, 9, 0, 0).unwrap();
    let cases: &[(Listed, bool, ThreadLines, &str)] = &[
        (
            Listed::Thread(summary("Three blockers left", ReadState::Read)),
            true,
            ThreadLines::Three,
            "Ada Lovelace",
        ),
        (
            Listed::Thread(summary("", ReadState::Read)),
            true,
            ThreadLines::Two,
            "Ada Lovelace",
        ),
        (
            Listed::Thread(summary("Three blockers left", ReadState::Read)),
            false,
            ThreadLines::Two,
            "Ada Lovelace",
        ),
        (
            Listed::Draft(draft("Notes", &["bea@example.test"], SendState::Editing)),
            true,
            ThreadLines::Three,
            "bea@example.test",
        ),
        (
            Listed::Draft(draft("Notes", &[], SendState::Editing)),
            false,
            ThreadLines::Two,
            copy::empty::NO_RECIPIENT,
        ),
        (
            Listed::Draft(draft(
                "Notes",
                &["bea@example.test"],
                SendState::Scheduled { at },
            )),
            true,
            ThreadLines::Two,
            "bea@example.test",
        ),
    ];
    for (listed, snippets, lines, name) in cases {
        let row = listed.row(None, *snippets, now(), &Utc, mail_core::when::Clock::H24);
        assert_eq!(
            (row.lines, row.name.as_str()),
            (*lines, *name),
            "{listed:?}"
        );
        assert_eq!(
            row.snippet.is_some(),
            *lines == ThreadLines::Three,
            "{listed:?}"
        );
    }
    let held = Listed::Draft(draft("", &[], SendState::Scheduled { at }));
    let row = held.row(None, true, now(), &Utc, mail_core::when::Clock::H24);
    assert_eq!(
        row.time,
        crate::ui::menus::when_words(at, now(), &Utc, mail_core::when::Clock::H24)
    );
    assert_eq!(row.label, copy::aria::open_row(copy::empty::NO_SUBJECT));
    assert_eq!(held.held(), Some(at));
}

/// An unread conversation is drawn strong, a read one and a draft plain; a search marks both
/// lines.
#[test]
fn a_row_is_drawn_from_its_state_and_its_search() {
    let unread = Listed::Thread(summary("Three blockers left", ReadState::Unread));
    assert_eq!(
        unread
            .row(None, true, now(), &Utc, mail_core::when::Clock::H24)
            .emphasis,
        Emphasis::Strong
    );
    let read = Listed::Thread(summary("Three blockers left", ReadState::Read));
    assert_eq!(
        read.row(None, true, now(), &Utc, mail_core::when::Clock::H24)
            .emphasis,
        Emphasis::Plain
    );
    let drafted = Listed::Draft(draft("Notes", &[], SendState::Editing));
    assert_eq!(
        drafted
            .row(None, true, now(), &Utc, mail_core::when::Clock::H24)
            .emphasis,
        Emphasis::Plain
    );

    let hit = RowHit {
        subject: std::iter::once(0..7).collect(),
        snippet: "blockers left".into(),
        snippet_marks: std::iter::once(0..8).collect(),
    };
    let row = read.row(Some(&hit), true, now(), &Utc, mail_core::when::Clock::H24);
    assert!(
        matches!(row.subject, TextLine::Runs(_)),
        "{:?}",
        row.subject
    );
    assert!(
        matches!(row.snippet, Some(TextLine::Runs(_))),
        "{:?}",
        row.snippet
    );
}

/// A conversation's menu is its actions; a draft's is Delete Draft; a held message's is Cancel
/// Sending alone.
#[test]
fn each_row_has_its_own_menu() {
    let thread = summary("", ReadState::Read);
    let listed = Listed::Thread(thread.clone());
    assert_eq!(
        listed.menu(now()),
        menu::rows(&menu::groups(&thread), &thread, now())
    );
    let editing = Listed::Draft(draft("Notes", &[], SendState::Editing));
    assert_eq!(picks(&editing.menu(now())), [Pick::DeleteDraft]);
    let held = Listed::Draft(draft("Notes", &[], SendState::Scheduled { at: now() }));
    assert_eq!(picks(&held.menu(now())), [Pick::CancelSending]);
}

/// A click opens the row: a conversation as the click asks, a draft whatever is held with it.
#[test]
fn a_click_opens_the_row() {
    let thread = summary("", ReadState::Read);
    let listed = Listed::Thread(thread.clone());
    for click in [Click::Plain, Click::Toggle, Click::Range] {
        assert_eq!(listed.open(click), Opens::Thread(thread.id, click));
    }
    let drafted = draft("Notes", &[], SendState::Scheduled { at: now() });
    let id = drafted.id;
    let listed = Listed::Draft(drafted);
    for click in [Click::Plain, Click::Toggle, Click::Range] {
        assert_eq!(listed.open(click), Opens::Draft(id));
    }
}
