//! Push relay: the address of the relay a Microsoft 365 account's new mail is pushed through, a
//! field on the account's page.
//!
//! Microsoft Graph pushes only to a public HTTPS address, which a desktop has not got; the relay
//! (`relay/` in the repository, deployed by the person on a Cloudflare account of their own) is
//! that address, and passes each notification on (`mail_core::sync::graph_push`). Empty, the
//! account is checked on a timer as before. The address is kept when the field is left or
//! Return pressed, checked first; one that cannot be a relay's is said in a dialog and the field
//! goes back to what was kept. Keeping one moves the revision, so the window's scheduler reads
//! again which accounts are pushed to.

use crate::ui::common::Problem;
use crate::ui::copy;
use crate::ui::copy::fields::account as row;
use crate::ui::data::AccountRow;
use dioxus::prelude::*;
use ds::components::fields::field_row::FieldRow;
use ds::prelude::*;
use mail_core::SqliteStore;
use mail_core::sync::graph_push;
use std::sync::Arc;

/// The account's field. Only a Graph account has one (the caller's to know).
#[component]
pub(super) fn PushRelay(row: AccountRow, revision: Signal<u64>) -> Element {
    let store = try_consume_context::<Arc<SqliteStore>>();
    let id = row.id.clone();
    let kept = {
        let (store, id) = (store.clone(), id.clone());
        move || {
            store
                .as_ref()
                .and_then(|store| graph_push::relay(store, id.clone()))
                .unwrap_or_default()
        }
    };
    let mut typed = use_signal(kept.clone());
    let mut failed = use_signal(|| None::<String>);
    let mut keep = move || {
        let Some(store) = store.as_ref() else {
            return;
        };
        let now = typed.peek().trim().to_owned();
        if now == kept() {
            return;
        }
        match graph_push::set_relay(store, id.clone(), &now) {
            Ok(stored) => {
                typed.set(stored.unwrap_or_default());
                revision += 1;
            }
            Err(error) => {
                typed.set(kept());
                failed.set(Some(error.to_string()));
            }
        }
    };
    rsx! {
        FieldRow {
            label: row::PUSH_RELAY,
            help: Some(TextLine::from(row::PUSH_RELAY_HELP)),
            TextField {
                label: row::PUSH_RELAY,
                value: typed(),
                placeholder: row::PUSH_RELAY_EXAMPLE,
                oninput: move |text: String| typed.set(text),
                onblur: {
                    let mut keep = keep.clone();
                    move |()| keep()
                },
                onsubmit: move |_| keep(),
            }
        }
        if let Some(why) = failed() {
            Problem { title: copy::titles::NOT_SAVED, why, onclose: move |()| failed.set(None) }
        }
    }
}

#[cfg(test)]
#[path = "push_relay_tests.rs"]
mod tests;
