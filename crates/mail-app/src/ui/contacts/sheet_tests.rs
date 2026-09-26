//! The Contacts sheet as drawn: its filter, its writes, every class it uses styled, and a file
//! of it over the frame to look at.

use std::sync::Arc;

use dioxus::prelude::*;
use dioxus_core::VirtualDom;
use mail_store::{SqliteStore, Store};

use super::ContactsSheet;
use super::tests::{ADDED, HEARD, NO_REPLY, the_book};
use crate::ui::fixtures::{click, dispatching, rebuild_into, type_into};
use crate::view::Shell;

/// The sheet open on `filter`, alone.
#[component]
fn Sheet(filter: String) -> Element {
    let shell = use_signal(|| Shell {
        contacts: Some(filter.clone()),
        ..Shell::default()
    });
    rsx! { ContactsSheet { shell } }
}

fn sheet(store: &Arc<SqliteStore>, filter: &str) -> VirtualDom {
    VirtualDom::new_with_props(
        Sheet,
        SheetProps {
            filter: filter.to_owned(),
        },
    )
    .with_root_context(store.clone())
}

fn listed(page: &str) -> Vec<String> {
    page.split("class=\"addr\">")
        .skip(1)
        .filter_map(|rest| rest.split('<').next())
        .map(str::to_owned)
        .collect()
}

#[tokio::test]
async fn the_sheet_lists_the_whole_book_and_narrows_as_the_filter_is_typed() {
    dispatching();
    let (store, _dir) = the_book();
    let mut dom = sheet(&store, "");
    let seen = rebuild_into(&mut dom);
    let everyone = listed(&dioxus_ssr::render(&dom));
    assert_eq!(everyone.len(), store.contacts().unwrap_or_default().len());
    assert!(everyone.contains(&NO_REPLY.to_owned()), "{everyone:?}");

    type_into(&mut dom, seen.one("value", ""), "quinn");
    assert_eq!(listed(&dioxus_ssr::render(&dom)), vec![ADDED.to_owned()]);
}

#[tokio::test]
async fn the_sheet_renames_and_forgets_an_entry() {
    dispatching();
    let (store, _dir) = the_book();
    let mut dom = sheet(&store, "");
    let seen = rebuild_into(&mut dom);
    // Drawn once and kept: the row is keyed, so its buttons outlive the renders below.
    let forget = seen.one("aria-label", &format!("Forget {HEARD}"));

    let seen = click(
        &mut dom,
        seen.one("aria-label", &format!("Add to contacts: {HEARD}")),
    );
    let field = seen.one("value", "Dana Okafor");
    type_into(&mut dom, field, "Dana Okafor (design)");
    click(
        &mut dom,
        seen.one("aria-label", &format!("Save the name for {HEARD}")),
    );
    let entry = store.contact(HEARD).ok().flatten();
    assert_eq!(
        entry.and_then(|contact| contact.name),
        Some("Dana Okafor (design)".to_owned())
    );

    let before = store.contacts().unwrap_or_default().len();
    click(&mut dom, forget);
    assert_eq!(store.contacts().unwrap_or_default().len(), before - 1);
    let page = dioxus_ssr::render(&dom);
    assert!(!listed(&page).contains(&HEARD.to_owned()), "{page}");
    assert!(
        page.contains(&format!("Forgot {HEARD}. Mail may teach it again.")),
        "{page}"
    );
}

/// Let the dialog's thread and the reads after it land, and draw what they wrote.
async fn settle(dom: &mut VirtualDom) {
    for _ in 0..40 {
        let quiet = std::time::Duration::from_millis(150);
        if tokio::time::timeout(quiet, dom.wait_for_work())
            .await
            .is_err()
        {
            break;
        }
        dom.render_immediate(&mut dioxus_core::NoOpMutations);
    }
}

#[tokio::test]
async fn import_vcard_reads_the_files_the_dialog_chose() {
    use crate::ui::pick::{Ask, Dialogs};
    dispatching();
    let (store, _dir) = the_book();
    let files = tempfile::tempdir().unwrap_or_else(|why| panic!("a temp dir: {why}"));
    let card = files.path().join("friends.vcf");
    std::fs::write(
        &card,
        "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Ines Moreau\r\nEMAIL:ines.moreau@example.test\r\nEND:VCARD\r\n",
    )
    .unwrap_or_else(|why| panic!("the card: {why}"));
    let missing = files.path().join("gone.vcf");
    let (dialogs, asked) = Dialogs::answering(vec![card, missing]);
    let before = store.contacts().unwrap_or_default().len();

    let mut dom = sheet(&store, "");
    dom.provide_root_context(dialogs);
    let seen = rebuild_into(&mut dom);
    click(&mut dom, seen.one("aria-label", "Import vCard…"));
    settle(&mut dom).await;

    assert_eq!(
        *asked
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
        [(Ask::Cards, None)],
        "one dialog, for vCards"
    );
    assert_eq!(store.contacts().unwrap_or_default().len(), before + 1);
    let page = dioxus_ssr::render(&dom);
    assert!(
        listed(&page).contains(&"ines.moreau@example.test".to_owned()),
        "{page}"
    );
    // The second file could not be read, and the sheet says so last.
    assert!(page.contains("Cannot read gone.vcf."), "{page}");
}

