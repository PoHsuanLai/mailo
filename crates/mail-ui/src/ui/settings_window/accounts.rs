//! Accounts: the accounts on this computer in a list on the left, as [`super::account_split`]
//! lays every per-account page out, each by its address and its provider's mark; on the right,
//! the chosen account's own page ([`super::account`]). Under the list stand Add Account… and
//! Other Keys, the keys and certificates no account here owns, which take the right pane in an
//! account's place. No protocol is named here: that is the account's page's.
//!
//! What is shown is the window's `Shell` (`accounts_pane.shown`), so a window can be asked onto an
//! account (`SettingsAt::Account`) and a removal can fall back to the first.
//!
//! Which Space shows which account is the Space's menu's; this page is every account at once.

use super::account::AccountPage;
use super::account_split::{AccountSplit, NoAccounts, shown, use_adding};
use crate::ui::common::{add_mark, tile};
use crate::ui::copy;
use crate::ui::data::account_rows;
use crate::ui::view::{AccountsPage, Shell};
use dioxus::prelude::*;
use ds::base::vocab::RowState;
use ds::components::lists::list::model::ListStyle;
use ds::prelude::*;
use ds::root::common::Common;
use ds::style::icon::family::PlateFamily;
use porter_core::AccountId;

/// The id of the page's panes' elements: an account's row is `accounts-pane-open-<account>`.
const PANE: &str = "accounts-pane";

/// The key of the Add Account… row, which no account id can be.
const ADD: &str = " add";

/// The element id of the Other Keys row.
const OTHER_KEYS_ROW: &str = "accounts-pane-open-other-keys";

/// The page: the accounts, and the one chosen's page beside them.
#[component]
pub(super) fn Accounts(shell: Signal<Shell>) -> Element {
    // Read again when an account comes or goes.
    let _ = crate::ui::live::moved_for(mail_client::Topics::ACCOUNTS);
    let adding = use_adding();
    let client = crate::ui::live::try_client();
    let rows = client.as_ref().map(account_rows).unwrap_or_default();
    let held = shell.read().accounts_pane.shown.clone();
    let other_keys = held == Some(AccountsPage::OtherKeys);
    let account = match &held {
        Some(AccountsPage::Account(id)) => shown(&rows, Some(id)),
        Some(AccountsPage::OtherKeys) => None,
        None => shown(&rows, None),
    }
    .map(|row| row.id.clone());
    // Add Account… is a list of its own under the accounts, as New Space is under the Spaces.
    let items = vec![
        ListItem::row(
            ADD.to_owned(),
            copy::controls::ADD_ACCOUNT.to_owned(),
            rsx! {
                Row {
                    leading: add_mark(),
                    title: copy::controls::ADD_ACCOUNT,
                    onclick: move |_| adding.start(),
                    common: Common {
                        aria_label: Some(copy::controls::ADD_ACCOUNT.to_owned()),
                        mounted: Some(adding.mounted()),
                        ..Common::default()
                    },
                }
            },
        ),
        ListItem::row(
            OTHER_KEYS_ROW.to_owned(),
            copy::titles::OTHER_KEYS.to_owned(),
            rsx! {
                Row {
                    leading: tile(Icon::Key, PlateFamily::Amber),
                    title: copy::titles::OTHER_KEYS,
                    state: RowState {
                        selection: if other_keys { Selection::Selected } else { Selection::Unselected },
                        ..RowState::default()
                    },
                    onclick: move |_| open(shell, AccountsPage::OtherKeys),
                    common: Common { id: Some(OTHER_KEYS_ROW.to_owned()), ..Common::default() },
                }
            },
        ),
    ];
    let under = rsx! {
        List::<String> { label: copy::aria::ADD_ACCOUNT, items, style: ListStyle::Plain }
    };
    rsx! {
        AccountSplit {
            pane: PANE,
            label: copy::titles::ACCOUNTS,
            rows: rows.clone(),
            chosen: account.clone(),
            onpick: move |id: AccountId| open(shell, AccountsPage::Account(id)),
            under: Some(under),
            if other_keys {
                Form {
                    crate::ui::pgp::keys::KeysPage { whose: crate::ui::pgp::keys::Whose::Others }
                }
            } else if let Some(account) = account {
                AccountPage { key: "{account}", shell, account }
            } else {
                Form {
                    FormSection { NoAccounts {} }
                }
            }
        }
        {adding.menu()}
    }
}

/// Show `page` beside the list. A removal under way keeps its step.
fn open(mut shell: Signal<Shell>, page: AccountsPage) {
    let mut write = shell.write();
    write.accounts_pane.shown = Some(page);
    if write.accounts_pane.step != crate::ui::view::AccountStep::Removing {
        write.accounts_pane.step = crate::ui::view::AccountStep::Showing;
    }
}

/// Show `account`'s page beside the list, as when the window is asked onto it.
pub(super) fn push(shell: Signal<Shell>, account: AccountId) {
    open(shell, AccountsPage::Account(account));
}
