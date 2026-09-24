//! The frame, the account tiles, the rows and Today, rendered from the real `App`.

use super::app::App;
use super::fixtures::{dispatching, rebuild_into, seeded, work};
use dioxus::prelude::*;
use mail_store::SqliteStore;
use std::sync::Arc;

fn page_of(store: Arc<SqliteStore>, dirs: Option<crate::appearance::WindowDirs>) -> String {
    let mut dom = VirtualDom::new(App).with_root_context(store);
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
        if window.contains("row-sub") {
            break at;
        }
        from = at + subject.len();
    };
    let start = page[..at].rfind("<li").unwrap_or_else(|| {
        let from = at.saturating_sub(180);
        panic!("{subject:?} is not in a row. around: {}", &page[from..at])
    });
    let end = page[at..]
        .find("</li>")
        .map(|offset| at + offset + "</li>".len())
        .unwrap_or(page.len());
    page[start..end].to_owned()
}

#[tokio::test]
async fn three_accounts_show_an_all_tile_and_the_microsoft_tile_filters() {
    dispatching();
    let built = work();
    let mut dom = VirtualDom::new(App)
        .with_root_context(built.store)
        .with_root_context(built.dirs);
    let seen = rebuild_into(&mut dom);
    let page = dioxus_ssr::render(&dom);
    let tiles = page.matches("class=\"pin acct\"").count();
    assert_eq!(tiles, 4, "three accounts and All:\n{page}");
    let tile_pressed = page
        .split("class=\"pin acct\"")
        .skip(1)
        .filter(|rest| {
            rest.split_once('>')
                .unwrap_or_default()
                .0
                .contains("aria-pressed=\"true\"")
        })
        .count();
    assert_eq!(tile_pressed, 1, "more than one tile is pressed:\n{page}");
    assert!(page.contains("aria-label=\"All accounts\""), "{page}");
    let marks: Vec<&str> = page
        .split("class=\"ds-provider\" data-size=\"tile\"")
        .skip(1)
        .filter_map(|rest| {
            rest.split_once('>')?
                .1
                .split_once('<')
                .map(|(mark, _)| mark)
        })
        .collect();
    assert_eq!(
        marks,
        ["G", "M", "F"],
        "each account tile names its provider:\n{page}"
    );

    let tile = seen.one("aria-label", "p.lai@corp.example");
    let _ = super::fixtures::click(&mut dom, tile);
    // The list's query answers off the render, so one render after the click can still show the
    // rows from before it. Draw as work arrives until only Microsoft's rows are left, or give up.
    let only_microsoft = |page: &str| {
        page.contains("m365</span>")
            && !page.contains("gmail</span>")
            && !page.contains("fastmail</span>")
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
        filtered.contains("m365</span>"),
        "the Microsoft tile did not show its rows:\n{filtered}"
    );
    assert!(
        !filtered.contains("gmail</span>") && !filtered.contains("fastmail</span>"),
        "rows outside Microsoft stayed on screen:\n{filtered}"
    );
}

#[test]
fn one_account_has_no_all_tile() {
    let (store, _dir) = seeded();
    let page = page_of(store, None);
    assert!(
        !page.contains("All accounts"),
        "a single account still offered All:\n{page}"
    );
    assert_eq!(page.matches("class=\"pin acct\"").count(), 1, "{page}");
}

#[test]
fn the_work_rows_carry_the_read_state_the_chip_and_the_archive_strip() {
    let built = work();
    // `root` keeps the temp directory alive; `dana` is the thread the frame opens.
    let _ = (built.root.path(), built.dana);
    let page = page_of(built.store, Some(built.dirs));
    let sam = row_containing(&page, "Notes from the sync review");
    assert!(sam.contains("data-read=\"read\""), "Sam is read:\n{sam}");
    assert!(sam.contains("data-on=\"true\""), "Sam is starred:\n{sam}");
    let dana = row_containing(&page, "UIDL stability");
    assert!(
        dana.contains("data-read=\"unread\""),
        "Dana is unread:\n{dana}"
    );
    assert!(dana.contains(">spec<"), "Dana's chip is spec:\n{dana}");
    assert!(
        !dana.contains("data-op=\"star\""),
        "the strip still has a star:\n{dana}"
    );
    assert!(
        dana.contains("aria-label=\"Archive\""),
        "the strip has no archive:\n{dana}"
    );
}

