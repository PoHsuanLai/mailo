//! Hover cards in the running window: they wait, they open, and they write nothing.

use super::cards::{Lines, first_lines};
use crate::ui::app::App;
use crate::ui::fixtures::{FakePointer, dispatching, pointer, rebuild_into, work};
use dioxus::prelude::*;
use dioxus_core::{NoOpMutations, VirtualDom};
use mail_domain::*;
use mail_store::{SqliteStore, Store};

const DANA: &str = "Re: UIDL stability across a UIDVALIDITY change";
const SPOOF: &str = "Unusual sign-in attempt blocked";

fn changes(store: &SqliteStore) -> i64 {
    store
        .connection()
        .query_row("SELECT total_changes()", [], |row| row.get(0))
        .unwrap_or(0)
}

/// Let `for_ms` of real time pass, running whatever the window's timers wake for.
async fn wait(dom: &mut VirtualDom, for_ms: u64) {
    let until = tokio::time::Instant::now() + std::time::Duration::from_millis(for_ms);
    loop {
        let left = until.saturating_duration_since(tokio::time::Instant::now());
        if left.is_zero() {
            break;
        }
        let _ = tokio::time::timeout(left, dom.wait_for_work()).await;
        dom.render_immediate(&mut NoOpMutations);
    }
}

fn over(offset: (f64, f64)) -> FakePointer {
    FakePointer {
        client: (420.0, 140.0),
        offset,
        held: false,
    }
}

/// The card's markup, from its opening tag to the end of the page's last card.
fn card(page: &str) -> Option<&str> {
    page.find("role=\"tooltip\"").map(|at| &page[at..])
}

#[tokio::test]
async fn hovering_a_row_for_a_second_writes_nothing() {
    dispatching();
    let built = work();
    let store = built.store.clone();
    let mut dom = VirtualDom::new(App)
        .with_root_context(built.store.clone())
        .with_root_context(built.dirs.clone());
    let seen = rebuild_into(&mut dom);
    let row = seen.one("aria-label", &format!("Open {DANA}"));
    let unread_before = store.thread(built.dana).unwrap().summary.read;
    assert_eq!(
        unread_before,
        ReadState::Unread,
        "the fixture's Dana is unread"
    );
    let before = changes(&store);

    pointer(&mut dom, "pointerenter", row, over((12.0, 10.0)));
    pointer(&mut dom, "pointerover", row, over((12.0, 10.0)));
    wait(&mut dom, 200).await;
    let early = dioxus_ssr::render(&dom);
    assert!(
        card(&early).is_none(),
        "a card opened before the pointer had rested"
    );
    wait(&mut dom, 800).await;

    let page = dioxus_ssr::render(&dom);
    let shown = card(&page).expect("no card after a second of rest");
    assert_eq!(changes(&store), before, "hovering a row wrote to the store");
    assert!(
        shown.contains("stays unread while you look"),
        "the thread card did not open, or does not say it leaves the thread unread:\n{shown}"
    );
    assert_eq!(
        store.thread(built.dana).unwrap().summary.read,
        ReadState::Unread,
        "hovering a row marked it read"
    );
}

#[tokio::test]
async fn the_sender_card_flags_a_borrowed_name() {
    dispatching();
    let built = work();
    let mut dom = VirtualDom::new(App)
        .with_root_context(built.store.clone())
        .with_root_context(built.dirs.clone());
    let seen = rebuild_into(&mut dom);
    let spoofed = crate::ui::fixtures::thread_like(&built.store, SPOOF);
    let row = seen.one("data-hc", &format!("thread:{spoofed}"));
    let name = seen.one("data-hc", &format!("sender:{spoofed}"));
    // Into the row, then onto the name inside it. The innermost hook wins: the sender card,
    // not the thread card.
    pointer(&mut dom, "pointerenter", row, over((12.0, 10.0)));
    pointer(&mut dom, "pointerover", row, over((12.0, 10.0)));
    pointer(&mut dom, "pointerover", name, over((4.0, 4.0)));
    wait(&mut dom, 700).await;
    let page = dioxus_ssr::render(&dom);
    let shown = card(&page).expect("no sender card");
    assert!(
        !shown.contains("stays unread while you look"),
        "the row's card opened instead of the name's:\n{shown}"
    );
    assert!(
        shown.contains("The name says ") && shown.contains("<b>Google</b>"),
        "no brand flag:\n{shown}"
    );
    assert!(
        shown.contains("g00gle-security.xyz"),
        "the flag does not name the real domain:\n{shown}"
    );
    assert!(
        shown.contains("First mail from this address"),
        "a first-time sender is not called one:\n{shown}"
    );
}

#[test]
fn a_card_shows_only_stored_text() {
    let raw = BlobId::generate();
    let cases: Vec<(Body, Lines)> = vec![
        (Body::Absent, Lines::NotDownloaded),
        (Body::Present { text: None, raw }, Lines::NoText),
        (
            Body::Present {
                text: Some("> quoted\n\nFirst line.\nSecond line.".to_owned()),
                raw,
            },
            Lines::Text("First line. Second line.".to_owned()),
        ),
        (
            Body::Present {
                text: Some("> only a quote".to_owned()),
                raw,
            },
            Lines::NoText,
        ),
    ];
    for (body, want) in cases {
        assert_eq!(first_lines(&body), want, "{body:?}");
    }
}
