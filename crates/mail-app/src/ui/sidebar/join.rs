//! The "+" after a Space's account tiles: the accounts this computer has that the Space does not
//! show yet, to bring one in, and Add New Account… for one it does not have at all.
//!
//! A Space over every account already shows them all, so its "+" goes straight to Add account,
//! as does that of a Space that holds every account there is.

use crate::ui::menu::{MenuItem, Right, Tile};
use crate::ui::space::{Scope, Space};
use porter_core::AccountId;

/// The menu's key for Add New Account….
pub(super) const NEW: &str = "new";

/// What pressing "+" does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Plus {
    /// Nothing to bring in: open Add account.
    AddNew,
    /// These accounts, by id and name, can join; the menu offers them first.
    Offer(Vec<(AccountId, String)>),
}

/// What "+" does for `space`, given every account as `(id, name)`.
pub(super) fn plus(space: &Space, accounts: &[(AccountId, String)]) -> Plus {
    let outside: Vec<(AccountId, String)> = match &space.payload.scope {
        Scope::All => Vec::new(),
        Scope::Accounts(_) => accounts
            .iter()
            .filter(|(id, _)| !space.payload.scope.shows(id.clone()))
            .cloned()
            .collect(),
    };
    if outside.is_empty() {
        Plus::AddNew
    } else {
        Plus::Offer(outside)
    }
}

/// The menu's rows: each account that can join, by its avatar, then Add New Account….
pub(super) fn items(
    outside: &[(AccountId, String)],
    color: impl Fn(AccountId) -> String,
) -> Vec<MenuItem> {
    let row = |key: String, tile: Tile, name: String, group: &str| MenuItem {
        key,
        tile,
        name,
        help: None,
        right: Right::None,
        group: Some(group.to_owned()),
        marks: Vec::new(),
        title: Vec::new(),
        detail: Vec::new(),
    };
    let mut rows: Vec<MenuItem> = outside
        .iter()
        .map(|(id, name)| {
            row(
                id.to_string(),
                Tile::Avatar {
                    letter: super::foot::initial(name),
                    color: color(id.clone()),
                },
                name.clone(),
                "Show in this Space",
            )
        })
        .collect();
    rows.push(row(
        NEW.to_owned(),
        Tile::Icon(ds::prelude::Icon::Plus),
        "Add New Account\u{2026}".to_owned(),
        "New",
    ));
    rows
}

/// The account a picked key names, among those offered. `None` for Add New Account… and for a
/// key that is no longer offered.
pub(super) fn picked(key: &str, outside: &[(AccountId, String)]) -> Option<AccountId> {
    outside
        .iter()
        .map(|(id, _)| id.clone())
        .find(|id| id.to_string() == key)
}

#[cfg(test)]
mod tests {
    use super::{NEW, Plus, items, picked, plus};
    use crate::ui::space::{Scope, Space};
    use porter_core::AccountId;
    use uuid::Uuid;

    fn account(n: u128) -> (AccountId, String) {
        (
            mail_domain::id::account_id_from_uuid(Uuid::from_u128(n)),
            format!("a{n}@example.test"),
        )
    }

    fn space(scope: Scope) -> Space {
        crate::ui::space::built(
            vec![(
                String::new(),
                ds::prelude::SpaceLook::default(),
                crate::ui::space::Mail::over(scope),
            )],
            0,
        )
        .current()
        .clone()
    }

    #[test]
    fn plus_offers_the_accounts_the_space_does_not_show() {
        let all = [account(1), account(2), account(3)];
        let of = |ns: &[u128]| Scope::Accounts(ns.iter().map(|n| account(*n).0).collect());
        // (scope, what "+" does)
        let cases = [
            (Scope::All, Plus::AddNew),
            (of(&[1, 2, 3]), Plus::AddNew),
            (of(&[1]), Plus::Offer(vec![account(2), account(3)])),
            (of(&[]), Plus::Offer(all.to_vec())),
        ];
        for (scope, expect) in cases {
            assert_eq!(plus(&space(scope.clone()), &all), expect, "{scope:?}");
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
                "Add New Account\u{2026}"
            ]
        );
        assert_eq!(picked(&rows[1].key, &outside), Some(account(3).0));
        assert_eq!(picked(NEW, &outside), None);
        assert_eq!(picked(&account(9).0.to_string(), &outside), None);
    }
}
