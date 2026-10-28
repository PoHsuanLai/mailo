//! Lane 9: contacts, where they are in the window. ⌘K's Import Contacts… brings a vCard in
//! through the file dialog; a group made as `mailo contacts group` makes one expands in To; and
//! a sender is renamed from their name in the reader, by Edit Contact… in its right-click menu.

use mail_core::Store;
use mail_core::contacts::groups;
use mail_ui::ui::copy;
use mail_ui::ui::native::DialogAsk;

use super::drive::{Drive as _, Key, PRIMARY};
use super::look::chips;
use super::window::{Window, panel_row, panel_settled};

const CARD: &str =
    "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Lin Wei\r\nEMAIL:lin@example.test\r\nEND:VCARD\r\n";

/// ⌘K, `command` typed, and its row picked.
fn command(window: &mut Window, command: &str) {
    window.press(&[PRIMARY], Key::Char('k'), 1);
    window.until("⌘K opens the panel", |h| {
        h.count(".ds-search-card input") == 1
    });
    window.type_text(command);
    let what = format!("the panel offers {command}");
    window.until(&what, |h| {
        panel_settled(h) && panel_row(h, command).is_some()
    });
    let n = panel_row(&window.harness, command).unwrap_or_default();
    window.click(&format!(".ds-search-card-rows .ds-menu > :nth-child({n})"));
}

#[test]
fn a_vcard_imports_from_the_command_panel_and_a_group_expands_in_to() {
    // The group, as `mailo contacts group` makes it: the window has no page for groups.
    let mut window = Window::open(|store| {
        let team = groups::create(store, "Team").expect("the group");
        for address in ["ada@example.test", "grace@example.test"] {
            groups::add(store, &team.id, address).expect("a member");
        }
    });
    let card = window.file("lin.vcf", CARD.as_bytes());
    window.answer(vec![card]);

    command(&mut window, "Import Contacts\u{2026}");
    window.until_stored("the card is in the book", |store| {
        store
            .contact("lin@example.test")
            .is_ok_and(|found| found.is_some())
    });
    assert_eq!(window.asked(), [DialogAsk::Cards]);
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
        h.count(".ds-menu .ds-menu-item") > 0
    });
    window.press(&[], Key::Enter, 1);
    let to = ".c-props [*|data-row=to]";
    window.until("both are in To", |h| {
        chips(h, to) == 2
            && h.text_of(to)
                .is_some_and(|to| to.contains("Ada Lovelace") && to.contains("Grace Hopper"))
    });
}

#[test]
fn a_sender_is_renamed_from_their_name_in_the_reader() {
    let mut window = Window::open(|_| {});
    window.open_subject("Flight to the conference");
    let name = ".msg-head .sender-name";
    window.until("the reader names the sender", |h| h.count(name) >= 1);
    let at = window.centre(name);
    window
        .harness
        .press(at, ds::base::press::PointerButton::Secondary);
    window.menu_item(copy::menus::EDIT_CONTACT);
    window.until("the editor asks for the name", |h| {
        h.is_focused(".contact-editor input")
    });
    // The field starts with the name the mail gave: select it all and type over it.
    window.press(&[PRIMARY], Key::Char('a'), 1);
    window.type_text("Ada King\n");
    window.until_stored("the book holds the new name", |store| {
        store
            .contact("ada@example.test")
            .ok()
            .flatten()
            .and_then(|contact| contact.name)
            .is_some_and(|name| name == "Ada King")
    });
    window.until("the editor closes", |h| h.count(".contact-editor") == 0);
}
