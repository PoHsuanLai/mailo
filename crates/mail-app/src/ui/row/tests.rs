//! A row's actions in the running window: a right click lists them all, a pick acts as the old
//! strip's button did (undo included), the row's one button is the ⋯, and the picks that open a
//! further menu open it.

use crate::ui::app::App;
use crate::ui::copy::menus;
use crate::ui::fixtures::{
    INSIDE_THE_SHELL, Seen, chord, click, dispatching, listed_subjects, menu_names, open_row_menu,
    rebuild_into, row_action, settle, work,
};
use dioxus::html::input_data::keyboard_types::Modifiers;
use dioxus::prelude::*;
use dioxus_core::{ElementId, VirtualDom};
use mail_core::{SqliteStore, Store};
use mail_domain::*;
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
    mail_store::testing::total_changes(store)
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
            crate::ui::window::OPEN_NAME,
            menus::FORWARD,
            menus::MARK_AS_READ,
            menus::STAR,
            menus::PIN,
            menus::MUTE,
            menus::SNOOZE,
            menus::REMIND,
            menus::LABEL,
            menus::MOVE_TO,
            menus::ARCHIVE,
            menus::MARK_AS_SPAM,
            menus::MOVE_TO_TRASH,
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
    row_action(&mut dom, &seen, &subject, menus::MARK_AS_READ).await;
    assert_eq!(store.thread(dana).unwrap().summary.read, ReadState::Read);
    chord(
        &mut dom,
        "z",
        crate::ui::fixtures::PRIMARY,
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
    row_action(&mut dom, &seen, &subject, menus::SNOOZE).await;
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
    use chordkit::{Desktop, Platform};
    use ds::base::press::{PointerButton, Press};
    let ours = Platform::Linux {
        desktop: Desktop::Ours,
    };
    // The key that toggles is the platform's primary one: Command on a Mac and our desktop,
    // Ctrl on Windows and other Linux. The other is not.
    let cases = [
        (
            "a plain click",
            ours,
            PointerButton::Primary,
            Modifiers::empty(),
            Some(Click::Plain),
        ),
        (
            "shift",
            ours,
            PointerButton::Primary,
            Modifiers::SHIFT,
            Some(Click::Range),
        ),
        (
            "command on our desktop",
            ours,
            PointerButton::Primary,
            Modifiers::SUPER,
            Some(Click::Toggle),
        ),
        (
            "command, as a window may report it",
            ours,
            PointerButton::Primary,
            Modifiers::META,
            Some(Click::Toggle),
        ),
        (
            "ctrl is not the primary key on our desktop",
            ours,
            PointerButton::Primary,
            Modifiers::CONTROL,
            Some(Click::Plain),
        ),
        (
            "ctrl on Windows",
            Platform::Windows,
            PointerButton::Primary,
            Modifiers::CONTROL,
            Some(Click::Toggle),
        ),
        (
            "shift wins over the primary key",
            ours,
            PointerButton::Primary,
            Modifiers::SHIFT | Modifiers::SUPER,
            Some(Click::Range),
        ),
        (
            "a right click",
            ours,
            PointerButton::Secondary,
            Modifiers::empty(),
            None,
        ),
        (
            "a middle click",
            ours,
            PointerButton::Middle,
            Modifiers::empty(),
            None,
        ),
    ];
    for (case, platform, button, modifiers, want) in cases {
        let press = Press {
            button,
            modifiers,
            ..Press::primary()
        };
        assert_eq!(super::click_of(press, platform), want, "{case}");
    }
}

/// The owner's report: right-clicking an archived conversation offered Archive. In the Archive,
/// its menu offers the way back, Move to Inbox, and not Archive.
#[tokio::test]
async fn an_archived_conversation_s_menu_offers_move_to_inbox_and_not_archive() {
    let Window {
        mut dom,
        seen,
        store,
        dana,
        subject,
        ..
    } = window();
    assert!(crate::ui::ops::apply_op(&store, dana, OpKind::Archive));
    let archive = seen.one("data-place", "Done");
    let mut seen = seen.merge(click(&mut dom, archive));
    for _ in 0..12 {
        if tokio::time::timeout(std::time::Duration::from_millis(60), dom.wait_for_work())
            .await
            .is_err()
        {
            break;
        }
        let mut more = Seen::default();
        dom.render_immediate(&mut more);
        seen = seen.merge(more);
    }
    let seen = settle(&mut dom, seen);
    assert!(
        listed_subjects(&dioxus_ssr::render(&dom)).contains(&subject),
        "the Archive does not list the archived conversation"
    );
    open_row_menu(&mut dom, &seen, &subject);
    let names = menu_names(&dioxus_ssr::render(&dom));
    assert!(
        names.iter().any(|name| name == menus::MOVE_TO_INBOX),
        "{names:?}"
    );
    assert!(
        !names.iter().any(|name| name == menus::ARCHIVE),
        "{names:?}"
    );
}