#[tokio::test]
async fn without_a_dialog_of_its_own_a_test_opens_none() {
    dispatching();
    let (store, _dir) = the_book();
    let before = store.contacts().unwrap_or_default().len();
    let mut dom = sheet(&store, "");
    let seen = rebuild_into(&mut dom);
    click(&mut dom, seen.one("aria-label", "Import vCard…"));
    settle(&mut dom).await;
    assert_eq!(store.contacts().unwrap_or_default().len(), before);
}

/// The sheet with a name being edited, and the sender card showing and naming.
fn every_state(store: &Arc<SqliteStore>) -> String {
    let mut dom = sheet(store, "");
    let seen = rebuild_into(&mut dom);
    click(
        &mut dom,
        seen.one("aria-label", &format!("Edit name: {ADDED}")),
    );
    let mut out = dioxus_ssr::render(&dom);
    assert!(out.contains("book-name"), "no name field: {out}");
    let mut card = super::tests::card(store, HEARD, "Dana Okafor");
    let seen = rebuild_into(&mut card);
    out += &dioxus_ssr::render(&card);
    click(
        &mut card,
        seen.one("aria-label", &format!("Add to contacts: {HEARD}")),
    );
    out += &dioxus_ssr::render(&card);
    out
}

#[tokio::test]
async fn every_class_the_contacts_draw_is_styled() {
    dispatching();
    let (store, _dir) = the_book();
    let markup = every_state(&store);
    let missing =
        crate::ui::style::tests::unstyled_classes(&markup, &crate::ui::style::tests::full_css());
    assert!(missing.is_empty(), "unstyled classes: {missing:?}");
}

/// `extra` as the first child of `.app`, where the window mounts its overlays.
fn inject(page: &str, extra: &str) -> String {
    let at = page.find("class=\"app").unwrap_or(0);
    let close = page[at..].find('>').map_or(page.len(), |rel| at + rel + 1);
    format!("{}{extra}{}", &page[..close], &page[close..])
}

#[tokio::test]
#[ignore = "writes target/contacts.html and target/contacts-dark.html for a person to look at"]
async fn render_the_contacts_sheet_to_a_file() {
    dispatching();
    let built = crate::ui::fixtures::work();
    built
        .store
        .put_contact(
            "mei.lin@example.com",
            Some("林美"),
            &mail_store::Origin::Manual,
        )
        .unwrap_or_else(|why| panic!("a contact: {why}"));
    let mut frame = VirtualDom::new(crate::ui::app::App)
        .with_root_context(built.store.clone())
        .with_root_context(built.dirs.clone());
    frame.rebuild_in_place();
    let mut dom = sheet(&built.store, "");
    let seen = rebuild_into(&mut dom);
    click(
        &mut dom,
        seen.one("aria-label", "Add to contacts: sam@example.com"),
    );
    let page = inject(&dioxus_ssr::render(&frame), &dioxus_ssr::render(&dom));
    crate::ui::fixtures::dump("contacts", &page);
}

#[tokio::test]
async fn the_sheet_makes_a_group_and_adds_someone_to_it() {
    dispatching();
    let (store, _dir) = the_book();
    let before = store.groups().unwrap_or_default().len();
    let mut dom = sheet(&store, "");
    let seen = rebuild_into(&mut dom);
    let seen = click(&mut dom, seen.one("aria-label", "New group"));
    type_into(&mut dom, seen.one("value", ""), "Book club");
    click(&mut dom, seen.one("aria-label", "Make the group"));
    let groups = store.groups().unwrap_or_default();
    assert_eq!(groups.len(), before + 1);
    assert_eq!(groups[0].name, "Book club");
    assert!(groups[0].members.is_empty(), "a new group has nobody yet");

    let seen = rebuild_into(&mut dom);
    let seen = click(&mut dom, seen.one("aria-label", "Edit the group Book club"));
    type_into(&mut dom, seen.one("value", ""), ADDED);
    click(&mut dom, seen.one("aria-label", "Add to the group"));
    let group = store.groups().unwrap_or_default().remove(0);
    assert_eq!(group.members, [format!("mailto:{ADDED}")]);
    let page = dioxus_ssr::render(&dom);
    assert!(
        page.contains(&format!("Dara Quinn &#60;{ADDED}&#62;")),
        "{page}"
    );
}
