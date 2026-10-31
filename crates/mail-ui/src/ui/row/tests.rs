//! A row's actions in the running window: a right click lists them all, a pick acts as the old
//! strip's button did (undo included), the row draws no button, and the picks that open a
//! further menu open it.

use crate::ui::app::App;
use crate::ui::fixtures::{
    INSIDE_THE_SHELL, Seen, chord, click, dispatching, listed_subjects, menu_names, open_row_menu,
    rebuild_into, row_action, settle, work,
};
use dioxus::html::input_data::keyboard_types::Modifiers;
use dioxus::prelude::*;
use dioxus_core::{ElementId, VirtualDom};
use mail_core::{SqliteStore, Store};
use mail_domain::*;
use mail_ui_model::copy::menus;
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
        .with_root_context(crate::ui::fixtures::client(&built.store))
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
            menus::REPLY,
            menus::REPLY_ALL,
            menus::FORWARD,
            menus::MARK_AS_READ,
            menus::STAR,
            menus::FLAG,
            menus::PIN,
            menus::SNOOZE,
            menus::LABEL,
            menus::MOVE_TO,
            menus::COPY_TO,
            menus::ARCHIVE,
            menus::MOVE_TO_TRASH,
            menus::ADD_TO_VIPS,
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

/// A row draws no buttons, not even a ⋯: a click on it opens it, and its actions are in its
/// right-click menu.
#[tokio::test]
async fn a_row_draws_no_buttons() {
    let Window { dom, .. } = window();
    let page = dioxus_ssr::render(&dom);
    assert!(!listed_subjects(&page).is_empty(), "no rows:\n{page}");
    assert!(
        !page.contains("class=\"ds-row-more\""),
        "a row draws a ⋯:\n{page}"
    );
    assert!(
        !page.contains("class=\"ds-strip"),
        "a row still draws the hover strip:\n{page}"
    );
}

/// Move to…, picked from the reader's ⋯, opens its folders there and nowhere else, and holds
/// the window's letter keys while it is open, as the row's menus do: an E typed toward its search
/// does not archive the open conversation. Esc puts it away.
#[tokio::test]
async fn the_readers_move_menu_holds_the_keys_and_opens_once() {
    let Window {
        mut dom,
        seen,
        store,
        dana,
        subject,
        ..
    } = window();
    let opened = click(&mut dom, crate::ui::fixtures::row_named(&seen, &subject));
    let opened = settle(&mut dom, opened);
    let more = *opened
        .all("aria-label", mail_ui_model::copy::aria::MORE)
        .last()
        .unwrap_or_else(|| panic!("the reader has no ⋯"));
    let pressed = click(&mut dom, more);
    let menu = settle(&mut dom, pressed);
    crate::ui::fixtures::pick_named(&mut dom, &menu, menus::MOVE_TO).await;
    let page = dioxus_ssr::render(&dom);
    assert_eq!(
        page.matches("aria-placeholder=\"Move to…\"").count(),
        1,
        "the folders are not open once, under the ⋯:\n{page}"
    );
    let changed = changes(&store);
    crate::ui::fixtures::key(&mut dom, "e");
    assert_eq!(changes(&store), changed, "E acted with Move to… open");
    assert!(
        store
            .thread(dana)
            .unwrap()
            .summary
            .mailboxes
            .contains(MailboxRole::Inbox),
        "E archived the conversation under Move to…"
    );
    crate::ui::fixtures::key(&mut dom, "Escape");
    let page = dioxus_ssr::render(&dom);
    assert_eq!(
        page.matches("aria-placeholder=\"Move to…\"").count(),
        0,
        "Esc left it open"
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
        page.contains(menus::CHOOSE_TIME),
        "Snooze… did not open the snooze menu:\n{page}"
    );
    assert_eq!(changes(&store), changed, "opening the snooze menu snoozed");
}

#[test]
fn only_the_primary_button_opens_or_picks_a_row() {
    use chordkit::{Desktop, Platform};
    use ds::base::press::{PointerButton, Press};
    use mail_ui_model::selection::Click;
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
    assert!(mail_ui_kit::ops::apply_op(
        &crate::ui::fixtures::client(&store),
        dana,
        OpKind::Archive
    ));
    let archive = seen.one("data-place", "Archive");
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
