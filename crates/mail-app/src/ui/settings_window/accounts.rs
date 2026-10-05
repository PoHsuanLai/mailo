//! Accounts: each account on this computer, with the sheet that shows its servers and removes it;
//! which accounts keep all their mail here; and Add Account….
//!
//! Which Space shows which account is the Space editor's; this page is every account at once.

use super::offline::OfflineCopy;
use crate::ui::data::account_rows;
use crate::ui::press::on_primary;
use crate::ui::view::Shell;
use dioxus::prelude::*;
use ds::components::fields::field_row::{FieldGroup, FieldRow};
use ds::prelude::*;
use ds::root::common::Common;
use mail_domain::Incoming;
use mail_store::SqliteStore;
use std::sync::Arc;

#[component]
pub(super) fn Accounts(shell: Signal<Shell>, revision: Signal<u64>) -> Element {
    // Read again when an account comes or goes.
    let _ = revision();
    let rows = try_consume_context::<Arc<SqliteStore>>()
        .map(|store| account_rows(&store))
        .unwrap_or_default();
    rsx! {
        FieldGroup { title: "Accounts",
            if rows.is_empty() {
                p { class: "capnote", "No accounts yet." }
            }
            for row in rows {
                FieldRow {
                    key: "{row.id}",
                    label: row.shown(),
                    help: Some(TextLine::from(kind_of(&row.plan.incoming))),
                    if !row.is_local() {
                        Button {
                            label: "Details\u{2026}",
                            common: Common {
                                aria_label: Some(format!("Details for {}", row.shown())),
                                ..Common::default()
                            },
                            onclick: {
                                let id = row.id;
                                on_primary(move || crate::ui::account_settings::open(shell, id))
                            },
                        }
                    }
                }
            }
            FieldRow { label: "New account",
                Button {
                    label: "Add Account\u{2026}",
                    onclick: on_primary(move || crate::ui::add_account::open(shell)),
                }
            }
        }
        OfflineCopy {}
    }
}

/// What kind of account a row is, said under its address.
fn kind_of(incoming: &Incoming) -> &'static str {
    match incoming {
        Incoming::Imap { .. } => "IMAP",
        Incoming::Pop3 { .. } => "POP3",
        Incoming::Graph => "Microsoft 365",
        Incoming::Jmap { .. } => "JMAP",
        Incoming::Local => "Mail kept on this computer",
    }
}
