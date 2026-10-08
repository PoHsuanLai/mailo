//! The Space's menu: quire's (`SpaceMenu`), with mail's Accounts ▸ in its slot.
//!
//! Rename, Colour, Appearance, the card's accent, New Space and Delete are the kit's. Accounts
//! lists every account with a check on the ones the Space shows; a pick puts it in or takes it
//! out and leaves the menu open, so several can be switched. Deleting a Space says the mail is
//! not affected, and forgets the Space's Today.

use crate::ui::appearance::WindowDirs;
use crate::ui::data::account_rows;
use crate::ui::space::{Handle, Mail, Member, Recall, Scope, SpaceId, with_member};
use crate::ui::today::Today;
use dioxus::prelude::*;
use ds::components::app::spaces::SpaceMenu;
use ds::components::menus::item::item::{AfterPick, MenuItem};
use ds::prelude::{Availability, Check};
use mail_store::SqliteStore;
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

/// Accounts ▸ for a Space over `scope`: one row per account, checked when the Space shows it.
/// Nothing when there are no accounts.
pub(in crate::ui) fn accounts_menu(
    scope: &Scope,
    accounts: &[(AccountId, String)],
) -> Vec<MenuItem<AccountId>> {
    if accounts.is_empty() {
        return Vec::new();
    }
    let rows = accounts
        .iter()
        .map(|(id, name)| {
            let on = if scope.shows(id.clone()) {
                Check::On
            } else {
                Check::Off
            };
            MenuItem::new(id.clone(), name.clone())
                .with_check(on)
                .with_after(AfterPick::KeepOpen)
        })
        .collect();
    vec![MenuItem::Submenu {
        title: "Accounts".to_owned(),
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
                .map(|space| accounts_menu(&space.payload.scope, &every))
        })
        .unwrap_or_default();
    rsx! {
        SpaceMenu::<Mail, Recall, AccountId> {
            handle,
            new_payload: move |()| Mail::over(Scope::All),
            extra,
            on_extra: move |(space, account): (SpaceId, AccountId)| {
                let all: Vec<AccountId> = accounts().into_iter().map(|(id, _)| id).collect();
                handle.apply(space, |one| {
                    one.payload.scope = toggled(&one.payload.scope, account, &all);
                });
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
