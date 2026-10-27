//! Settings' Vacation Reply: a pane of two levels, as Spaces is. Its root lists each account by
//! its provider's mark and its address, with whether its reply is on; a row pushes that account's
//! reply ([`AwayPage`]) as a page of its own, its back button the way home.

use chrono::{DateTime, Utc};
use dioxus::prelude::*;
use ds::components::fields::field_row::FieldRow;
use ds::components::lists::list::model::ListStyle;
use ds::components::lists::row::size::RowSize;
use ds::prelude::*;
use ds::root::common::Common;
use mail_client::Client;
use mail_client::when::Clock;

use super::super::common::Told;
use super::super::data::{AccountRow, account_rows};
use super::super::menus::when_words;
use super::away::AwayPage;
use super::server::reach;
use super::{Said, push};
use crate::ui::copy;
use crate::ui::view::{Shell, VacationAt};

/// The stack's element id: its back buttons are `vacation-pane-back-root` and `-detail`.
const STACK: &str = "vacation-pane";

/// The element id of the row that pushes `at`, where the keyboard goes back to.
fn row_id(at: &VacationAt) -> Option<String> {
    match at {
        VacationAt::List => None,
        VacationAt::Account(id) => Some(format!("vacation-pane-open-{id}")),
    }
}

/// Back to the accounts.
pub(in crate::ui) fn back(mut shell: Signal<Shell>) {
    let back = shell.peek().vacation_pane.popped();
    shell.write().vacation_pane = back;
}

/// The accounts a reply can be set on: every one with a server.
fn replying(client: &Client) -> Vec<AccountRow> {
    account_rows(client)
        .into_iter()
        .filter(|row| !row.is_local())
        .collect()
}

/// The pane: the accounts at its root, one account's reply over them.
#[component]
pub(in crate::ui) fn VacationPage(shell: Signal<Shell>) -> Element {
    let client = crate::ui::live::client();
    let rows = replying(&client);
    let unpushed: Said = use_signal(|| None);
    let pushed = use_callback(move |row: AccountRow| push(row, unpushed));
    if rows.is_empty() {
        return rsx! {
            PageHeader { title: copy::titles::VACATION_REPLY }
            Form {
                FormSection {
                    FieldRow { label: copy::empty::ADD_AN_ACCOUNT_FIRST }
                }
            }
        };
    }
    let path = shell.read().vacation_pane.clone();
    let named = rows.clone();
    let title = use_callback(move |at: VacationAt| match at {
        VacationAt::List => copy::titles::VACATION_REPLY.to_owned(),
        VacationAt::Account(id) => named.iter().find(|row| row.id == id).map_or_else(
            || copy::titles::VACATION_REPLY.to_owned(),
            AccountRow::shown,
        ),
    });
    let page = use_callback(move |at: VacationAt| match at {
        VacationAt::List => rsx! { VacationList { shell, rows: rows.clone() } },
        VacationAt::Account(id) => match rows.iter().find(|row| row.id == id).cloned() {
            // Read again for another account.
            Some(row) => rsx! { AwayPage { key: "{row.id}", row, pushed } },
            None => rsx! {},
        },
    });
    let opener = use_callback(|at: VacationAt| row_id(&at));
    rsx! {
        PaneStack::<VacationAt> {
            path,
            title,
            page,
            opener: Some(opener),
            on_back: move |()| back(shell),
            common: Common { id: Some(STACK.to_owned()), ..Common::default() },
        }
        Told { title: copy::titles::NOT_ON_SERVER, said: unpushed }
    }
}

/// The root: each account, and whether its reply is on.
#[component]
fn VacationList(shell: Signal<Shell>, rows: Vec<AccountRow>) -> Element {
    // A reply kept, here or anywhere, moves the store.
    let _ = crate::ui::live::moved();
    let client = crate::ui::live::client();
    let now = Utc::now();
    let clock = crate::ui::clock::hours();
    let items: Vec<ListItem<String>> = rows
        .into_iter()
        .map(|row| {
            let at = VacationAt::Account(row.id.clone());
            let shown = row.shown();
            let state = status(&client, &row, now, clock);
            ListItem::row(
                row.id.to_string(),
                shown.clone(),
                rsx! {
                    Row {
                        leading: crate::ui::provider_chip::account_mark(&row),
                        title: shown.clone(),
                        accessory: Accessory::Text(state),
                        size: RowSize::Settings,
                        onclick: move |_| {
                            let mut shell = shell;
                            shell.write().vacation_pane = PanePath::new(VacationAt::List).pushed(at.clone());
                        },
                        common: Common {
                            id: row_id(&VacationAt::Account(row.id.clone())),
                            aria_label: Some(copy::aria::details_for(&shown)),
                            ..Common::default()
                        },
                    }
                },
            )
        })
        .collect();
    rsx! {
        Form {
            FormSection {
                List::<String> { label: copy::titles::VACATION_REPLY, items, style: ListStyle::Grouped }
            }
        }
    }
}

/// What an account's row says of its reply: off, on, on until or from a time, ended, or that its
/// server cannot send one.
fn status(client: &Client, row: &AccountRow, now: DateTime<Utc>, clock: Clock) -> String {
    use copy::status::vacation as words;
    if reach(&row.plan).is_err() {
        return words::UNAVAILABLE.to_owned();
    }
    // A reply the client cannot read is not known to be off.
    let kept = match client.stored().vacation(row.id.clone()) {
        Ok(Some(kept)) => kept,
        Ok(None) => return words::OFF.to_owned(),
        Err(_) => return words::UNAVAILABLE.to_owned(),
    };
    let at = |t| when_words(t, now, &chrono::Local, clock);
    match (kept.during.from, kept.during.to) {
        (_, Some(end)) if end <= now => words::ENDED.to_owned(),
        (Some(start), _) if now < start => words::from(&at(start)),
        (_, Some(end)) => words::until(&at(end)),
        _ => words::ON.to_owned(),
    }
}
