//! The Space's menu: quire's (`SpaceMenu`), with mail's Accounts ▸ in its slot.
//!
//! Edit Space…, Appearance, the card's accent, New Space and Delete are the kit's. Accounts
//! lists every account with a check on the ones the Space shows; a pick puts it in or takes it
//! out and leaves the menu open, so several can be switched. Under them is Add Account: the
//! providers that sign in straight in the browser, then Other Account…, or with none of those
//! the window itself. Deleting a Space says the mail is not affected, and forgets the Space's
//! Today.

use crate::ui::add_account::{AddPick, add_menu, offered_redirects, picked};
use crate::ui::appearance::WindowDirs;
use crate::ui::copy;
use crate::ui::data::account_rows;
use crate::ui::space::{Handle, Mail, Member, Recall, Scope, SpaceId, with_member};
use crate::ui::today::Today;
use dioxus::prelude::*;
use ds::components::app::spaces::SpaceMenu;
use ds::components::menus::item::item::{AfterPick, MenuItem};
use ds::prelude::{Availability, Check, Icon, MenuImage};
use mail_core::SqliteStore;
use mail_core::profile::ProfileId;
use porter_core::AccountId;
use std::sync::Arc;

/// Every account there is, in order, and how the window names it.
fn accounts() -> Vec<(AccountId, String)> {
    try_consume_context::<Arc<SqliteStore>>()
        .map(|store| {
            account_rows(&store)
                .into_iter()
                .map(|row| (row.id.clone(), row.shown()))
                .collect()
        })
        .unwrap_or_default()
}

/// A pick in Accounts ▸: an account put in or taken out, or a way to add one.
#[derive(Debug, Clone, PartialEq)]
pub(in crate::ui) enum AccountPick {
    Toggle(AccountId),
    Add(AddPick),
}

/// Accounts ▸ for a Space over `scope`: one row per account, checked when the Space shows it,
/// then Add Account. `redirects` are the providers that sign in straight in the browser: with
/// any, Add Account is a submenu of them and Other Account…; with none, it opens the window.
pub(in crate::ui) fn accounts_menu(
    scope: &Scope,
    accounts: &[(AccountId, String)],
    redirects: &[(porter_provider::Issuer, String)],
) -> Vec<MenuItem<AccountPick>> {
    let mut rows: Vec<MenuItem<AccountPick>> = accounts
        .iter()
        .map(|(id, name)| {
            let on = if scope.shows(id.clone()) {
                Check::On
            } else {
                Check::Off
            };
            MenuItem::new(AccountPick::Toggle(id.clone()), name.clone())
                .with_check(on)
                .with_after(AfterPick::KeepOpen)
        })
        .collect();
    if !rows.is_empty() {
        rows.push(MenuItem::Separator);
    }
    rows.push(if redirects.is_empty() {
        MenuItem::new(AccountPick::Add(AddPick::Other), copy::menus::ADD_ACCOUNT)
            .with_image(MenuImage::Icon(Icon::Plus))
    } else {
        MenuItem::Submenu {
            title: copy::menus::ADD_ACCOUNT_SUBMENU.to_owned(),
            image: Some(MenuImage::Icon(Icon::Plus)),
            availability: Availability::Enabled,
            children: add_menu(redirects)
                .into_iter()
                .map(|item| item.map(&AccountPick::Add))
                .collect(),
        }
    });
    vec![MenuItem::Submenu {
        title: copy::menus::ACCOUNTS.to_owned(),
        image: None,
        availability: Availability::Enabled,
        children: rows,
    }]
}

/// `scope` with `account` switched: out when the Space shows it, in when it does not. `all` is
/// every account, which a Space over every account becomes when one leaves it.
pub(in crate::ui) fn toggled(scope: &Scope, account: AccountId, all: &[AccountId]) -> Scope {
    let member = if scope.shows(account.clone()) {
        Member::Out
    } else {
        Member::In
    };
    with_member(scope, account, member, all)
}

/// The Space's menu, wherever it was opened.
#[component]
pub(in crate::ui) fn MailSpaceMenu(handle: Handle, mut today: Signal<Today>) -> Element {
    let every = accounts();
    let extra = handle
        .open()
        .and_then(|open| {
            handle
                .spaces()
                .read()
                .get(open.space)
                .map(|space| accounts_menu(&space.payload.scope, &every, &offered_redirects()))
        })
        .unwrap_or_default();
    rsx! {
        SpaceMenu::<Mail, Recall, AccountPick> {
            handle,
            new_payload: move |()| Mail::new_in(&ProfileId::default()),
            extra,
            on_extra: move |(space, pick): (SpaceId, AccountPick)| match pick {
                AccountPick::Toggle(account) => {
                    let all: Vec<AccountId> = accounts().into_iter().map(|(id, _)| id).collect();
                    handle.apply(space, |one| {
                        one.payload.scope = toggled(&one.payload.scope, account, &all);
                    });
                }
                AccountPick::Add(pick) => picked(pick),
            },
            kept: "Your mail".to_owned(),
            on_deleted: move |space: SpaceId| {
                today.write().drop_space(space);
                if let Some(dirs) = try_consume_context::<WindowDirs>() {
                    let _ = crate::ui::today::save(&dirs.state, &today.peek());
                }
            },
        }
    }
}

#[cfg(test)]
#[path = "space_menu/tests.rs"]
mod tests;
