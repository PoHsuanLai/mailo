//! Accounts: each account on this computer, a row that opens the sheet showing its servers and
//! removing it; which accounts keep all their mail here; and Add Account….
//!
//! Which Space shows which account is the Space's menu's; this page is every account at once.

use super::offline::OfflineCopy;
use crate::ui::common::{person_tile, tile};
use crate::ui::data::account_rows;
use crate::ui::view::Shell;
use dioxus::prelude::*;
use ds::components::lists::list::model::ListStyle;
use ds::components::lists::row::size::RowSize;
use ds::prelude::*;
use ds::root::common::Common;
use ds::style::icon::family::PlateFamily;
use mail_domain::Incoming;
use mail_store::SqliteStore;
use std::sync::Arc;

/// The key of the Add Account… row, which no account id can be.
const ADD: &str = " add";

#[component]
pub(super) fn Accounts(shell: Signal<Shell>, revision: Signal<u64>) -> Element {
    // Read again when an account comes or goes.
    let _ = revision();
    let rows = try_consume_context::<Arc<SqliteStore>>()
        .map(|store| account_rows(&store))
        .unwrap_or_default();
    let mut items: Vec<ListItem<String>> = Vec::new();
    if rows.is_empty() {
        items.push(ListItem::row(
            " none".to_owned(),
            "No accounts yet".to_owned(),
            rsx! { Row { title: "No accounts yet", size: RowSize::Settings } },
        ));
    }
    for row in rows {
        let shown = row.shown();
        let key = row.id.to_string();
        // Mail kept only here has no servers to show, so its row opens nothing.
        let opens = !row.is_local();
        let id = row.id.clone();
        items.push(ListItem::row(
            key,
            shown.clone(),
            rsx! {
                Row {
                    leading: person_tile(&shown, &row.address),
                    title: shown.clone(),
                    detail: Some(TextLine::from(kind_of(&row.plan.incoming))),
                    size: RowSize::Settings,
                    accessory: if opens { Accessory::Chevron } else { Accessory::None },
                    onclick: opens.then(|| EventHandler::new(move |_| {
                        crate::ui::account_settings::open(shell, id.clone())
                    })),
                    common: Common {
                        aria_label: opens.then(|| format!("Details for {shown}")),
                        ..Common::default()
                    },
                }
            },
        ));
    }
    items.push(ListItem::row(
        ADD.to_owned(),
        "Add Account\u{2026}".to_owned(),
        rsx! {
            Row {
                leading: tile(Icon::Plus, PlateFamily::Blue),
                title: "Add Account\u{2026}",
                size: RowSize::Settings,
                accessory: Accessory::Chevron,
                onclick: move |_| crate::ui::add_account::open(),
                common: Common {
                    aria_label: Some("Add Account\u{2026}".to_owned()),
                    ..Common::default()
                },
            }
        },
    ));
    rsx! {
        Form {
            FormSection {
                title: Some("Accounts".to_owned()),
                footer: Some("Which accounts a Space shows is chosen from the Space's menu.".to_owned()),
                List::<String> { label: "Accounts", items, style: ListStyle::Grouped }
            }
            OfflineCopy {}
        }
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
