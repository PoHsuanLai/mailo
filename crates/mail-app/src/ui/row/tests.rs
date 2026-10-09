//! A row's actions in the running window: a right click lists them all, a pick acts as the old
//! strip's button did (undo included), the row's one button is the ⋯, and the picks that open a
//! further menu open it.

use crate::ui::app::App;
use crate::ui::fixtures::{
    INSIDE_THE_SHELL, Seen, chord, dispatching, listed_subjects, menu_names, open_row_menu,
    rebuild_into, row_action, work,
};
use dioxus::html::input_data::keyboard_types::Modifiers;
use dioxus::prelude::*;
use dioxus_core::{ElementId, VirtualDom};
use mail_domain::*;
use mail_store::{SqliteStore, Store};
use std::sync::Arc;

struct Window {
    dom: VirtualDom,
    seen: Seen,
    store: Arc<SqliteStore>,
    dana: ThreadId,
    /// Dana's row's subject: the newest conversation in the Inbox, unread, in the inbox.
    subject: String,
    _root: tempfile::TempDir,
}

fn window() -> Window {
    dispatching();
    let built = work();
    let mut dom = VirtualDom::new(App)
        .with_root_context(built.store.clone())
        .with_root_context(built.dirs.clone());
    let seen = rebuild_into(&mut dom);
    let subject = built.store.thread(built.dana).unwrap().summary.subject;
    Window {
        dom,
        seen,
        store: built.store,
        dana: built.dana,
        subject,
        _root: built.root,
    }
}

fn changes(store: &SqliteStore) -> i64 {
    mail_store::testing::total_changes(&store)
}

#[tokio::test]
async fn a_right_click_on_a_row_lists_every_action_it_has() {
    let Window {
        mut dom,
        seen,
        subject,
        ..
    } = window();
    open_row_menu(&mut dom, &seen, &subject);
    let page = dioxus_ssr::render(&dom);
    assert!(
        page.contains(&format!("aria-label=\"Actions for {subject}\"")),
        "no menu opened for the row:\n{page}"
    );
    assert_eq!(
        menu_names(&page),
        [
            "Open in new window",
            "Forward",
            "Mark as read",
            "Star",
            "Pin",
            "Mute",
            "Snooze…",
            "Remind me if no reply…",
            "Label…",
            "Move to…",
            "Archive",
            "Trash",
        ]
    );
    // A context menu at the pointer, its groups ruled apart.
    assert!(page.contains("data-placement=\"context\""), "{page}");
}

#[tokio::test]
async fn mark_as_read_from_the_menu_is_undone_like_any_op() {
    let Window {
        mut dom,
        seen,
        store,
        dana,
        subject,
        ..
    } = window();
    assert_eq!(store.thread(dana).unwrap().summary.read, ReadState::Unread);
    row_action(&mut dom, &seen, &subject, "Mark as read").await;
    assert_eq!(store.thread(dana).unwrap().summary.read, ReadState::Read);
    chord(
        &mut dom,
        "z",
        Modifiers::CONTROL,
        ElementId(INSIDE_THE_SHELL as usize),
    );
    assert_eq!(
        store.thread(dana).unwrap().summary.read,
        ReadState::Unread,
        "⌘Z did not take the read back"
    );
}

/// Each row carries one button, quire's `RowMore` (the ⋯), which says it opens a menu and that
/// the menu is shut. A press on it measures the button to hang the menu from, which a document
/// with no renderer cannot do, so the press itself is `native_layout`'s, on Blitz.
#[tokio::test]
async fn each_row_carries_the_more_button_alone() {
    let Window { dom, seen, .. } = window();
    let page = dioxus_ssr::render(&dom);
    let rows = listed_subjects(&page);
    assert_eq!(
        page.matches("class=\"ds-row-more\"").count(),
        rows.len(),
        "not one ⋯ per row:\n{page}"
    );
    assert_eq!(
        seen.all("aria-label", "More actions").len(),
        rows.len(),
        "a ⋯ is not named More actions"
    );
    assert!(
        !page.contains("class=\"ds-strip"),
        "a row still draws the hover strip:\n{page}"
    );
}

#[tokio::test]
async fn snooze_from_the_menu_opens_the_snooze_menu() {
    let Window {
        mut dom,
        seen,
        store,
        subject,
        ..
    } = window();
    let changed = changes(&store);
    row_action(&mut dom, &seen, &subject, "Snooze…").await;
    let page = dioxus_ssr::render(&dom);
    assert!(
        page.contains("Snooze until"),
        "Snooze… did not open the snooze menu:\n{page}"
    );
    assert_eq!(changes(&store), changed, "opening the snooze menu snoozed");
}

#[test]
fn only_the_primary_button_opens_or_picks_a_row() {
    use crate::ui::selection::Click;
    use ds::base::press::{PointerButton, Press};
    let cases = [
        (
            "a plain click",
            PointerButton::Primary,
            Modifiers::empty(),
            Some(Click::Plain),
        ),
        (
            "shift",
            PointerButton::Primary,
            Modifiers::SHIFT,
            Some(Click::Range),
        ),
        (
            "ctrl",
            PointerButton::Primary,
            Modifiers::CONTROL,
            Some(Click::Toggle),
        ),
        (
            "cmd",
            PointerButton::Primary,
            Modifiers::META,
            Some(Click::Toggle),
        ),
        (
            "shift wins over ctrl",
            PointerButton::Primary,
            Modifiers::SHIFT | Modifiers::CONTROL,
            Some(Click::Range),
        ),
        (
            "a right click",
            PointerButton::Secondary,
            Modifiers::empty(),
            None,
        ),
        (
            "a middle click",
            PointerButton::Middle,
            Modifiers::empty(),
            None,
        ),
    ];
    for (case, button, modifiers, want) in cases {
        let press = Press {
            button,
            modifiers,
            ..Press::primary()
        };
        assert_eq!(super::click_of(press), want, "{case}");
    }
}
