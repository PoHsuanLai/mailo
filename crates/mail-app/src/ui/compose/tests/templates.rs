//! Templates from the composer: saved from `/`, listed, started from on an empty page, deleted.

use mail_store::Store;

use super::super::page::Float;
use super::super::templates::{
    SAVE_KEY, START_KEY, every, forget, page_slash_items, pick, save_named, template_rows,
};
use super::*;
use crate::ui::fixtures::{ACCOUNT, seeded};

fn fresh(store: &SqliteStore) -> Draft {
    crate::compose::draft_new(store, ACCOUNT, &[], "", "", Utc::now())
        .unwrap_or_else(|why| panic!("a new draft: {why}"))
}

/// Open `/` at the caret, as typing it does.
fn slash(page: &mut Page) {
    type_text(page, "/");
    assert!(
        matches!(page.float, Float::Slash { .. }),
        "/ opened no menu: {:?}",
        page.float
    );
}

fn keys(items: &[crate::ui::menu::MenuItem]) -> Vec<&str> {
    items.iter().map(|item| item.key.as_str()).collect()
}

#[test]
fn the_slash_menu_offers_a_template_only_while_the_body_is_empty() {
    let mut empty = page_of("");
    slash(&mut empty);
    let offered = page_slash_items(&empty);
    assert!(keys(&offered).contains(&START_KEY), "{:?}", keys(&offered));
    assert!(keys(&offered).contains(&SAVE_KEY));

    let mut written = page_of("");
    type_text(&mut written, "Dear Dana, ");
    slash(&mut written);
    let offered = page_slash_items(&written);
    assert!(!keys(&offered).contains(&START_KEY), "{:?}", keys(&offered));
    assert!(keys(&offered).contains(&SAVE_KEY));

    // Typing narrows them like every other row.
    type_text(&mut written, "temp");
    assert_eq!(keys(&page_slash_items(&written)), [SAVE_KEY]);
}

#[tokio::test]
async fn saved_listed_started_from_and_deleted() {
    let (store, _dir) = seeded();
    let draft = fresh(&store);
    let (window, _seen) = Window::open(store.clone(), draft.clone(), None);
    let before = every(&store).len();

    // Written, then kept from `/` under a name. The `/` typed to get there does not stay.
    let mut page = window.page();
    let said = window.dom.in_runtime(|| {
        let mut write = page.write();
        write.subject = "Weekly notes".to_owned();
        type_text(&mut write, "Here is what moved this week. ");
        slash(&mut write);
        assert!(pick(&mut write, SAVE_KEY));
        assert_eq!(write.float, Float::SaveTemplate(String::new()));
        write.float = Float::SaveTemplate("Weekly".to_owned());
        save_named(&store, &mut write, Utc::now())
    });
    assert_eq!(said.as_deref(), Ok("Saved as template “Weekly”"));
    let all = every(&store);
    assert_eq!(all.len(), before + 1, "nothing was kept");
    let kept = all
        .iter()
        .map(|(_, template)| template)
        .find(|template| template.name == "Weekly")
        .unwrap_or_else(|| panic!("no template called Weekly in {all:?}"))
        .clone();
    assert_eq!(kept.subject, "Weekly notes");
    assert!(
        !kept.text.contains('/'),
        "the / went into the template: {:?}",
        kept.text
    );
    let rows = template_rows(&all, "");
    let row = rows
        .iter()
        .find(|row| row.key == kept.id.to_string())
        .unwrap_or_else(|| panic!("the template has no row"));
    assert_eq!(row.name, "Weekly");
    assert!(
        row.help
            .as_deref()
            .is_some_and(|help| help.ends_with("· Here is what moved this week.")),
        "{:?}",
        row.help
    );

    // A new, empty page addressed to Dana starts from it.
    let blank = fresh(&store);
    let (mut window, _seen) = Window::open(store.clone(), blank.clone(), None);
    let mut page = window.page();
    window.dom.in_runtime(|| {
        let mut write = page.write();
        write.to = vec![dana()];
        slash(&mut write);
        assert!(pick(&mut write, START_KEY));
    });
    let markup = window.render();
    assert!(markup.contains("Start from a template"), "{markup}");
    assert!(
        markup.contains(">Weekly<"),
        "the template is not listed:\n{markup}"
    );
    let drafts_before = store.drafts(ACCOUNT).map(|all| all.len()).unwrap_or(0);

    // Enter on the list, as the body hands it the key: the one template is the active row.
    let shell = window.shell;
    let taken = window.dom.in_scope(dioxus_core::ScopeId::APP, || {
        super::super::templates::key(page, shell, "Enter")
    });
    assert!(taken, "the open list did not take Enter");
    window.render();

    let opened = window
        .dom
        .in_runtime(|| shell.peek().composing.as_ref().map(|open| open.draft))
        .unwrap_or_else(|| panic!("no page is open"));
    assert_ne!(opened, blank.id, "the empty page is still the one open");
    let started = store
        .draft(opened)
        .unwrap_or_else(|why| panic!("not persisted: {why}"));
    assert_eq!(started.subject, kept.subject);
    assert_eq!(started.text, kept.text);
    assert_eq!(
        started
            .to
            .iter()
            .map(|a| a.email.as_str())
            .collect::<Vec<_>>(),
        ["dana@example.test"],
        "the chosen To was not kept"
    );
    assert!(
        store.draft(blank.id).is_err(),
        "the empty draft was left behind"
    );
    assert_eq!(
        store.drafts(ACCOUNT).map(|all| all.len()).unwrap_or(0),
        drafts_before,
        "one draft in place of the other"
    );

    // And it goes, saying so.
    assert_eq!(
        forget(&store, &kept.id.to_string()).as_deref(),
        Ok("Deleted template “Weekly”")
    );
    assert_eq!(every(&store).len(), before);
}
