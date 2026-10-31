//! The frame, the account tiles, Favourites and the rows, rendered from the real `App`.

use super::app::App;
use super::fixtures::{dispatching, rebuild_into, seeded, work};
use dioxus::prelude::*;
use mail_core::SqliteStore;
use std::sync::Arc;

fn page_of(store: Arc<SqliteStore>, dirs: Option<mail_ui_kit::appearance::WindowDirs>) -> String {
    let mut dom = VirtualDom::new(App).with_root_context(crate::ui::fixtures::client(&store));
    if let Some(dirs) = dirs {
        dom = dom.with_root_context(dirs);
    }
    dom.rebuild_in_place();
    dioxus_ssr::render(&dom)
}

fn row_containing(page: &str, subject: &str) -> String {
    let mut from = 0;
    let at = loop {
        let Some(rel) = page[from..].find(subject) else {
            panic!("no {subject:?} in the list");
        };
        let at = from + rel;
        let window = &page[at.saturating_sub(80)..at];
        if window.contains("ds-thread-sub") {
            break at;
        }
        from = at + subject.len();
    };
    let start = page[..at]
        .rfind("<div class=\"ds-list-item\"")
        .unwrap_or_else(|| {
            let from = at.saturating_sub(180);
            panic!("{subject:?} is not in a row. around: {}", &page[from..at])
        });
    let end = page[at..]
        .find("<div class=\"ds-list-item\"")
        .map(|offset| at + offset)
        .unwrap_or(page.len());
    page[start..end].to_owned()
}

/// Each account tile on `page`, as the markup after its class: quire's `PinTile`s (not the
/// Add tile after them), local folders' among them, on quire's folder mark.
fn account_tiles(page: &str) -> Vec<&str> {
    page.split("class=\"ds-pin-tile\"")
        .skip(1)
        .filter(|rest| !rest.starts_with(" data-face=\"add\""))
        .collect()
}

/// A Space of three accounts draws All and a tile for each, with its provider's mark, and the
/// "+" after them; pressing Microsoft's tile leaves only Microsoft's rows in the list.
#[tokio::test]
async fn three_accounts_show_an_all_tile_and_the_microsoft_tile_filters() {
    dispatching();
    let built = work();
    let mut dom = VirtualDom::new(App)
        .with_root_context(crate::ui::fixtures::client(&built.store))
        .with_root_context(built.dirs);
    let seen = rebuild_into(&mut dom);
    let page = dioxus_ssr::render(&dom);
    assert_eq!(
        account_tiles(&page).len(),
        4,
        "three accounts and All:\n{page}"
    );
    let pressed = account_tiles(&page)
        .into_iter()
        .filter(|rest| {
            rest.split_once('>')
                .unwrap_or_default()
                .0
                .contains("aria-pressed=\"true\"")
        })
        .count();
    assert_eq!(pressed, 1, "more than one tile is pressed:\n{page}");
    assert!(page.contains("aria-label=\"All accounts\""), "{page}");
    assert!(
        page.contains(&format!(
            "aria-label=\"{}\"",
            mail_ui_model::copy::aria::ADD_ACCOUNT
        )),
        "no +:\n{page}"
    );

    let tile = seen.one_in(
        "aria-label",
        mail_ui_model::copy::aria::ACCOUNTS_IN_SPACE,
        "p.lai@corp.example",
    );
    let _ = super::fixtures::click(&mut dom, tile);
    // The list's query answers off the render, so one render after the click can still show the
    // rows from before it. Draw as work arrives until only Microsoft's rows are left, or give up.
    let only_microsoft = |page: &str| {
        page.contains("Open Unusual sign-in attempt blocked")
            && !page.contains("Open This Week in Rust 566")
            && !page.contains("Open Notes from the sync review")
    };
    let give_up = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut filtered = dioxus_ssr::render(&dom);
    while !only_microsoft(&filtered) && tokio::time::Instant::now() < give_up {
        let more = tokio::time::timeout(std::time::Duration::from_millis(200), dom.wait_for_work());
        if more.await.is_ok() {
            dom.render_immediate(&mut dioxus_core::NoOpMutations);
        }
        filtered = dioxus_ssr::render(&dom);
    }
    assert!(
        only_microsoft(&filtered),
        "the Microsoft tile did not narrow the list to its rows:\n{filtered}"
    );
}

