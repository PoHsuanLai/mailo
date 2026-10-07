//! The Rules page of Settings: an account's rules, its vacation reply, and putting both on its
//! server.
//!
//! ⌘K "Rules…" opens Settings on it. Everything it writes goes through the same
//! rows `mailo rules`, `mailo vacation` and `mailo sieve push` use — [`work`], [`away`] and
//! [`server`] are those questions and writes as functions; the parts only draw them. One account
//! at a time, because a rule acts on one account's labels and folders, and a server runs one
//! account's script.

mod away;
mod editor;
mod list;
mod server;
pub(in crate::ui) mod work;

use dioxus::prelude::*;
use ds::components::fields::field_row::{FieldGroup, FieldRow};
use ds::components::menus::item::item::MenuItem;
use ds::components::menus::pop_up_button::PopUpButton;
use mail_store::SqliteStore;
use porter_core::AccountId;
use std::sync::Arc;

use super::data::account_rows;
use crate::ui::view::{RulesPage as Showing, Shell};
use away::AwayPart;
use list::RulesPart;
use server::ServerPart;

/// The page. Its groups are each one account's, the account chosen at the top when there are
/// several.
#[component]
pub(in crate::ui) fn RulesPage(shell: Signal<Shell>, revision: Signal<u64>) -> Element {
    let store = consume_context::<Arc<SqliteStore>>();
    let rows = account_rows(&store);
    let chosen = shell.read().rules.account.clone();
    let account = chosen
        .and_then(|id| rows.iter().find(|row| row.id == id))
        .or_else(|| rows.first())
        .cloned();
    let items: Vec<MenuItem<AccountId>> = rows
        .iter()
        .map(|row| MenuItem::new(row.id.clone(), row.shown()))
        .collect();
    let several = items.len() > 1;
    rsx! {
        if several && let Some(current) = account.as_ref() {
            FieldGroup {
                FieldRow { label: "Account",
                    PopUpButton::<AccountId> {
                        items,
                        value: Some(current.id.clone()),
                        title: Some("Account".to_owned()),
                        onpick: move |id: AccountId| {
                            shell.write().rules = Showing { account: Some(id) };
                        },
                    }
                }
            }
        }
        match account {
            None => rsx! {
                FieldGroup { title: "Rules",
                    FieldRow { label: "Add an account first." }
                }
            },
            // The first node's key is the whole block's: another account remounts all three parts.
            Some(row) => rsx! {
                RulesPart { key: "{row.id}", account: row.id.clone(), shown: row.shown(), revision }
                AwayPart { row: row.clone() }
                ServerPart { row }
            },
        }
    }
}

#[cfg(test)]
mod away_tests;
#[cfg(test)]
mod page_tests;
#[cfg(test)]
mod work_tests;
