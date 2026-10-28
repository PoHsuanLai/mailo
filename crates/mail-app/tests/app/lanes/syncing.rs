//! Lane 18: how an account is synced, on its page in Settings. Check for New Mail and Download
//! Mail From are each a menu that keeps the choice in the window's `offline.json` at once, for
//! this account alone, and an IMAP account has no Remove from Server: only POP3 has copies a pass
//! removes.

use mail_core::offline::{self, Every, Remove, Window as Reach};
use mail_ui::ui::copy;

use super::hands::{click, menu_item, until};
use super::window::{Window, account};

/// The account's row on the Accounts page.
fn account_row() -> String {
    format!("#accounts-pane-open-{}", account())
}

/// The menu button named `label`.
fn menu(label: &str) -> String {
    format!("button[*|aria-label=\"{label}\"]")
}

#[test]
fn an_accounts_page_keeps_how_often_and_how_far_back_it_is_synced() {
    let mut window = Window::open(|_| {});
    let dirs = window.dirs();
    window.in_settings(copy::titles::ACCOUNTS, |settings| {
        let row = account_row();
        until(settings, "the account is listed", |h| h.count(&row) == 1);
        click(settings, &row);
        let every = menu(copy::fields::CHECK_FOR_NEW_MAIL);
        until(settings, "its page offers Check for New Mail", |h| {
            h.count(&every) == 1
        });
        assert_eq!(
            settings.count(&menu(copy::fields::REMOVE_FROM_SERVER)),
            0,
            "an IMAP account has no server copies to remove"
        );

        click(settings, &every);
        menu_item(settings, &copy::controls::every_minutes(15));
        until(settings, "every fifteen minutes is kept", |_| {
            offline::load(&dirs.config).every_of(&account()) == Every::Minutes(15)
        });

        click(settings, &menu(copy::fields::DOWNLOAD_MAIL_FROM));
        menu_item(settings, &copy::controls::last_days(90));
        until(settings, "the last ninety days are kept", |_| {
            offline::load(&dirs.config).window_of(&account()) == Reach::Days(90)
        });
        until(settings, "the menus say what is kept", |h| {
            h.text_of(&every)
                .is_some_and(|said| said.contains(&copy::controls::every_minutes(15)))
        });

        click(settings, &every);
        menu_item(settings, copy::controls::MANUALLY);
        until(settings, "manually is kept", |_| {
            offline::load(&dirs.config).every_of(&account()) == Every::Manually
        });
    });
    let kept = offline::load(&dirs.config);
    assert_eq!(kept.remove_of(&account()), Remove::Never, "untouched");
}