/// One account is one tile, already the list's, and no All.
#[test]
fn one_account_has_no_all_tile() {
    let (store, _dir) = seeded();
    let page = page_of(store, None);
    assert!(
        !page.contains("All accounts"),
        "a single account still offered All:\n{page}"
    );
    assert_eq!(account_tiles(&page).len(), 1, "{page}");
}

/// Favourites is drawn only once a conversation is pinned, and then lists it.
#[test]
fn favourites_appear_once_a_conversation_is_pinned() {
    let (store, _dir) = seeded();
    let page = page_of(store.clone(), None);
    assert!(
        !page.contains(mail_ui_model::copy::headers::FAVOURITES),
        "Favourites with nothing pinned:\n{page}"
    );
    let hi = super::fixtures::thread_like(&store, "hi");
    assert!(mail_core::snooze::pin(&store, hi, chrono::Utc::now()).unwrap_or(false));
    let page = page_of(store.clone(), None);
    assert!(
        page.contains(mail_ui_model::copy::headers::FAVOURITES),
        "no Favourites with a pinned conversation:\n{page}"
    );
    let subject = mail_core::Store::thread(store.as_ref(), hi)
        .map(|thread| thread.summary.subject)
        .unwrap_or_default();
    assert!(
        page.contains(&format!("data-favourite=\"{subject}\"")),
        "the pinned conversation is not a favourite:\n{page}"
    );
}

/// A row names its account (its provider's mark, the account in its tip) only while the list
/// shows more than one: with one, the place already says whose mail it is.
#[test]
fn a_row_names_its_account_only_with_several_in_view() {
    let (store, _dir) = seeded();
    let one = page_of(store, None);
    assert!(
        !one.contains("class=\"via\""),
        "one account, and a row names it"
    );
    let built = work();
    let several = page_of(built.store, Some(built.dirs));
    assert!(
        several.contains("class=\"via\""),
        "several accounts, and no row names its own"
    );
    assert!(
        !several.contains(">imap<"),
        "the provider's name is spelled out"
    );
}

#[test]
fn the_work_rows_carry_the_read_state_the_chip_and_the_more_button() {
    let built = work();
    // `root` keeps the temp directory alive; `dana` is the thread the frame opens.
    let _ = (built.root.path(), built.dana);
    let page = page_of(built.store, Some(built.dirs));
    let sam = row_containing(&page, "Notes from the sync review");
    assert!(
        sam.contains("data-emphasis=\"plain\""),
        "Sam is read:\n{sam}"
    );
    assert!(
        sam.contains("aria-pressed=\"true\""),
        "Sam is starred:\n{sam}"
    );
    let dana = row_containing(&page, "UIDL stability");
    assert!(
        dana.contains("data-emphasis=\"strong\""),
        "Dana is unread:\n{dana}"
    );
    assert!(dana.contains(">spec<"), "Dana's chip is spec:\n{dana}");
    // The row draws no buttons: its actions are in its right-click menu.
    assert!(
        !dana.contains("class=\"ds-row-more\"") && !dana.contains("aria-haspopup"),
        "the row draws a button:\n{dana}"
    );
}

/// Opening a conversation keeps a shortcut to it for ⌘K's Recent, in the window's own file.
#[tokio::test]
async fn opening_conversations_keeps_shortcuts_for_recent() {
    dispatching();
    let built = work();
    // The fixture seeds Today for the screenshot. This test starts from an empty list
    // so the two opens are what the file records.
    std::fs::write(built.dirs.state.join("today.json"), "{\"entries\":[]}\n").unwrap();
    let mut dom = VirtualDom::new(App)
        .with_root_context(crate::ui::fixtures::client(&built.store))
        .with_root_context(built.dirs.clone());
    let seen = rebuild_into(&mut dom);
    let first = seen.one(
        "aria-label",
        "Open Re: UIDL stability across a UIDVALIDITY change",
    );
    let second = seen.one("aria-label", "Open Notes from the sync review");
    let opened = super::fixtures::click(&mut dom, first);
    let (second, _) = follow(&mut dom, opened, "Open Notes from the sync review", second).await;
    let _ = super::fixtures::click(&mut dom, second);
    loop {
        let more = tokio::time::timeout(std::time::Duration::from_millis(200), dom.wait_for_work());
        if more.await.is_err() {
            break;
        }
        let _ = paint(&mut dom);
    }
    let stored = std::fs::read_to_string(built.dirs.state.join("today.json")).unwrap_or_default();
    assert_eq!(
        stored.matches("\"item\"").count(),
        2,
        "opening two threads did not store two shortcuts: {stored}"
    );
}

