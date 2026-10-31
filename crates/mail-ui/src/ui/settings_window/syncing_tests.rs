//! How an account is synced, on its page in Settings: each menu opens on what is kept, against a
//! temporary config directory only, and Remove from Server is a POP3 account's alone.

use super::{asked, asks_first, every_label, remove_label, window_label};
use crate::ui::data::account_rows;
use crate::ui::fixtures::{Work, work};
use mail_core::offline::{self, Every, Remove, Window};
use mail_domain::*;
use mail_ui_model::copy;
use porter_core::AccountId;

const ADDRESS: &str = "poh@acme.example";

fn account(built: &Work) -> AccountId {
    account_rows(&crate::ui::fixtures::client(&built.store))
        .into_iter()
        .find(|row| row.address == ADDRESS)
        .map(|row| row.id)
        .unwrap_or_else(|| panic!("the fixture has no {ADDRESS}"))
}

/// The fixture's account, made a POP3 one.
fn as_pop3(built: &Work, id: AccountId) {
    let plan = AccountPlan {
        incoming: Incoming::Pop3 {
            host: "pop.acme.example".to_owned(),
            port: 995,
            tls: Tls::Implicit,
            leave: LeaveOnServer::Keep,
        },
        ..account_rows(&crate::ui::fixtures::client(&built.store))
            .into_iter()
            .find(|row| row.id == id)
            .map(|row| row.plan)
            .unwrap_or_else(|| panic!("the fixture has no {ADDRESS}"))
    };
    built.store.set_account_plan(id, &plan).unwrap();
}

fn page(built: &Work) -> String {
    let (dom, _) = crate::ui::settings_window::account::tests::pushed(built);
    dioxus_ssr::render(&dom)
}

#[test]
fn each_choice_is_named_as_the_menu_offers_it() {
    let every: Vec<String> = Every::CHOICES.into_iter().map(every_label).collect();
    assert_eq!(
        every,
        [
            "Automatically",
            "Every minute",
            "Every 5 minutes",
            "Every 15 minutes",
            "Every 30 minutes",
            "Every hour",
            "Manually"
        ]
    );
    let window: Vec<String> = Window::CHOICES.into_iter().map(window_label).collect();
    assert_eq!(
        window,
        [
            "All mail",
            "The last 30 days",
            "The last 90 days",
            "The last year"
        ]
    );
    let remove: Vec<String> = Remove::CHOICES.into_iter().map(remove_label).collect();
    assert_eq!(
        remove,
        [
            "Never",
            "After one day",
            "After one week",
            "After one month",
            "When deleted here"
        ]
    );
}

#[tokio::test]
async fn the_menus_open_on_what_was_kept_and_only_pop3_removes_from_the_server() {
    let built = work();
    let id = account(&built);
    let shown = page(&built);
    assert!(shown.contains(copy::fields::CHECK_FOR_NEW_MAIL), "{shown}");
    assert!(shown.contains(copy::fields::DOWNLOAD_MAIL_FROM), "{shown}");
    assert!(
        !shown.contains(copy::fields::REMOVE_FROM_SERVER),
        "an IMAP account has no server copies to remove:\n{shown}"
    );

    as_pop3(&built, id.clone());
    offline::change(&built.dirs.config, |kept| {
        kept.with_every(id.clone(), Every::Minutes(15))
            .with_window(id.clone(), Window::Days(90))
            .with_remove(id.clone(), Remove::AfterDays(7))
    })
    .unwrap_or_else(|why| panic!("{why}"));
    let shown = page(&built);
    assert!(shown.contains(copy::fields::REMOVE_FROM_SERVER), "{shown}");
    for kept in ["Every 15 minutes", "The last 90 days", "After one week"] {
        assert!(shown.contains(kept), "{kept} is not shown:\n{shown}");
    }
}

#[test]
fn any_removal_from_the_server_but_never_is_confirmed_first() {
    for chosen in [
        Remove::AfterDays(1),
        Remove::AfterDays(30),
        Remove::WhenDeleted,
    ] {
        assert!(asks_first(Remove::Never, chosen), "{chosen:?}");
        assert!(!asks_first(chosen, chosen), "{chosen:?} is already kept");
        assert!(!asked(chosen).is_empty());
    }
    assert!(
        !asks_first(Remove::WhenDeleted, Remove::Never),
        "Never deletes nothing"
    );
    assert!(asks_first(Remove::AfterDays(7), Remove::WhenDeleted));
    assert!(asked(Remove::AfterDays(1)).contains("1 day old"));
    assert!(asked(Remove::AfterDays(7)).contains("7 days old"));
}
