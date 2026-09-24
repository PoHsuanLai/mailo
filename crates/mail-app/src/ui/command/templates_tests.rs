//! "New from template" in Ctrl T: the action lists the templates, Enter starts a draft from the
//! active one and opens it, and the × deletes one.

use std::cell::Cell;

use dioxus_core::VirtualDom;
use mail_store::Store;

use super::super::CommandMenu;
use super::*;
use crate::ui::fixtures::{ACCOUNT, chord, click, dispatching, rebuild_into, seeded};

thread_local! {
    static SHELL: Cell<Option<Signal<Shell>>> = const { Cell::new(None) };
}

/// Ctrl T open on `typed`, over the store the test holds.
#[component]
fn Open(typed: String) -> Element {
    let shell = use_signal(|| Shell {
        command: Some(typed.clone()),
        ..Shell::default()
    });
    SHELL.with(|slot| slot.set(Some(shell)));
    let pages = use_signal(|| 1u32);
    let revision = use_signal(|| 0u64);
    let side_hidden = use_signal(|| false);
    let sync_state = use_signal(|| crate::view::SyncState::Idle);
    let spaces = use_signal(crate::space::Spaces::default);
    let in_a_field = use_signal(|| false);
    rsx! {
        CommandMenu { shell, pages, revision, side_hidden, sync_state, spaces, in_a_field }
    }
}

fn kept(store: &SqliteStore, name: &str, subject: &str, body: &str) -> mail_domain::Template {
    let draft = crate::compose::draft_new(store, ACCOUNT, &[], subject, body, Utc::now())
        .unwrap_or_else(|why| panic!("a draft: {why}"));
    crate::template::save(store, draft.id, name, Utc::now())
        .unwrap_or_else(|why| panic!("a template: {why}"))
}

#[tokio::test]
async fn new_from_template_lists_starts_and_deletes() {
    dispatching();
    let (store, _dir) = seeded();
    let template = kept(
        &store,
        "Weekly",
        "Weekly notes",
        "Here is what moved this week.",
    );
    let mut dom = VirtualDom::new_with_props(
        Open,
        OpenProps {
            typed: ACTION.to_owned(),
        },
    )
    .with_root_context(store.clone());
    let seen = rebuild_into(&mut dom);
    let shell = SHELL.with(Cell::get).unwrap_or_else(|| panic!("no shell"));
    let field = seen.one(
        "placeholder",
        "Search mail, people, actions · try from:dana or has:attachment",
    );

    // The action is the top row for its own name; Enter takes it.
    let listed = chord(&mut dom, "Enter", Default::default(), field);
    let markup = dioxus_ssr::render(&dom);
    assert!(
        markup.contains(">Weekly<"),
        "the template is not listed:\n{markup}"
    );
    assert!(markup.contains("Here is what moved this week."), "{markup}");
    let field = listed.one("placeholder", "New from template · type to narrow");

    let drafts_before = store.drafts(ACCOUNT).map(|all| all.len()).unwrap_or(0);
    chord(&mut dom, "Enter", Default::default(), field);
    assert_eq!(
        store.drafts(ACCOUNT).map(|all| all.len()).unwrap_or(0),
        drafts_before + 1,
        "no draft was started"
    );
    let opened = dom
        .in_runtime(|| shell.peek().composing.as_ref().map(|open| open.draft))
        .unwrap_or_else(|| panic!("the draft did not open as a page"));
    let started = store.draft(opened).unwrap_or_else(|why| panic!("{why}"));
    assert_eq!(started.subject, template.subject);
    assert_eq!(started.text, template.text);
    assert!(
        dom.in_runtime(|| shell.peek().command.is_none()),
        "Ctrl T stayed open"
    );

    // Open again on the list, and delete from it.
    let mut dom = VirtualDom::new_with_props(
        Open,
        OpenProps {
            typed: ACTION.to_owned(),
        },
    )
    .with_root_context(store.clone());
    let seen = rebuild_into(&mut dom);
    let field = seen.one(
        "placeholder",
        "Search mail, people, actions · try from:dana or has:attachment",
    );
    let listed = chord(&mut dom, "Enter", Default::default(), field);
    click(
        &mut dom,
        listed.one("aria-label", "Delete template “Weekly”"),
    );
    let markup = dioxus_ssr::render(&dom);
    assert!(
        crate::template::all(&store).is_ok_and(|all| all.is_empty()),
        "the template is still kept"
    );
    assert!(markup.contains("No templates yet"), "{markup}");
    let missing = crate::ui::style::tests::unstyled_classes(&markup, crate::ui::style::STYLE);
    assert!(missing.is_empty(), "unstyled classes: {missing:?}");
}

/// Ctrl T listing two templates over the Work Space, in both themes (`target/later-ctrl-t.html`).
#[tokio::test]
#[ignore = "writes target/later-ctrl-t.html and its -dark twin for a person to look at"]
async fn render_new_from_template_to_a_file() {
    dispatching();
    let built = crate::ui::fixtures::work();
    let account = crate::compose::sending_accounts(&built.store)
        .first()
        .map(|(_, id)| *id)
        .unwrap_or_else(|| panic!("the Work Space has an account"));
    built
        .store
        .connection()
        .execute(
            "INSERT INTO identities (id, account, from_name, from_email, is_default)
             SELECT ?1, id, 'Ada', 'ada@example.test', '\"default\"' FROM accounts WHERE id = ?2",
            rusqlite::params![
                mail_domain::IdentityId::generate().to_string(),
                account.to_string()
            ],
        )
        .unwrap_or_else(|why| panic!("an identity: {why}"));
    for (name, subject, body) in [
        (
            "Weekly update",
            "Weekly notes",
            "Here is what moved this week, and what did not.",
        ),
        (
            "Interview thanks",
            "Thank you",
            "Thank you for your time today.",
        ),
        (
            "Out of office",
            "Away until Monday",
            "I am away until Monday and will reply then.",
        ),
    ] {
        let draft =
            crate::compose::draft_new(&built.store, account, &[], subject, body, Utc::now())
                .unwrap_or_else(|why| panic!("{why}"));
        crate::template::save(&built.store, draft.id, name, Utc::now())
            .unwrap_or_else(|why| panic!("{why}"));
    }
    let mut app = VirtualDom::new(crate::ui::app::App)
        .with_root_context(built.store.clone())
        .with_root_context(built.dirs.clone());
    app.rebuild_in_place();
    let mut menu = VirtualDom::new_with_props(
        Open,
        OpenProps {
            typed: ACTION.to_owned(),
        },
    )
    .with_root_context(built.store.clone());
    let seen = rebuild_into(&mut menu);
    let field = seen.one(
        "placeholder",
        "Search mail, people, actions · try from:dana or has:attachment",
    );
    chord(&mut menu, "Enter", Default::default(), field);
    let page = dioxus_ssr::render(&app);
    let overlay = dioxus_ssr::render(&menu);
    let at = page
        .find("class=\"app\"")
        .and_then(|at| page[at..].find('>').map(|close| at + close + 1))
        .unwrap_or(page.len());
    crate::ui::fixtures::dump(
        "later-ctrl-t",
        &format!("{}{overlay}{}", &page[..at], &page[at..]),
    );
}
