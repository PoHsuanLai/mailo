use super::{accounts_menu, toggled};
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

/// The Accounts rows of the menu, as (name, check, stays open).
fn rows(items: &[MenuItem<AccountId>]) -> Vec<(String, Check, bool)> {
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
        .map(|child| match child {
            MenuItem::Item {
                title,
                check,
                after,
                ..
            } => (
                title.clone(),
                check.unwrap_or(Check::Off),
                *after == AfterPick::KeepOpen,
            ),
            other => panic!("not an account row: {other:?}"),
        })
        .collect()
}

#[test]
fn accounts_are_checked_by_the_space_s_scope_and_stay_open_on_a_pick() {
    let all = named(&[1, 2, 3]);
    let shown = rows(&accounts_menu(
        &Scope::Accounts(vec![account(1), account(3)]),
        &all,
    ));
    assert_eq!(
        shown,
        [
            ("a1@example.test".to_owned(), Check::On, true),
            ("a2@example.test".to_owned(), Check::Off, true),
            ("a3@example.test".to_owned(), Check::On, true),
        ]
    );
    let every = rows(&accounts_menu(&Scope::All, &all));
    assert!(
        every.iter().all(|(_, check, _)| *check == Check::On),
        "{every:?}"
    );
    assert!(
        accounts_menu(&Scope::All, &[]).is_empty(),
        "a menu with no accounts"
    );
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
