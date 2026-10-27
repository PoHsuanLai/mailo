//! Lane 15: who a message is from. On the account's page in Settings the account's own address
//! is given a full name and an alias is added; a new message's From offers the alias beside the
//! account's own address, and picking it moves the draft to it; the alias is removed on the same
//! page, and the draft written as it goes back to the account's own address.

use ds_harness::Harness;
use mail_core::{SqliteStore, Store};
use mail_domain::{IdentityId, IsDefault};
use mail_ui::ui::copy;
use std::sync::Arc;

use super::drive::{Key, PRIMARY};
use super::hands::{click, press, scroll_to, type_text, until};
use super::window::{Window, account};

const ALIAS: &str = "press@example.test";

/// The settings page's scroller, which a row below the fold is scrolled into.
const SCROLLER: &str = ".settings-scroll";

/// What carries `label` as its accessible name.
fn labelled(label: &str) -> String {
    format!("[*|aria-label=\"{label}\"]")
}

/// The field labelled `label`.
fn input(label: &str) -> String {
    format!("input{}", labelled(label))
}

/// The account's own identity and its aliases, as the store has them.
fn kept(store: &SqliteStore) -> (mail_domain::Identity, Vec<mail_domain::Identity>) {
    let all = store.identities(account()).expect("the identities read");
    let own = all
        .iter()
        .find(|one| one.default == IsDefault::Default)
        .cloned()
        .expect("the account's own identity");
    let aliases = all
        .into_iter()
        .filter(|one| one.default == IsDefault::Alternate)
        .collect();
    (own, aliases)
}

/// Push the account's page in Settings › Accounts.
fn account_page(settings: &mut Harness) {
    let row = format!("#accounts-pane-open-{}", account());
    until(settings, "the account is listed", |h| h.count(&row) == 1);
    click(settings, &row);
    let name = input(copy::fields::FULL_NAME);
    until(settings, "its page shows its name", |h| h.count(&name) == 1);
}

/// Click the field labelled `label`, scrolled to, and type `text` over what it holds.
fn fill(settings: &mut Harness, label: &str, text: &str) {
    let field = input(label);
    scroll_to(settings, SCROLLER, &field);
    click(settings, &field);
    press(settings, &[PRIMARY], Key::Char('a'), 1);
    type_text(settings, text);
}

#[test]
fn an_alias_added_in_settings_is_offered_in_from_and_removed_again() {
    let mut window = Window::open(|_| {});
    let store = Arc::clone(&window.store);

    // The account's own address is given a name, and an alias is added beside it.
    window.in_settings(copy::titles::ACCOUNTS, |settings| {
        account_page(settings);
        fill(settings, copy::fields::FULL_NAME, "Ada Lovelace\n");
        until(settings, "the name is kept", |_| {
            kept(&store).0.from.name.as_deref() == Some("Ada Lovelace")
        });

        let add = "#account-add-alias";
        scroll_to(settings, SCROLLER, add);
        click(settings, add);
        fill(settings, copy::fields::NAME, "The Press Desk");
        fill(settings, copy::fields::EMAIL, ALIAS);
        let save = labelled(copy::controls::SAVE);
        scroll_to(settings, SCROLLER, &save);
        click(settings, &save);
        until(settings, "the alias is kept and its fields close", |h| {
            kept(&store).1.len() == 1 && h.count(&input(copy::fields::EMAIL)) == 0
        });
    });
    let (own, aliases) = kept(&store);
    let alias = aliases[0].clone();
    assert_eq!(alias.from.email, ALIAS);
    assert_eq!(alias.from.name.as_deref(), Some("The Press Desk"));
    assert_eq!(alias.reply_to, None, "a blank Reply-To is none");

    // A new message offers the alias in From, and picking it sends from it.
    window.press(&[], Key::Char('c'), 1);
    window.until("c opens a composer", |h| h.count(".cpage .c-body") == 1);
    let from = ".c-props [*|data-row=from] button";
    window.until("From is offered, with two addresses to send as", |h| {
        h.count(from) == 1
    });
    window.click(from);
    window.menu_item(ALIAS);
    let from_alias = |store: &SqliteStore, id: IdentityId| {
        store
            .drafts(account())
            .expect("the drafts read")
            .iter()
            .any(|draft| draft.identity == id)
    };
    window.until_stored("the draft leaves from the alias", move |store| {
        from_alias(store, alias.id)
    });
    window.until("From says the alias", |h| {
        h.text_of(from).is_some_and(|said| said.contains(ALIAS))
    });

    // The alias is removed on the account's page; the draft written as it is the account's own.
    window.in_settings(copy::titles::ACCOUNTS, |settings| {
        account_page(settings);
        let row = format!("#account-alias-{}", alias.id);
        scroll_to(settings, SCROLLER, &row);
        click(settings, &row);
        let remove = labelled(copy::controls::REMOVE_ALIAS);
        scroll_to(settings, SCROLLER, &remove);
        click(settings, &remove);
        until(
            settings,
            "the alias is gone from the store and the page",
            |h| kept(&store).1.is_empty() && h.count(&row) == 0,
        );
    });
    assert!(
        !from_alias(&window.store, alias.id),
        "a draft still names the removed alias"
    );
    assert!(from_alias(&window.store, own.id));
}
