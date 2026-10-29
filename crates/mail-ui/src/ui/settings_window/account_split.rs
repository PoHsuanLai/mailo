//! The layout every per-account page of Settings shares, as Apple Mail's Accounts and Signatures
//! panes have it: the accounts in a list on the left, and on the right what the page keeps for
//! the one chosen (Accounts, Signatures, Rules, Vacation Reply). A page gives [`AccountSplit`] its
//! accounts, which one is chosen and what to do when another is picked, then draws the account's
//! own content as its children, keyed by the account so that what a page of it held (a field
//! half typed, a rule's page pushed over its list) is not carried to the next.
//!
//! The list always shows, even with one account. The page keeps which is chosen in the window's
//! `Shell`, as [`shown`] reads it: the one that was held while it is still there, else the first.
//! Anything a page pushes (a rule's page, a signature's) is pushed in the right pane, under the
//! list, which stays put. A page's own list actions (Accounts' Add Account…) stand under the
//! accounts in the `under` slot.
//!
//! With no account at all there is nothing to choose between: [`NoAccounts`] stands in for the
//! pair, saying so with an Add Account… to start from, as the Notifications switches say it.

use crate::ui::add_account::Join;
use crate::ui::copy;
use crate::ui::data::AccountRow;
use crate::ui::press::on_primary;
use dioxus::prelude::*;
use ds::base::vocab::RowState;
use ds::components::chrome::split_view::model::{Collapsing, PaneSize, PaneSpec, SplitPane};
use ds::components::chrome::split_view::view::SplitView;
use ds::components::fields::field_row::FieldRow;
use ds::components::lists::list::model::ListStyle;
use ds::host::measure::MountedRef;
use ds::prelude::*;
use ds::root::common::Common;
use ds::style::tokens::control_size::ControlSize;
use porter_core::AccountId;

/// The list's pane: 230 wide, 180 to 320, and it does not fold.
const LIST: PaneSpec = PaneSpec {
    preferred: PaneSize::Fixed(Px(230.0)),
    min: PaneSize::Fixed(Px(180.0)),
    max: PaneSize::Fixed(Px(320.0)),
    collapsing: Collapsing::Never,
};

/// The account of `rows` a page shows: `held` while it is still there, else the first.
pub(in crate::ui) fn shown<'a>(
    rows: &'a [AccountRow],
    held: Option<&AccountId>,
) -> Option<&'a AccountRow> {
    held.and_then(|held| rows.iter().find(|row| row.id == *held))
        .or_else(|| rows.first())
}

/// The element id of the row that picks `account` on the page whose panes are `pane`.
pub(in crate::ui) fn row_id(pane: &str, account: &AccountId) -> String {
    format!("{pane}-open-{account}")
}

/// The accounts `rows` in a list beside `children`. `pane` names the page for its elements'
/// ids; `chosen` is the account highlighted (none while the page shows something that is no
/// account's); `onpick` hears another. `under` stands beneath the list.
#[component]
pub(in crate::ui) fn AccountSplit(
    pane: &'static str,
    label: &'static str,
    rows: Vec<AccountRow>,
    chosen: Option<AccountId>,
    onpick: EventHandler<AccountId>,
    #[props(default)] under: Option<Element>,
    children: Element,
) -> Element {
    let picked = rows.clone();
    let items: Vec<ListItem<String>> = rows
        .iter()
        .map(|row| {
            let shown = row.shown();
            let selection = if chosen.as_ref() == Some(&row.id) {
                Selection::Selected
            } else {
                Selection::Unselected
            };
            // The address under a name the person gave: where the name is the address, once is
            // enough.
            let detail = (shown != row.address && !row.is_local())
                .then(|| TextLine::from(row.address.clone()));
            let id = row.id.clone();
            ListItem::row(
                row.id.to_string(),
                shown.clone(),
                rsx! {
                    Row {
                        leading: crate::ui::provider_chip::account_mark(row),
                        title: shown.clone(),
                        detail,
                        state: RowState { selection, ..RowState::default() },
                        onclick: move |_| onpick.call(id.clone()),
                        common: Common {
                            id: Some(row_id(pane, &row.id)),
                            aria_label: Some(shown.clone()),
                            ..Common::default()
                        },
                    }
                },
            )
        })
        .collect();
    let list = rsx! {
        div { class: "account-list",
            List::<String> {
                label,
                items,
                style: ListStyle::Plain,
                cursor: chosen.as_ref().map(ToString::to_string),
                onselect: move |key: String| {
                    if let Some(row) = picked.iter().find(|row| row.id.to_string() == key) {
                        onpick.call(row.id.clone());
                    }
                },
            }
            if let Some(under) = under {
                {under}
            }
        }
    };
    rsx! {
        SplitView {
            label,
            panes: vec![SplitPane::new(LIST, list)],
            div { class: "account-detail", {children} }
        }
    }
}

/// The Add Account… the pages that need an account offer: its press, the menu of the providers
/// that sign in straight in the browser while it is open, and the element it hangs from.
#[derive(Clone, Copy)]
pub(in crate::ui) struct Adding {
    offered: Signal<Option<Vec<(porter_provider::Issuer, String)>>>,
    anchor: Signal<Option<MountedRef>>,
}

/// An [`Adding`] for the component that calls it; draw its [`Adding::menu`] beside the element
/// that is given [`Adding::mounted`] and starts it.
pub(in crate::ui) fn use_adding() -> Adding {
    Adding {
        offered: use_signal(|| None),
        anchor: use_signal(|| None),
    }
}

impl Adding {
    /// Start adding an account that joins no Space: Settings is every account, in no Space.
    pub(in crate::ui) fn start(mut self) {
        crate::ui::add_account::add_from(Join::Nowhere, move |offered| {
            self.offered.set(Some(offered))
        });
    }

    /// The `mounted` of the element the menu hangs from.
    pub(in crate::ui) fn mounted(mut self) -> EventHandler<MountedEvent> {
        EventHandler::new(move |event: MountedEvent| {
            self.anchor.set(Some(MountedRef(event.data())));
        })
    }

    /// The menu while it is open.
    pub(in crate::ui) fn menu(mut self) -> Element {
        let Some(redirects) = (self.offered)() else {
            return rsx! {};
        };
        rsx! {
            crate::ui::add_account::AddMenu {
                anchor: (self.anchor)(),
                redirects,
                join: Join::Nowhere,
                on_close: move |()| self.offered.set(None),
            }
        }
    }
}

/// The row that says there is no account yet, with Add Account… beside it: for a section of a
/// page that has no account to show.
#[component]
pub(in crate::ui) fn NoAccounts() -> Element {
    let adding = use_adding();
    rsx! {
        FieldRow { label: copy::empty::ADD_AN_ACCOUNT_FIRST,
            Button {
                label: copy::controls::ADD_ACCOUNT,
                size: ControlSize::Small,
                common: Common {
                    aria_label: Some(copy::aria::ADD_ACCOUNT.to_owned()),
                    mounted: Some(adding.mounted()),
                    ..Common::default()
                },
                onclick: on_primary(move || adding.start()),
            }
        }
        {adding.menu()}
    }
}

/// A page with no account: its title, and [`NoAccounts`] in a form of its own.
#[component]
pub(in crate::ui) fn NoAccountsPage(title: &'static str) -> Element {
    rsx! {
        PageHeader { title }
        Form {
            FormSection { NoAccounts {} }
        }
    }
}
