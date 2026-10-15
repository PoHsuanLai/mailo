//! Accounts: a pane of two levels, as System Settings' Internet Accounts is. Its root lists each
//! account on this computer by its address and its provider's mark, a chevron row that pushes the
//! account's own page ([`super::account`]) into the same pane; then Add Account…; then Other
//! Keys, the keys and certificates no account here owns. No protocol is named here: that is the
//! account's page's.
//!
//! The pages are quire's `PaneStack`; which are shown is the window's `Shell`
//! (`accounts_pane.path`), so a window can be asked onto an account's page (`SettingsAt::Account`)
//! and a removal can go back to the list. The stack takes the back
//! button, Escape and ⌘[, moves the keyboard to the pushed page's back button and, on the way
//! back, to the row that opened it.
//!
//! Which Space shows which account is the Space's menu's; this page is every account at once.

use super::account::{AccountPage, back, row_of};
use crate::ui::common::tile;
use crate::ui::copy;
use crate::ui::data::{AccountRow, account_rows};
use crate::ui::provider_chip::{self, MarkAt};
use crate::ui::view::{AccountStep, AccountsPage, Marks, Shell};
use dioxus::prelude::*;
use ds::components::content::provider_mark::MarkProvider;
use ds::components::lists::list::model::ListStyle;
use ds::components::lists::row::size::RowSize;
use ds::host::measure::MountedRef;
use ds::prelude::*;
use ds::root::common::Common;
use ds::style::icon::family::PlateFamily;
use ds::style::tokens::control_size::ControlSize;
use mail_core::SqliteStore;
use porter_core::AccountId;
use std::sync::Arc;

/// The key of the Add Account… row, which no account id can be.
const ADD: &str = " add";

/// The element id of the Other Keys row, where the keyboard goes back to.
const OTHER_KEYS_ROW: &str = "accounts-pane-open-other-keys";

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
        AccountsPage::List => copy::titles::ACCOUNTS.to_owned(),
        AccountsPage::Account(account) => {
            row_of(&account).map_or_else(|| account.to_string(), |row| row.shown())
        }
        AccountsPage::OtherKeys => copy::titles::OTHER_KEYS.to_owned(),
    });
    let page = use_callback(move |page: AccountsPage| match page {
        AccountsPage::List => rsx! { AccountList { shell, revision } },
        AccountsPage::Account(account) => rsx! { AccountPage { shell, revision, account } },
        AccountsPage::OtherKeys => rsx! {
            Form {
                crate::ui::pgp::keys::KeysPage { whose: crate::ui::pgp::keys::Whose::Others }
            }
        },
    });
    let opener = use_callback(|page: AccountsPage| match page {
        AccountsPage::List => None,
        AccountsPage::Account(account) => Some(row_id(&account)),
        AccountsPage::OtherKeys => Some(OTHER_KEYS_ROW.to_owned()),
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

/// The root: every account, Add Account…, and Other Keys.
#[component]
fn AccountList(shell: Signal<Shell>, revision: Signal<u64>) -> Element {
    // Read again when an account comes or goes.
    let _ = revision();
    // The add menu while it is open (the providers that sign in straight in the browser), hung
    // from the Add Account… row.
    let mut adding = use_signal(|| None::<Vec<(porter_provider::Issuer, String)>>);
    let mut add_row = use_signal(|| None::<MountedRef>);
    let store = try_consume_context::<Arc<SqliteStore>>();
    let rows = store
        .as_ref()
        .map(|store| account_rows(store))
        .unwrap_or_default();
    let mut items: Vec<ListItem<String>> = Vec::new();
    if rows.is_empty() {
        items.push(ListItem::row(
            " none".to_owned(),
            copy::empty::NO_ACCOUNTS.to_owned(),
            rsx! { Row { title: copy::empty::NO_ACCOUNTS, size: RowSize::Settings } },
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
                    leading: mark_of(&row),
                    title: shown.clone(),
                    size: RowSize::Settings,
                    accessory: if opens { Accessory::Chevron } else { Accessory::None },
                    onclick: opens.then(|| EventHandler::new(move |_| push(shell, id.clone()))),
                    common: Common {
                        id: Some(row_id(&row.id)),
                        aria_label: opens.then(|| copy::aria::details_for(&shown)),
                        ..Common::default()
                    },
                }
            },
        ));
    }
    items.push(ListItem::row(
        ADD.to_owned(),
        copy::controls::ADD_ACCOUNT.to_owned(),
        rsx! {
            Row {
                leading: tile(Icon::Plus, PlateFamily::Blue),
                title: copy::controls::ADD_ACCOUNT,
                size: RowSize::Settings,
                accessory: Accessory::Chevron,
                onclick: move |_| {
                    crate::ui::add_account::add_from(|offered| adding.set(Some(offered)))
                },
                common: Common {
                    aria_label: Some(copy::controls::ADD_ACCOUNT.to_owned()),
                    mounted: Some(EventHandler::new(move |event: MountedEvent| {
                        add_row.set(Some(MountedRef(event.data())));
                    })),
                    ..Common::default()
                },
            }
        },
    ));
    let other_keys = vec![ListItem::row(
        OTHER_KEYS_ROW.to_owned(),
        copy::titles::OTHER_KEYS.to_owned(),
        rsx! {
            Row {
                leading: tile(Icon::Key, PlateFamily::Amber),
                title: copy::titles::OTHER_KEYS,
                size: RowSize::Settings,
                accessory: Accessory::Chevron,
                onclick: move |_| other_keys(shell),
                common: Common { id: Some(OTHER_KEYS_ROW.to_owned()), ..Common::default() },
            }
        },
    )];
    rsx! {
        Form {
            FormSection {
                List::<String> { label: copy::titles::ACCOUNTS, items, style: ListStyle::Grouped }
            }
            FormSection {
                List::<String> { label: copy::titles::OTHER_KEYS, items: other_keys, style: ListStyle::Grouped }
            }
        }
        if let Some(redirects) = adding() {
            crate::ui::add_account::AddMenu {
                anchor: add_row(),
                redirects,
                on_close: move |()| adding.set(None),
            }
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

/// Push the Other Keys page over the list.
fn other_keys(mut shell: Signal<Shell>) {
    shell.write().accounts_pane.path =
        PanePath::new(AccountsPage::List).pushed(AccountsPage::OtherKeys);
}

/// What leads an account's row: its provider's mark, as the provider-marks setting draws it, or
/// the mark of mail kept on this computer.
fn mark_of(row: &AccountRow) -> RowLeading {
    let provider = mail_core::provider::provider(&row.plan);
    let marks = Marks::from(crate::ui::prefs::current().window.provider_marks);
    let quire = if row.is_local() {
        MarkProvider::Local
    } else {
        provider_chip::mark_of(provider)
    };
    let style = provider_chip::mark_style(provider, marks, MarkAt::List);
    RowLeading::Element(rsx! {
        ProviderMark { provider: quire, size: ControlSize::Large, style }
    })
}
