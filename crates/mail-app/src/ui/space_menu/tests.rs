use super::{AccountPick, accounts_menu, toggled};
use crate::ui::add_account::AddPick;
use crate::ui::space::Scope;
use ds::components::menus::item::item::{AfterPick, MenuItem};
use ds::prelude::Check;
use porter_core::AccountId;
use uuid::Uuid;

fn account(n: u128) -> AccountId {
    mail_domain::id::account_id_from_uuid(Uuid::from_u128(n))
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
    assert_eq!(title, "Accounts");
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
fn accounts_are_checked_by_the_space_s_scope_and_stay_open_on_a_pick() {
    let all = named(&[1, 2, 3]);
    let shown = rows(&accounts_menu(
        &Scope::Accounts(vec![account(1), account(3)]),
        &all,
        &[],
    ));
    assert_eq!(
        shown,
        [
            ("a1@example.test".to_owned(), Check::On, true),
            ("a2@example.test".to_owned(), Check::Off, true),
            ("a3@example.test".to_owned(), Check::On, true),
            add("-"),
            add("Add Account\u{2026}"),
        ]
    );
    let every = rows(&accounts_menu(&Scope::All, &all, &[]));
    assert!(
        every[..3].iter().all(|(_, check, _)| *check == Check::On),
        "{every:?}"
    );
}

/// Add Account lives in the Space's menu › Accounts: the window itself with no provider that
/// signs in in the browser, else a submenu of those and Other Account…. With no account yet it
/// is all Accounts holds.
#[test]
fn add_account_ends_the_accounts_menu() {
    assert_eq!(
        rows(&accounts_menu(&Scope::All, &[], &[])),
        [add("Add Account\u{2026}")]
    );
    let google = (porter_provider::Issuer::Google, "Google".to_owned());
    assert_eq!(
        rows(&accounts_menu(&Scope::All, &named(&[1]), &[google])),
        [
            ("a1@example.test".to_owned(), Check::On, true),
            add("-"),
            add("Add Account > Google"),
            add("Add Account > -"),
            add("Add Account > Other Account\u{2026}"),
        ]
    );
    let menu = accounts_menu(&Scope::All, &[], &[]);
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
    let all = [account(1), account(2), account(3)];
    let of = |ns: &[u128]| Scope::Accounts(ns.iter().map(|n| account(*n)).collect());
    // (scope, the account picked, the scope after)
    let cases = [
        (Scope::All, 2, of(&[1, 3])),
        (of(&[1]), 2, of(&[1, 2])),
        (of(&[1, 2]), 2, of(&[1])),
        // Every account again is every account, so one added later shows too.
        (of(&[1, 3]), 2, Scope::All),
    ];
    for (scope, n, after) in cases {
        assert_eq!(toggled(&scope, account(n), &all), after, "{scope:?} {n}");
    }
}
