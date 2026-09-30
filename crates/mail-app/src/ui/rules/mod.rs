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
use ds::components::content::label::LabelRole;
use ds::components::controls::segmented::Tracking;
use ds::components::overlays::sheet_width::SheetWidth;
use ds::prelude::*;
use mail_domain::AccountId;
use mail_store::SqliteStore;
use std::sync::Arc;

use super::data::account_rows;
use super::press::SheetClose;
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
    crate::ui::host::Host::focus_app();
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
    let choices: Vec<Choice<AccountId>> = rows
        .iter()
        .map(|row| Choice::new(row.id, row.shown()))
        .collect();
    let several = choices.len() > 1;
    rsx! {
        Sheet {
            label: "Rules".to_owned(),
            onclose: move |()| close(shell),
            width: SheetWidth::Wide,
            div { class: "rules",
                if several && let Some(current) = account.as_ref() {
                    SegmentedControl::<AccountId> {
                        label: "Account".to_owned(),
                        choices,
                        tracking: Tracking::SelectOne(current.id),
                        onchange: move |id: AccountId| {
                            shell.write().rules = Some(Showing { account: Some(id) });
                        },
                    }
                }
                match account {
                    None => rsx! {
                        Label {
                            text: "Add an account first: a rule belongs to one.".to_owned(),
                            role: LabelRole::Secondary,
                        }
                    },
                    Some(row) => rsx! {
                        div { class: "rules-main",
                            RulesPart { key: "r-{row.id}", account: row.id, revision }
                            AwayPart { key: "v-{row.id}", row: row.clone() }
                            ServerPart { key: "s-{row.id}", row }
                        }
                    },
                }
                div { class: "sheet-actions",
                    SheetClose { label: "Done".to_owned(), on_close: move |()| close(shell) }
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
