//! The To field's people menu, against a real contact book: its order is the book's, it leaves
//! out whoever is already on the message, and it never offers the user their own address or a
//! no-reply sender — and the ⌘K menu names the same people for the same text.

use super::super::page::{Float, List};
use super::super::recipients::{people_items, pick_person, typed};
use super::*;
use crate::ui::contacts::tests::{ADDED, HEARD, ME, NO_REPLY, WRITTEN, the_book};

fn keys(page: &Page, list: List) -> Vec<String> {
    people_items(page, list)
        .into_iter()
        .map(|item| item.key)
        .collect()
}

/// What the book itself answers for `text`, best first.
fn book_order(store: &SqliteStore, text: &str) -> Vec<String> {
    use mail_core::Store;
    store
        .contacts_matching(text, crate::ui::contacts::book::SUGGESTED)
        .unwrap_or_else(|why| panic!("the book: {why}"))
        .into_iter()
        .map(|contact| contact.address)
        .collect()
}

#[test]
fn the_people_menu_is_the_books_order_less_whoever_is_on_the_message() {
    let (store, _dir) = the_book();
    let mut page = Page::of(&draft_of(""), Vec::new(), Vec::new());
    let before = keys(&page, List::To);
    assert_eq!(
        before,
        Vec::<String>::new(),
        "a menu before anything was typed"
    );

    typed(&mut page, List::To, "da".to_owned(), store.as_ref());
    let offered = keys(&page, List::To);
    assert_eq!(offered, book_order(&store, "da"));
    // Written to twice outranks heard from once, and the hand-added entry is offered at all.
    for (address, why) in [
        (WRITTEN, "written to"),
        (ADDED, "added"),
        (HEARD, "heard from"),
    ] {
        assert!(offered.contains(&address.to_owned()), "{why}: {offered:?}");
    }
    let rank = |address: &str| offered.iter().position(|one| one == address);
    assert!(rank(WRITTEN) < rank(HEARD), "{offered:?}");
    for (address, why) in [(ME, "the user's own"), (NO_REPLY, "a no-reply sender")] {
        assert!(
            !offered.contains(&address.to_owned()),
            "{why} offered: {offered:?}"
        );
    }
    assert_eq!(
        page.float,
        Float::People {
            list: List::To,
            active: 0
        }
    );

    // Picking one puts it on the message; typing the same text again leaves it out.
    pick_person(&mut page, List::To, WRITTEN);
    assert_eq!(page.to.len(), 1);
    typed(&mut page, List::Cc, "da".to_owned(), store.as_ref());
    let without: Vec<String> = offered
        .iter()
        .filter(|one| one.as_str() != WRITTEN)
        .cloned()
        .collect();
    assert_eq!(keys(&page, List::Cc), without);
}

/// The page with "r" typed in To and the book's people under it, laid out by the real app.
fn with_the_menu_open() -> (String, tempfile::TempDir) {
    super::render::window_with(|page, store| {
        page.subject = "Offsite, Friday".to_owned();
        typed(page, List::To, "r".to_owned(), store);
    })
}

/// The people menu open in the window, for the style lint in `style::tests`.
pub(in crate::ui) fn drawn_states() -> String {
    let (markup, _root) = with_the_menu_open();
    assert!(
        markup.contains(r#"role="menu""#),
        "no people menu:\n{markup}"
    );
    markup
}

#[test]
fn ctrl_t_names_the_same_people_as_the_to_field_for_the_same_text() {
    let (store, _dir) = the_book();
    for text in ["da", "dan", "okafor", "quinn", "example"] {
        let mut page = Page::of(&draft_of(""), Vec::new(), Vec::new());
        typed(&mut page, List::To, text.to_owned(), store.as_ref());
        let composer = keys(&page, List::To);
        let menu = crate::ui::command::tests::people_for(&store, text);
        assert!(!composer.is_empty(), "{text}: nobody in the composer");
        // ⌘K draws three people, four when one is the top hit; the composer up to eight.
        // Up to three, the lists are the same list.
        if composer.len() <= 3 {
            assert_eq!(menu, composer, "{text}");
        } else {
            assert!(menu.len() == 3 || menu.len() == 4, "{text}: {menu:?}");
            assert_eq!(menu, composer[..menu.len()], "{text}");
        }
    }
}
