//! The alert a new message gets when there is no account to send it from: the pencil, ⌘N and
//! ⌘K's Compose all start one, and with no account each would otherwise do nothing at all.

use ds::components::overlays::alert_model::{AlertButton, AlertRole, AlertStyle};
use ds::prelude::*;

use dioxus::prelude::*;

use crate::ui::view::Shell;
use mail_ui_model::copy;

/// "No account to send from", with Add Account… and Cancel.
#[component]
pub(super) fn NoAccountAlert(shell: Signal<Shell>) -> Element {
    let mut shell = shell;
    rsx! {
        Alert {
            title: "No account to send from".to_owned(),
            message: Some(TextLine::from("Add an account to write a message.")),
            style: AlertStyle::Warning,
            buttons: vec![
                AlertButton::new("Add Account\u{2026}", AlertRole::Normal, EventHandler::new(move |()| {
                    shell.write().no_account = false;
                    crate::ui::add_account::open();
                })),
                AlertButton::new(copy::controls::CANCEL, AlertRole::Cancel, EventHandler::new(move |()| {
                    shell.write().no_account = false;
                })),
            ],
        }
    }
}
