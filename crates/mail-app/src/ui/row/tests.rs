//! A row's actions in the running window: a right click lists them all, a pick acts as the old
//! strip's button did (undo included), the hover strip holds only the ⋯, which opens the same
//! menu and does nothing else, and the picks that open a further menu open it.

use crate::ui::app::App;
use crate::ui::fixtures::{
    INSIDE_THE_SHELL, Seen, chord, click, dispatching, listed_subjects, menu_names, open_row_menu,
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
    store
        .connection()
        .query_row("SELECT total_changes()", [], |row| row.get(0))
        .unwrap_or(0)
}

/// Draw a few frames, as the window would between them.
async fn frames(dom: &mut VirtualDom) {
    for _ in 0..8 {
        let _ =
            tokio::time::timeout(std::time::Duration::from_millis(40), dom.wait_for_work()).await;
        dom.render_immediate(&mut dioxus_core::NoOpMutations);
    }
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
async fn archive_from_the_menu_archives_it_and_offers_the_undo() {
    let Window {
        mut dom,
        seen,
        store,
        dana,
        subject,
        ..
    } = window();
    let before = store.thread(dana).unwrap().summary.mailboxes;
    assert!(before.contains(MailboxRole::Inbox));
    row_action(&mut dom, &seen, &subject, "Archive").await;
    assert!(
        !store
            .thread(dana)
            .unwrap()
            .summary
            .mailboxes
            .contains(MailboxRole::Inbox),
        "the pick did not archive the conversation"
    );
    let page = dioxus_ssr::render(&dom);
    assert!(
        page.contains("class=\"ds-toast-action\""),
        "the archive put up no toast with an Undo:\n{page}"
    );
    chord(
        &mut dom,
        "z",
        Modifiers::CONTROL,
        ElementId(INSIDE_THE_SHELL as usize),
    );
    assert_eq!(
        store.thread(dana).unwrap().summary.mailboxes,
        before,
        "⌘Z did not take the menu's archive back"
    );
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

#[tokio::test]
async fn the_hover_strip_is_one_button_that_only_opens_the_menu() {
    let Window {
        mut dom,
        seen,
        store,
        subject,
        ..
    } = window();
    let page = dioxus_ssr::render(&dom);
    let rows = listed_subjects(&page);
    assert_eq!(
        page.matches("ds-strip-action\"").count(),
        rows.len(),
        "a row's strip holds more than the ⋯:\n{page}"
    );
    let mores = seen.all("aria-label", "More actions");
    assert_eq!(mores.len(), rows.len(), "not one ⋯ per row");
    let at = rows
        .iter()
        .position(|row| *row == subject)
        .expect("Dana's row is listed");
    let (changed, readers, before) = (
        changes(&store),
        page.matches("ds-empty-state").count(),
        rows.clone(),
    );

    click(&mut dom, mores[at]);
    frames(&mut dom).await;
    let page = dioxus_ssr::render(&dom);
    assert_eq!(
        menu_names(&page).first().map(String::as_str),
        Some("Open in new window"),
        "the ⋯ did not open the row's menu:\n{page}"
    );
    assert!(
        page.contains(&format!("aria-label=\"Actions for {subject}\"")),
        "the menu is not this row's"
    );
    assert!(
        page.contains("aria-expanded=\"true\""),
        "the ⋯ does not say its menu is open"
    );
    assert_eq!(changes(&store), changed, "the ⋯ wrote to the store");
    assert_eq!(listed_subjects(&page), before, "the ⋯ changed the list");
    assert_eq!(
        page.matches("ds-empty-state").count(),
        readers,
        "the ⋯ opened the conversation"
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