fn paint(dom: &mut VirtualDom) -> super::fixtures::Seen {
    let mut seen = super::fixtures::Seen::default();
    dom.render_immediate(&mut seen);
    seen
}

/// Draw until no background work is left, and return the newest element carrying
/// `aria-label=label`, else `was`.
///
/// Opening a thread starts work whose results redraw the list, and a redrawn row is a new
/// element: clicking the id from an earlier paint lands on a node that is gone. `latest` is what
/// the click that started it all painted.
async fn follow(
    dom: &mut VirtualDom,
    mut latest: super::fixtures::Seen,
    label: &str,
    mut was: dioxus_core::ElementId,
) -> (dioxus_core::ElementId, super::fixtures::Seen) {
    let mut all = super::fixtures::Seen::default();
    loop {
        if let Some(id) = latest.get("aria-label", label) {
            was = id;
        }
        all = all.merge(latest);
        let more = tokio::time::timeout(std::time::Duration::from_millis(200), dom.wait_for_work());
        if more.await.is_err() {
            return (was, all);
        }
        latest = paint(dom);
    }
}

/// The Scheduled place is drawn while a message waits for its time on one of the Space's
/// accounts, and not otherwise: not before one is scheduled, and not for another Space's.
#[tokio::test]
async fn scheduled_is_in_the_sidebar_only_while_a_send_waits_in_the_space() {
    use mail_core::Store as _;
    dispatching();
    let (store, _dir) = seeded();
    async fn sidebar(
        store: &std::sync::Arc<mail_core::SqliteStore>,
        spaces: mail_ui_kit::space::Spaces,
    ) -> bool {
        let mut dom = VirtualDom::new(App)
            .with_root_context(crate::ui::fixtures::client(store))
            .with_root_context(spaces);
        let _ = rebuild_into(&mut dom);
        // The Space's accounts reach the shell after the first frame, through work the window
        // queues: wait until it is quiet, not for a fixed number of renders.
        for _ in 0..40 {
            let quiet = std::time::Duration::from_millis(150);
            if tokio::time::timeout(quiet, dom.wait_for_work())
                .await
                .is_err()
            {
                break;
            }
            dom.render_immediate(&mut dioxus::core::NoOpMutations);
        }
        super::fixtures::drain(&mut dom);
        dioxus_ssr::render(&dom).contains(r#"data-place="Scheduled""#)
    }
    let mine = super::fixtures::acct_account();
    let theirs = mail_domain::id::new_account_id();
    let over = |account: &porter_core::AccountId| {
        mail_ui_kit::space::first_run(std::slice::from_ref(account))
    };
    assert!(
        !sidebar(&store, over(&mine)).await,
        "Scheduled with nothing waiting"
    );

    let now = chrono::Utc::now();
    let draft = mail_core::compose::draft_new(&store, mine.clone(), &[], "Friday", "", now)
        .unwrap_or_else(|why| panic!("{why}"));
    store
        .set_send_state(
            draft.id,
            &mail_domain::SendState::Scheduled {
                at: now + chrono::TimeDelta::days(1),
            },
            now,
        )
        .unwrap_or_else(|why| panic!("{why}"));
    assert!(
        sidebar(&store, over(&mine)).await,
        "no Scheduled with a message waiting"
    );
    // Another Space's: what the row is drawn from lists nothing there. Asked of the store, as
    // the window reaches a Space's accounts through effects whose order a test should not race.
    assert!(
        crate::ui::compose::scheduled(
            &crate::ui::fixtures::client(&store),
            &mail_ui_kit::space::Scope::Accounts(vec![theirs])
        )
        .is_empty(),
        "another Space's message would draw Scheduled"
    );
}
