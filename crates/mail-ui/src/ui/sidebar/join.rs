//! The "+" after a Space's account tiles: the accounts this computer has that the Space does not
//! show yet, to bring one in, and Add Account… for one it does not have at all.
//!
//! A Space that shows every account there is has nothing to bring in, and its "+" goes straight
//! to adding a new one.

use crate::ui::menu::{MenuItem, Tile};
use mail_ui_model::copy;
use porter_core::AccountId;

/// The menu's key for Add Account….
pub(super) const NEW: &str = "new";

/// What pressing "+" does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Plus {
    /// Nothing to bring in: add a new account.
    AddNew,
    /// These accounts, by id and name, can join; the menu offers them first.
    Offer(Vec<(AccountId, String)>),
}

/// What "+" does for a Space showing `shown`, given every account as `(id, name)`.
pub(super) fn plus(shown: &[AccountId], accounts: &[(AccountId, String)]) -> Plus {
    let outside: Vec<(AccountId, String)> = accounts
        .iter()
        .filter(|(id, _)| !shown.contains(id))
        .cloned()
        .collect();
    if outside.is_empty() {
        Plus::AddNew
    } else {
        Plus::Offer(outside)
    }
}

/// The menu's rows: each account that can join, by its avatar in `color`, then Add Account….
pub(super) fn items(
    outside: &[(AccountId, String)],
    color: impl Fn(&str) -> String,
) -> Vec<MenuItem> {
    let row = |key: String, tile: Tile, name: String, group: &str| MenuItem {
        tile,
        group: Some(group.to_owned()),
        ..MenuItem::plain(key, name)
    };
    let mut rows: Vec<MenuItem> = outside
        .iter()
        .map(|(id, name)| {
            row(
                id.to_string(),
                Tile::Avatar {
                    letter: super::panes::initial(name),
                    color: color(name),
                },
                name.clone(),
                copy::headers::SHOW_HERE,
            )
        })
        .collect();
    rows.push(row(
        NEW.to_owned(),
        Tile::Icon(ds::prelude::Icon::Plus),
        copy::menus::ADD_ACCOUNT.to_owned(),
        copy::headers::NEW_ACCOUNT,
    ));
    rows
}

/// The account a picked key names, among those offered. `None` for Add Account… and for a key
/// that is no longer offered.
pub(super) fn picked(key: &str, outside: &[(AccountId, String)]) -> Option<AccountId> {
    outside
        .iter()
        .map(|(id, _)| id.clone())
        .find(|id| id.to_string() == key)
}

#[cfg(test)]
mod tests {
    use super::{NEW, Plus, items, picked, plus};
    use porter_core::AccountId;
    use uuid::Uuid;

    fn account(n: u128) -> (AccountId, String) {
        (
            mail_domain::id::account_id_from_uuid(Uuid::from_u128(n)),
            format!("a{n}@example.test"),
        )
    }

    fn ids(ns: &[u128]) -> Vec<AccountId> {
        ns.iter().map(|n| account(*n).0).collect()
    }

    #[test]
    fn plus_offers_the_accounts_the_space_does_not_show() {
        let all = [account(1), account(2), account(3)];
        // (the Space's accounts, what "+" does)
        let cases = [
            (ids(&[1, 2, 3]), Plus::AddNew),
            (ids(&[1]), Plus::Offer(vec![account(2), account(3)])),
            (ids(&[]), Plus::Offer(all.to_vec())),
        ];
        for (shown, expect) in cases {
            assert_eq!(plus(&shown, &all), expect, "{shown:?}");
        }
    }

    #[test]
    fn the_menu_lists_them_then_add_new_and_a_pick_names_one_of_them() {
        let outside = [account(2), account(3)];
        let rows = items(&outside, |_| "#336699".to_owned());
        let names: Vec<&str> = rows.iter().map(|row| row.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "a2@example.test",
                "a3@example.test",
                mail_ui_model::copy::menus::ADD_ACCOUNT
            ]
        );
        assert_eq!(picked(&rows[1].key, &outside), Some(account(3).0));
        assert_eq!(picked(NEW, &outside), None);
        assert_eq!(picked(&account(9).0.to_string(), &outside), None);
    }
}
