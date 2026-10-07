//! Lane 9: a contact group. Settings → Contacts makes a "Team" of two addresses and imports a
//! vCard through the file dialog; in a new message, typing Team in To offers the group, and
//! picking it puts both people in.

use ds_harness::{Harness, Query};
use mail_app::ui::native::DialogAsk;
use mail_store::{SqliteStore, Store};
use std::sync::Arc;

use super::drive::{Drive, Key};
use super::hands::{click, press, scroll_to, type_text, until};
use super::window::Window;

const CARD: &str =
    "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Lin Wei\r\nEMAIL:lin@example.test\r\nEND:VCARD\r\n";

/// How many members the store's groups have between them.
fn members(store: &SqliteStore) -> usize {
    store.groups().map_or(0, |groups| {
        groups.iter().map(|group| group.members.len()).sum()
    })
}

/// In Settings → Contacts: the group, its two members, and the imported card.
fn make_the_team(settings: &mut Harness, store: &SqliteStore) {
    click(settings, "[*|aria-label=\"New group\"]");
    until(settings, "the group's name is asked for", |h| {
        h.count(".book-name input") == 1
    });
    click(settings, ".book-name input");
    type_text(settings, "Team\n");
    until(settings, "the page says it made the group", |h| {
        h.text_of(".settings-page")
            .is_some_and(|page| page.contains("Made the group Team."))
    });
    click(settings, "[*|aria-label=\"Edit the group Team\"]");
    for (n, address) in ["ada@example.test", "grace@example.test"]
        .into_iter()
        .enumerate()
    {
        click(settings, "input[*|aria-label=\"Add an address\"]");
        type_text(settings, address);
        click(settings, "[*|aria-label=\"Add to the group\"]");
        let what = format!("{address} joins the group");
        until(settings, &what, |_| members(store) == n + 1);
    }
    press(settings, &[], Key::Escape, 1);

    scroll_to(
        settings,
        ".settings-scroll",
        "[*|aria-label=\"Import vCard…\"]",
    );
    click(settings, "[*|aria-label=\"Import vCard…\"]");
    until(settings, "the card is in the book", |_| {
        store
            .contact("lin@example.test")
            .is_ok_and(|found| found.is_some())
    });
}

#[test]
fn a_group_made_in_settings_expands_in_to_and_a_vcard_imports_through_the_dialog() {
    let mut window = Window::open(|_| {});
    let card = window.file("lin.vcf", CARD.as_bytes());
    window.answer(vec![card]);
    let store = Arc::clone(&window.store);
    window.in_settings("Contacts", |settings| make_the_team(settings, &store));
    assert_eq!(window.asked(), [DialogAsk::Cards]);

    let groups = window.store.groups().expect("the groups");
    assert_eq!(groups.len(), 1, "{groups:?}");
    assert_eq!(groups[0].name, "Team");
    assert_eq!(
        groups[0].members,
        ["mailto:ada@example.test", "mailto:grace@example.test"]
    );
    let lin = window
        .store
        .contact("lin@example.test")
        .expect("the book reads")
        .expect("the card was imported");
    assert_eq!(lin.name.as_deref(), Some("Lin Wei"));

    // A new message: Team in To offers the group, and Enter puts both in.
    window.press(&[], Key::Char('c'), 1);
    window.until("c opens a composer with To ready", |h| {
        h.is_focused(".cpage .c-pin input")
    });
    window.type_text("Team");
    window.until("To offers the group", |h| {
        h.text_of(".ds-menu")
            .is_some_and(|menu| menu.contains("Group · 2 people"))
    });
    window.press(&[], Key::Enter, 1);
    window.until("both are in To", |h| {
        h.count(".c-props [*|aria-label=\"Remove Ada Lovelace\"]") == 1
            && h.count(".c-props [*|aria-label=\"Remove Grace Hopper\"]") == 1
    });
}
