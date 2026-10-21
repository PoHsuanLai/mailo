//! The Space's menu: quire's (`SpaceMenu`), with mail's Accounts ▸ in its slot.
//!
//! Appearance, New Space and Delete are the kit's; Edit Space… opens the Space's page in Settings
//! (`settings_window::open_space`), where its name, colour, theme and accounts are. Accounts
//! lists every account with a check on the ones the Space shows; a pick puts it in or takes it
//! out and leaves the menu open, so several can be switched. Under them is Add Account: the
//! providers that sign in straight in the browser, then Other Account…, or with none of those
//! the window itself. Deleting a Space says the mail is not affected, and forgets the Space's
//! Today.

use crate::ui::add_account::{AddPick, add_menu, offered_redirects, picked};
use crate::ui::appearance::WindowDirs;
use crate::ui::copy;
use crate::ui::data::account_rows;
use crate::ui::space::{Handle, Mail, Member, Recall, SpaceId, held_elsewhere, with_member};
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
pub(in crate::ui) fn accounts() -> Vec<(AccountId, String)> {
    try_consume_context::<Arc<SqliteStore>>()
        .map(|store| {
            account_rows(&store)
                .into_iter()
                .map(|row| (row.id.clone(), row.shown()))
                .collect()
        })
        .unwrap_or_default()
}

/// Every account there is, in order: what a new Space starts with.
pub(in crate::ui) fn every_id() -> Vec<AccountId> {
    accounts().into_iter().map(|(id, _)| id).collect()
}

/// A pick in Accounts ▸: an account put in or taken out, or a way to add one.
#[derive(Debug, Clone, PartialEq)]
pub(in crate::ui) enum AccountPick {
    Toggle(AccountId),
    Add(AddPick),
}

/// Whether an account the Space shows may leave it: not when it is in no other Space, where it
/// would be out of sight (and would come straight back, `space::adopt_unplaced`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum Leaving {
    Allowed,
    LastSpace,
}

/// Accounts ▸ for a Space showing `shown`: one row per account, checked when the Space shows
/// it, then Add Account. `leaving` says, for a checked account, whether it may be taken out.
/// `redirects` are the providers that sign in straight in the browser: with any, Add Account is
/// a submenu of them and Other Account…; with none, it opens the window.
pub(in crate::ui) fn accounts_menu(
    shown: &[AccountId],
    accounts: &[(AccountId, String)],
    leaving: impl Fn(&AccountId) -> Leaving,
    redirects: &[(porter_provider::Issuer, String)],
) -> Vec<MenuItem<AccountPick>> {
    let mut rows: Vec<MenuItem<AccountPick>> = accounts
        .iter()
        .map(|(id, name)| {
            let on = shown.contains(id);
            let row = MenuItem::new(AccountPick::Toggle(id.clone()), name.clone())
                .with_check(if on { Check::On } else { Check::Off })
                .with_after(AfterPick::KeepOpen);
            if on && leaving(id) == Leaving::LastSpace {
                row.with_availability(Availability::Disabled)
                    .with_tip(copy::tooltips::IN_NO_OTHER_SPACE)
            } else {
                row
            }
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

/// `accounts` with `account` switched: out when the Space shows it, in when it does not.
pub(in crate::ui) fn toggled(accounts: &[AccountId], account: AccountId) -> Vec<AccountId> {
    let member = if accounts.contains(&account) {
        Member::Out
    } else {
        Member::In
    };
    with_member(accounts, account, member)
}

/// The Space's menu, wherever it was opened.
#[component]
pub(in crate::ui) fn MailSpaceMenu(handle: Handle, mut today: Signal<Today>) -> Element {
    let every = accounts();
    let extra = handle
        .open()
        .and_then(|open| {
            let held = handle.spaces();
            let spaces = held.read();
            spaces.get(open.space).map(|space| {
                let leaving = |id: &AccountId| {
                    if held_elsewhere(&spaces, open.space, id) {
                        Leaving::Allowed
                    } else {
                        Leaving::LastSpace
                    }
                };
                accounts_menu(
                    &space.payload.accounts,
                    &every,
                    leaving,
                    &offered_redirects(),
                )
            })
        })
        .unwrap_or_default();
    rsx! {
        SpaceMenu::<Mail, Recall, AccountPick> {
            handle,
            new_payload: move |()| Mail::new_in(&ProfileId::default(), every_id()),
            extra,
            on_extra: move |(space, pick): (SpaceId, AccountPick)| match pick {
                AccountPick::Toggle(account) => {
                    handle.apply(space, |one| {
                        one.payload.accounts = toggled(&one.payload.accounts, account);
                    });
                }
                AccountPick::Add(pick) => picked(pick),
            },
            kept: "Your mail".to_owned(),
            on_edit: move |space: SpaceId| crate::ui::settings_window::open_space(space),
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
