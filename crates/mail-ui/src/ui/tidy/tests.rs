use super::{Age, candidates};
use crate::ui::app::App;
use crate::ui::clock::WallClock;
use crate::ui::fixtures::{
    INSIDE_THE_SHELL, PRIMARY, chord, click, dispatching, pick_named, rebuild_into, settle,
    thread_like, work,
};
use chrono::{TimeDelta, Utc};
use dioxus::prelude::*;
use mail_core::{SqliteStore, Store};
use mail_domain::*;
use std::sync::Arc;

/// The fixture's one read conversation that nobody starred: Sam's notes are read but starred.
const ACCEPTED: &str = "Accepted: Design review, Thursday 14:00";

fn subjects(store: &Arc<SqliteStore>, scope: Option<Filter>, age: Age) -> Vec<String> {
    let later = Utc::now() + TimeDelta::days(30);
    candidates(&crate::ui::fixtures::client(store), scope, age, later)
        .into_iter()
        .map(|summary| summary.subject)
        .collect()
}

fn in_inbox(store: &SqliteStore, thread: ThreadId) -> bool {
    store
        .thread(thread)
        .unwrap()
        .summary
        .mailboxes
        .contains(MailboxRole::Inbox)
}

/// Read and old enough is offered; unread, starred, or pinned is not, and neither is anything
/// younger than the age chosen.
#[test]
fn only_old_read_conversations_nobody_marked_are_offered() {
    let built = work();
    let store = built.store.clone();
    assert_eq!(subjects(&store, None, Age::OneWeek), [ACCEPTED]);
    // Thirty days on, nothing in the fixture is three months old.
    assert!(subjects(&store, None, Age::ThreeMonths).is_empty());
    let accepted = thread_like(&store, "Accepted");
    assert!(mail_core::snooze::pin(&store, accepted, Utc::now()).unwrap_or(false));
    assert!(
        subjects(&store, None, Age::OneWeek).is_empty(),
        "a pinned conversation is offered"
    );
}

/// Another Space's accounts are not tidied.
#[test]
fn only_the_spaces_accounts_are_offered() {
    let built = work();
    let elsewhere = Filter::Account(mail_domain::id::new_account_id());
    assert!(subjects(&built.store, Some(elsewhere), Age::OneWeek).is_empty());
}

/// From the sidebar's menu, the sheet counts what its button archives; the button archives
/// exactly that, and ⌘Z brings it back.
#[tokio::test]
async fn the_sheet_archives_what_it_lists_and_undo_restores_it() {
    dispatching();
    let built = work();
    let later = Utc::now() + TimeDelta::days(30);
    let mut dom = VirtualDom::new(App)
        .with_root_context(crate::ui::fixtures::client(&built.store))
        .with_root_context(built.dirs.clone())
        .with_root_context(WallClock::new(move || later));
    let seen = rebuild_into(&mut dom);
    let accepted = thread_like(&built.store, "Accepted");
    let listed: Vec<ThreadId> = candidates(
        &crate::ui::fixtures::client(&built.store),
        None,
        Age::default(),
        later,
    )
    .into_iter()
    .map(|summary| summary.id)
    .collect();
    assert_eq!(listed, [accepted]);

    let opened = click(
        &mut dom,
        seen.one("aria-label", mail_ui_model::copy::aria::SIDEBAR_MENU),
    );
    let opened = settle(&mut dom, opened);
    let seen = pick_named(&mut dom, &opened, mail_ui_model::copy::menus::TIDY_UP).await;
    let page = dioxus_ssr::render(&dom);
    let button = mail_ui_model::copy::controls::archive_count(1);
    assert!(
        page.contains(ACCEPTED),
        "the sheet does not name it:\n{page}"
    );
    assert!(
        page.contains(&button),
        "the sheet does not count 1:\n{page}"
    );

    let seen = settle(&mut dom, seen);
    let archive = *seen
        .all("data-answer", "archive")
        .last()
        .unwrap_or_else(|| panic!("no {button:?} button"));
    let clicked = click(&mut dom, archive);
    let _ = settle(&mut dom, clicked);
    assert!(!in_inbox(&built.store, accepted), "it was not archived");
    for thread in [built.dana, built.sam] {
        assert!(in_inbox(&built.store, thread), "more than it listed went");
    }
    assert!(
        !dioxus_ssr::render(&dom).contains(&button),
        "the sheet stayed open"
    );

    chord(
        &mut dom,
        "z",
        PRIMARY,
        dioxus_core::ElementId(INSIDE_THE_SHELL as usize),
    );
    assert!(
        in_inbox(&built.store, accepted),
        "undo did not bring it back"
    );
}
