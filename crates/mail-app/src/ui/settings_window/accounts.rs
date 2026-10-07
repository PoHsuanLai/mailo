//! Accounts: a pane of two levels, as System Settings' Internet Accounts is. Its root lists each
//! account on this computer, a chevron row that pushes the account's own page ([`super::account`],
//! its servers and Remove Account…) into the same pane; which accounts keep all their mail here;
//! and Add Account….
//!
//! The pages are quire's `PaneStack`; which are shown is the window's `Shell`
//! (`accounts_pane.path`), so the main window can ask for an account's page
//! ([`super::open_account`]) and a removal can go back to the list. The stack takes the back
//! button, Escape and ⌘[, moves the keyboard to the pushed page's back button and, on the way
//! back, to the row that opened it.
//!
//! Which Space shows which account is the Space's menu's; this page is every account at once.

use super::account::{AccountPage, back, held_asking, remove_held, row_of};
use super::offline::OfflineCopy;
use crate::ui::common::{person_tile, tile};
use crate::ui::data::account_rows;
use crate::ui::press::on_primary;
use crate::ui::view::{AccountStep, AccountsPage, Shell};
use dioxus::prelude::*;
use ds::components::content::label::Label;
use ds::components::controls::button_model::{Bezel, ButtonRole};
use ds::components::fields::field_row::FieldRow;
use ds::components::lists::list::model::ListStyle;
use ds::components::lists::row::size::RowSize;
use ds::components::overlays::alert_model::{AlertButton, AlertRole, AlertStyle};
use ds::prelude::*;
use ds::root::common::Common;
use ds::style::icon::family::PlateFamily;
use mail_domain::Incoming;
use mail_store::SqliteStore;
use porter_core::AccountId;
use std::sync::Arc;

/// The key of the Add Account… row, which no account id can be.
const ADD: &str = " add";

/// The pane's name, the root's title and what an account's back button says.
const TITLE: &str = "Accounts";

/// The stack's element id: its back buttons are `accounts-pane-back-root` and `-detail`.
const STACK: &str = "accounts-pane";

/// The element id of the row that pushes `account`, where the keyboard goes back to.
fn row_id(account: &AccountId) -> String {
    format!("accounts-pane-open-{account}")
}

/// The pane: the list at its root, an account's page over it.
#[component]
pub(super) fn Accounts(shell: Signal<Shell>, revision: Signal<u64>) -> Element {
    let path = shell.read().accounts_pane.path.clone();
    let title = use_callback(|page: AccountsPage| match page {
        AccountsPage::List => TITLE.to_owned(),
        AccountsPage::Account(account) => {
            row_of(&account).map_or_else(|| account.to_string(), |row| row.shown())
        }
    });
    let page = use_callback(move |page: AccountsPage| match page {
        AccountsPage::List => rsx! { AccountList { shell, revision } },
        AccountsPage::Account(account) => rsx! { AccountPage { shell, revision, account } },
    });
    let opener = use_callback(|page: AccountsPage| match page {
        AccountsPage::List => None,
        AccountsPage::Account(account) => Some(row_id(&account)),
    });
    rsx! {
        PaneStack::<AccountsPage> {
            path,
            title,
            page,
            opener: Some(opener),
            on_back: move |()| back(shell),
            common: Common { id: Some(STACK.to_owned()), ..Common::default() },
        }
    }
}

/// The root: every account, Add Account… and which accounts keep all their mail here.
#[component]
fn AccountList(shell: Signal<Shell>, revision: Signal<u64>) -> Element {
    // Read again when an account comes or goes.
    let _ = revision();
    let store = try_consume_context::<Arc<SqliteStore>>();
    let rows = store
        .as_ref()
        .map(|store| account_rows(store))
        .unwrap_or_default();
    // Linked to the desktop's accounts, with accounts here that Mail itself signed in: one line,
    // and each of them to remove.
    let held = store.as_ref().and_then(|store| {
        crate::accountd::held_line(store).map(|line| (line, crate::accountd::held(store)))
    });
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
                    onclick: opens.then(|| EventHandler::new(move |_| push(shell, id.clone()))),
                    common: Common {
                        id: Some(row_id(&row.id)),
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
            if let Some((line, accounts)) = held {
                HeldAccounts { line, accounts, revision }
            }
            FormSection {
                footer: Some("Which accounts a Space shows is chosen from the Space's menu.".to_owned()),
                List::<String> { label: "Accounts", items, style: ListStyle::Grouped }
            }
            OfflineCopy {}
        }
    }
}

/// Push `account`'s page over the list. The pane is two levels deep, so it is the list and that
/// page whatever was shown.
pub(super) fn push(mut shell: Signal<Shell>, account: AccountId) {
    let mut write = shell.write();
    write.accounts_pane.path =
        PanePath::new(AccountsPage::List).pushed(AccountsPage::Account(account));
    if write.accounts_pane.step != AccountStep::Removing {
        write.accounts_pane.step = AccountStep::Showing;
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

/// The one line about accounts Mail signed in itself, with Add Account, and each of them by address
/// with a Remove button. Removing is the person's: it asks first, in the words of any removal, and
/// is the same removal as `mailo account remove`. It is what makes the address free for accountd's
/// account of it.
#[component]
fn HeldAccounts(
    line: &'static str,
    accounts: Vec<(AccountId, String)>,
    revision: Signal<u64>,
) -> Element {
    let mut asking = use_signal(|| None::<(AccountId, String)>);
    let failed = use_signal(|| None::<String>);
    let question = asking.read().clone().map(|(id, address)| {
        let (title, body, confirm) = held_asking(&id, &address);
        (id, title, body, confirm)
    });
    rsx! {
        FormSection {
            FieldRow { label: line,
                Button {
                    bezel: Bezel::Inline,
                    label: "Add Account",
                    onclick: on_primary(crate::ui::add_account::open),
                }
            }
            for (id, address) in accounts {
                FieldRow { key: "{id}", label: address.clone(),
                    Button {
                        bezel: Bezel::Inline,
                        label: "Remove\u{2026}",
                        role: ButtonRole::Destructive,
                        common: Common {
                            aria_label: Some(format!("Remove {address}")),
                            ..Common::default()
                        },
                        onclick: on_primary(move || asking.set(Some((id.clone(), address.clone())))),
                    }
                }
            }
            if let Some(why) = failed.read().clone() {
                Label { text: why, severity: Some(Severity::Warn) }
            }
        }
        if let Some((id, title, body, confirm)) = question {
            Alert {
                title,
                message: Some(TextLine::from(body)),
                style: AlertStyle::Critical,
                buttons: vec![
                    AlertButton::new(confirm, AlertRole::Destructive, EventHandler::new(move |()| {
                        asking.set(None);
                        remove_held(revision, failed, id.clone());
                    })),
                    AlertButton::new(
                        "Cancel",
                        AlertRole::Cancel,
                        EventHandler::new(move |()| asking.set(None)),
                    ),
                ],
            }
        }
    }
}
