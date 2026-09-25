//! Adding an account from the window: "Add account…" from Ctrl T, the "+" after the account
//! tiles, and the Space editor.
//!
//! An address, then Look up — which sends only its domain — then what was found, where it came
//! from and how the account signs in, then Use these settings. Nothing is added and no
//! credential leaves before that last press. [`flow`] is every decision, with the network and the
//! keyring handed in as [`flow::Seams`]; the sheet only draws it.

pub(in crate::ui) mod flow;
mod sheet;

pub(in crate::ui) use sheet::AddAccountSheet;

use dioxus::prelude::*;

use crate::view::Shell;

/// The seams the window was handed, or the real ones.
pub(in crate::ui) fn seams() -> flow::Seams {
    try_consume_context::<flow::Seams>().unwrap_or_else(flow::Seams::real)
}

/// Open the sheet with an empty address, and put the cursor in it.
pub(in crate::ui) fn open(mut shell: Signal<Shell>) {
    shell.write().adding = Some(String::new());
    crate::ui::host::Host::focus_next_frame(".acct-sheet .files-main input");
}

/// Close the sheet and give the keyboard back to the window. The password, if one was typed,
/// goes with the sheet.
pub(in crate::ui) fn close(mut shell: Signal<Shell>) {
    shell.write().adding = None;
    crate::ui::host::Host::focus_app();
}

#[cfg(test)]
mod flow_tests;
#[cfg(test)]
mod sheet_tests;