#[tokio::test]
async fn closing_a_today_entry_writes_the_file_and_not_the_mail() {
    dispatching();
    let built = work();
    // The fixture seeds Today for the screenshot. This test starts from an empty list
    // so the two opens are what the file records.
    std::fs::write(built.dirs.state.join("today.json"), "{\"entries\":[]}\n").unwrap();
    let mut dom = VirtualDom::new(App)
        .with_root_context(built.store.clone())
        .with_root_context(built.dirs.clone());
    let seen = rebuild_into(&mut dom);
    let first = seen.one(
        "aria-label",
        "Open Re: UIDL stability across a UIDVALIDITY change",
    );
    let second = seen.one("aria-label", "Open Notes from the sync review");
    let opened = super::fixtures::click(&mut dom, first);
    let second = follow(&mut dom, opened, "Open Notes from the sync review", second).await;
    let shown = super::fixtures::click(&mut dom, second);
    let close_label = format!("Close {}", built.sam);
    let close = follow_any(&mut dom, shown, &close_label).await;
    let stored = std::fs::read_to_string(built.dirs.state.join("today.json")).unwrap_or_default();
    assert_eq!(
        stored.matches("\"thread\"").count(),
        2,
        "opening two threads did not store two shortcuts: {stored}"
    );
    let before = changes(&built.store);
    super::fixtures::click(&mut dom, close);
    let stored = std::fs::read_to_string(built.dirs.state.join("today.json")).unwrap_or_default();
    assert_eq!(
        stored.matches("\"thread\"").count(),
        1,
        "closing one shortcut left {stored}"
    );
    assert_eq!(
        changes(&built.store),
        before,
        "closing a Today shortcut wrote to the mail store"
    );
}

fn changes(store: &SqliteStore) -> i64 {
    store
        .connection()
        .query_row("SELECT total_changes()", [], |row| row.get(0))
        .unwrap_or(0)
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
) -> dioxus_core::ElementId {
    loop {
        if let Some(id) = latest.get("aria-label", label) {
            was = id;
        }
        let more = tokio::time::timeout(std::time::Duration::from_millis(200), dom.wait_for_work());
        if more.await.is_err() {
            return was;
        }
        latest = paint(dom);
    }
}

/// [`follow`] for an element that may not have been drawn yet.
async fn follow_any(
    dom: &mut VirtualDom,
    mut latest: super::fixtures::Seen,
    label: &str,
) -> dioxus_core::ElementId {
    let mut found = None;
    loop {
        if let Some(id) = latest.get("aria-label", label) {
            found = Some(id);
        }
        let more = tokio::time::timeout(std::time::Duration::from_millis(200), dom.wait_for_work());
        if more.await.is_err() {
            return found.unwrap_or_else(|| panic!("nothing is labelled {label:?}"));
        }
        latest = paint(dom);
    }
}

#[tokio::test]
#[ignore]
async fn render_the_frame_to_a_file() {
    // Rendered once per scheme, from `appearance.toml`'s theme, so the frame's Space tint is
    // the one each scheme derives, not a relabelled copy of the light one.
    dispatching();
    for (suffix, scheme) in [("", ds::Scheme::Light), ("-dark", ds::Scheme::Dark)] {
        let built = work();
        let mut dom = VirtualDom::new(App)
            .with_root_context(built.store.clone())
            .with_root_context(built.dirs)
            .with_root_context(super::fixtures::in_scheme(scheme));
        let seen = rebuild_into(&mut dom);
        let dana = seen.one(
            "aria-label",
            "Open Re: UIDL stability across a UIDVALIDITY change",
        );
        super::fixtures::click(&mut dom, dana);
        let body = dioxus_ssr::render(&dom);
        super::fixtures::write_page(&format!("frame{suffix}"), &super::fixtures::page(&body, ""));
    }
}
