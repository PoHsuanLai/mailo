//! Settings' Vacation Reply: the accounts with a server in a list on the left
//! ([`super::super::settings_window::account_split`]), and on the right the chosen account's reply
//! ([`AwayPage`]). An account whose server cannot send a reply is listed all the same, its page's
//! switch greyed.

use dioxus::prelude::*;
use mail_client::Client;
use porter_core::AccountId;

use super::super::common::Told;
use super::super::data::{AccountRow, account_rows};
use super::away::AwayPage;
use super::{Said, push_away};
use crate::ui::copy;
use crate::ui::settings_window::account_split::{AccountSplit, NoAccountsPage, shown};
use crate::ui::view::Shell;

/// The element id the page's accounts' rows are named from: `vacation-pane-open-<account>`.
const PANE: &str = "vacation-pane";

/// The accounts a reply can be looked at on: every one with a server, whether or not it can send
/// one.
fn replying(client: &Client) -> Vec<AccountRow> {
    account_rows(client)
        .into_iter()
        .filter(|row| !row.is_local())
        .collect()
}

/// The page: the accounts, and the chosen one's reply beside them.
#[component]
pub(in crate::ui) fn VacationPage(shell: Signal<Shell>) -> Element {
    // Read again when an account comes or goes.
    let _ = crate::ui::live::moved_for(mail_client::Topics::ACCOUNTS);
    let client = crate::ui::live::client();
    let rows = replying(&client);
    let unpushed: Said = use_signal(|| None);
    let pushed = use_callback(move |row: AccountRow| push_away(row, unpushed));
    if rows.is_empty() {
        return rsx! { NoAccountsPage { title: copy::titles::VACATION_REPLY } };
    }
    let held = shell.read().vacation_account.clone();
    let chosen = shown(&rows, held.as_ref()).cloned();
    rsx! {
        AccountSplit {
            pane: PANE,
            label: copy::titles::VACATION_REPLY,
            rows: rows.clone(),
            chosen: chosen.as_ref().map(|row| row.id.clone()),
            onpick: move |id: AccountId| {
                let mut shell = shell;
                shell.write().vacation_account = Some(id);
            },
            // Read again for another account.
            if let Some(row) = chosen {
                AwayPage { key: "{row.id}", row, pushed }
            }
        }
        Told { title: copy::titles::NOT_ON_SERVER, said: unpushed }
    }
}
