//! Push relay: the address of the relay a Microsoft 365 account's new mail is pushed through, a
//! field on the account's page.
//!
//! Microsoft Graph pushes only to a public HTTPS address, which a desktop has not got; the relay
//! (`relay/` in the repository, deployed by the person on a Cloudflare account of their own) is
//! that address, and passes each notification on (`mail_core::sync::graph_push`). Empty, the
//! account is checked on a timer as before. The address is kept when the field is left or
//! Return pressed, checked first; why one cannot be a relay's is said under the field, which
//! keeps what was typed until it is changed. Keeping one moves the revision, so the window's
//! scheduler reads again which accounts are pushed to.

use crate::ui::data::AccountRow;
use dioxus::prelude::*;
use ds::components::fields::field_row::FieldRow;
use ds::components::fields::text_field_model::Invalid;
use ds::motion::detail::stamp::EventStamp;
use ds::prelude::*;
use mail_client::RelayRefused;
use mail_client::prelude::CoreError;
use mail_ui_model::copy::errors;
use mail_ui_model::copy::fields::account as row;

/// The account's field. Only a Graph account has one (the caller's to know).
#[component]
pub(super) fn PushRelay(row: AccountRow) -> Element {
    let client = crate::ui::live::try_client();
    let id = row.id.clone();
    let kept = {
        let (client, id) = (client.clone(), id.clone());
        move || {
            client
                .as_ref()
                .and_then(|client| client.graph_pushing().relay(id.clone()))
                .unwrap_or_default()
        }
    };
    let mut typed = use_signal(kept.clone());
    let mut failed = use_signal(|| None::<&'static str>);
    let mut keep = move || {
        let Some(client) = client.as_ref() else {
            return;
        };
        let now = typed.peek().trim().to_owned();
        if now == kept() {
            return;
        }
        match client.graph_pushing().set_relay(id.clone(), &now) {
            Ok(stored) => {
                failed.set(None);
                typed.set(stored.unwrap_or_default());
            }
            Err(error) => failed.set(Some(refused(&error))),
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
                validity: refusal(failed()),
                oninput: move |text: String| {
                    typed.set(text);
                    failed.set(None);
                },
                onblur: {
                    let mut keep = keep.clone();
                    move |()| keep()
                },
                onsubmit: move |_| keep(),
            }
        }
    }
}

/// Why `error` kept the relay from being set, in the window's words.
fn refused(error: &CoreError) -> &'static str {
    match error {
        CoreError::Relay(RelayRefused::NotAnAddress) => errors::RELAY_NOT_AN_ADDRESS,
        CoreError::Relay(RelayRefused::NotHttps) => errors::RELAY_NOT_HTTPS,
        CoreError::Relay(RelayRefused::Extras) => errors::RELAY_EXTRAS,
        other => {
            log::warn!("the push relay could not be kept: {other}");
            errors::RELAY_NOT_KEPT
        }
    }
}

/// What the field draws under itself: nothing, or why, with one stamp for each phrasing.
fn refusal(why: Option<&'static str>) -> Validity {
    match why {
        None => Validity::Valid,
        Some(why) => Validity::Invalid(Invalid {
            message: why.into(),
            stamp: EventStamp(u32::try_from(why.len()).unwrap_or(0)),
        }),
    }
}

#[cfg(test)]
#[path = "push_relay_tests.rs"]
mod tests;
