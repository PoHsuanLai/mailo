use super::{AccountPick, accounts_menu, toggled};
use crate::ui::add_account::AddPick;
use ds::components::menus::item::item::{AfterPick, MenuItem};
use ds::prelude::{Availability, Check};
use porter_core::AccountId;
use uuid::Uuid;

fn account(n: u128) -> AccountId {
    mail_domain::id::account_id_from_uuid(Uuid::from_u128(n))
}

fn ids(ns: &[u128]) -> Vec<AccountId> {
    ns.iter().map(|n| account(*n)).collect()
}

fn named(ns: &[u128]) -> Vec<(AccountId, String)> {
    ns.iter()
        .map(|n| (account(*n), format!("a{n}@example.test")))
        .collect()
}

/// The Accounts rows of the menu, as (name, check, stays open), a rule as `-`, and a submenu
/// as its title with its rows after `>`.
fn rows(items: &[MenuItem<AccountPick>]) -> Vec<(String, Check, bool)> {
    let [
        MenuItem::Submenu {
            title, children, ..
        },
    ] = items
    else {
        panic!("not one Accounts submenu: {items:?}");
    };
    assert_eq!(title, crate::ui::copy::menus::ACCOUNTS);
    children
        .iter()
        .flat_map(|child| match child {
            MenuItem::Item {
                title,
                check,
                after,
                ..
            } => vec![(
                title.clone(),
                check.unwrap_or(Check::Off),
                *after == AfterPick::KeepOpen,
            )],
            MenuItem::Separator => vec![("-".to_owned(), Check::Off, false)],
            MenuItem::Submenu {
                title, children, ..
            } => children
                .iter()
                .map(|row| match row {
                    MenuItem::Item { title: name, .. } => {
                        (format!("{title} > {name}"), Check::Off, false)
                    }
                    MenuItem::Separator => (format!("{title} > -"), Check::Off, false),
                    other => panic!("not an add row: {other:?}"),
                })
                .collect(),
            other => panic!("not an account row: {other:?}"),
        })
        .collect()
}

fn add(title: &str) -> (String, Check, bool) {
    (title.to_owned(), Check::Off, false)
}

#[test]
fn accounts_are_checked_by_the_space_s_accounts_and_stay_open_on_a_pick() {
    let all = named(&[1, 2, 3]);
    let shown = rows(&accounts_menu(&ids(&[1, 3]), &all, &[]));
    assert_eq!(
        shown,
        [
            ("a1@example.test".to_owned(), Check::On, true),
            ("a2@example.test".to_owned(), Check::Off, true),
            ("a3@example.test".to_owned(), Check::On, true),
            add("-"),
            add(crate::ui::copy::menus::ADD_ACCOUNT),
        ]
    );
}

/// Every account may be taken out of the Space or put in it: one may be in no Space at all.
#[test]
fn any_account_may_leave_the_space() {
    let menu = accounts_menu(&ids(&[1, 2]), &named(&[1, 2, 3]), &[]);
    let [MenuItem::Submenu { children, .. }] = menu.as_slice() else {
        panic!("not one Accounts submenu");
    };
    let open: Vec<(String, Availability)> = children
        .iter()
        .filter_map(|row| match row {
            MenuItem::Item {
                title,
                availability,
                value: AccountPick::Toggle(_),
                ..
            } => Some((title.clone(), *availability)),
            _ => None,
        })
        .collect();
    assert_eq!(
        open,
        [
            ("a1@example.test".to_owned(), Availability::Enabled),
            ("a2@example.test".to_owned(), Availability::Enabled),
            ("a3@example.test".to_owned(), Availability::Enabled),
        ]
    );
}

#[test]
fn add_account_ends_the_accounts_menu() {
    assert_eq!(
        rows(&accounts_menu(&[], &[], &[])),
        [add(crate::ui::copy::menus::ADD_ACCOUNT)]
    );
    let google = (porter_provider::Issuer::Google, "Google".to_owned());
    assert_eq!(
        rows(&accounts_menu(&ids(&[1]), &named(&[1]), &[google])),
        [
            ("a1@example.test".to_owned(), Check::On, true),
            add("-"),
            add("Add Account > Google"),
            add("Add Account > -"),
            add("Add Account > Other Account\u{2026}"),
        ]
    );
    let menu = accounts_menu(&[], &[], &[]);
    let [MenuItem::Submenu { children, .. }] = menu.as_slice() else {
        panic!("not one Accounts submenu");
    };
    let picks: Vec<AccountPick> = children
        .iter()
        .filter_map(|row| match row {
            MenuItem::Item { value, .. } => Some(value.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(picks, [AccountPick::Add(AddPick::Other)]);
}

#[test]
fn a_pick_takes_an_account_out_or_puts_it_in() {
    // (the Space's accounts, the account picked, its accounts after)
    let cases = [
        (ids(&[1]), 2, ids(&[1, 2])),
        (ids(&[1, 2]), 2, ids(&[1])),
        // Every account is only a list of them: one added later is not in it.
        (ids(&[1, 3]), 2, ids(&[1, 3, 2])),
    ];
    for (accounts, n, after) in cases {
        assert_eq!(toggled(&accounts, account(n)), after, "{accounts:?} {n}");
    }
}
