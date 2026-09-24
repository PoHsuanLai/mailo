//! The Rules sheet: an account's rules, its vacation reply, and putting both on its server.
//!
//! Ctrl T "Rules…" and the Space editor open it. Everything it writes goes through the same
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
use mail_store::SqliteStore;
use std::sync::Arc;

use super::data::account_rows;
use super::press::SheetClose;
use super::space_editor::Seg;
use crate::view::{RulesSheet as Showing, Shell};
use away::AwayPart;
use list::RulesPart;
use server::ServerPart;

/// Open the sheet on the first account.
pub(in crate::ui) fn open(mut shell: Signal<Shell>) {
    shell.write().rules = Some(Showing::default());
}

/// Close the sheet and give the keyboard back to the window.
pub(in crate::ui) fn close(mut shell: Signal<Shell>) {
    shell.write().rules = None;
    dioxus::document::eval("document.querySelector('.app')?.focus()");
}

/// The sheet. Mounted while `shell.rules` is `Some`.
#[component]
pub(in crate::ui) fn RulesSheet(shell: Signal<Shell>, revision: Signal<u64>) -> Element {
    let store = consume_context::<Arc<SqliteStore>>();
    let rows = account_rows(&store);
    let chosen = shell.read().rules.as_ref().and_then(|sheet| sheet.account);
    let account = chosen
        .and_then(|id| rows.iter().find(|row| row.id == id))
        .or_else(|| rows.first())
        .cloned();
    let options: Vec<(String, bool)> = rows
        .iter()
        .map(|row| {
            (
                row.shown(),
                account.as_ref().is_some_and(|a| a.id == row.id),
            )
        })
        .collect();
    let ids: Vec<_> = rows.iter().map(|row| row.id).collect();
    let several = ids.len() > 1;
    rsx! {
        div {
            class: "rules-wrap",
            onclick: move |_| close(shell),
            div {
                class: "rules",
                role: "dialog",
                aria_label: "Rules",
                onclick: move |event| event.stop_propagation(),
                div { class: "rules-head",
                    h3 { "Rules" }
                    SheetClose { on_close: move |()| close(shell) }
                }
                if several {
                    div { class: "rules-accounts",
                        Seg {
                            label: "Account".to_owned(),
                            options,
                            on_pick: move |index: usize| {
                                if let Some(id) = ids.get(index) {
                                    shell.write().rules = Some(Showing { account: Some(*id) });
                                }
                            },
                        }
                    }
                }
                match account {
                    None => rsx! {
                        p { class: "rules-none", "Add an account first: a rule belongs to one." }
                    },
                    Some(row) => rsx! {
                        div { class: "rules-main",
                            RulesPart { key: "r-{row.id}", account: row.id, revision }
                            AwayPart { key: "v-{row.id}", row: row.clone() }
                            ServerPart { key: "s-{row.id}", row }
                        }
                    },
                }
            }
        }
    }
}

#[cfg(test)]
mod away_tests;
#[cfg(test)]
mod sheet_tests;
#[cfg(test)]
mod work_tests;
