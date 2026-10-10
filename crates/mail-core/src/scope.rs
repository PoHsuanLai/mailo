//! Which accounts a view of the mail covers.
//!
//! A Space in the window is one, the launcher's unread count is asked over one, and the command
//! line asks over every account ([`Scope::All`]). It is a choice of accounts, not a drawing, so it
//! lives here where each front end can name it without the other.

use mail_domain::Filter;
use porter_core::AccountId;
use serde::{Deserialize, Serialize};

/// Which accounts a view shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "kind", content = "v", rename_all = "snake_case")]
pub enum Scope {
    /// Every account, in one list.
    #[default]
    All,
    /// These accounts and no others. None at all is a view that shows nothing, which is what
    /// a Space whose accounts were all removed is until one is added to it.
    Accounts(Vec<AccountId>),
}

impl Scope {
    /// Whether `account`'s mail is in it.
    pub fn shows(&self, account: AccountId) -> bool {
        match self {
            Scope::All => true,
            Scope::Accounts(ids) => ids.contains(&account),
        }
    }

    /// Only `pressed`, when an account tile is pressed; else this.
    pub fn narrowed(&self, pressed: Option<AccountId>) -> Scope {
        match pressed {
            Some(id) => Scope::Accounts(vec![id]),
            None => self.clone(),
        }
    }

    /// As a store filter. `None` is every account, and no accounts is [`Filter::Nothing`].
    pub fn filter(&self) -> Option<Filter> {
        match self {
            Scope::All => None,
            Scope::Accounts(ids) => Some(match ids.as_slice() {
                [] => Filter::Nothing,
                [one] => Filter::Account(one.clone()),
                many => Filter::Or(many.iter().cloned().map(Filter::Account).collect()),
            }),
        }
    }

    /// `filter`, narrowed to `within`: the accounts of this scope, and what `within` selects.
    pub fn over(&self, within: Filter) -> Filter {
        match self.filter() {
            None => within,
            Some(accounts) => Filter::And(vec![accounts, within]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mail_domain::id::account_id_from_uuid;

    fn account(n: u128) -> AccountId {
        account_id_from_uuid(uuid::Uuid::from_u128(n))
    }

    #[test]
    fn a_scope_shows_the_accounts_it_names_and_every_account_when_it_names_none() {
        let some = Scope::Accounts(vec![account(1), account(2)]);
        assert!(Scope::All.shows(account(9)));
        assert!(some.shows(account(2)));
        assert!(!some.shows(account(3)));
        assert!(!Scope::Accounts(Vec::new()).shows(account(1)));
    }

    #[test]
    fn a_pressed_account_narrows_the_scope_and_nothing_pressed_leaves_it() {
        let some = Scope::Accounts(vec![account(1), account(2)]);
        assert_eq!(some.narrowed(None), some);
        assert_eq!(
            some.narrowed(Some(account(2))),
            Scope::Accounts(vec![account(2)])
        );
        assert_eq!(
            Scope::All.narrowed(Some(account(1))),
            Scope::Accounts(vec![account(1)])
        );
    }

    #[test]
    fn a_scope_is_a_store_filter_that_agrees_with_what_it_shows() {
        assert_eq!(Scope::All.filter(), None);
        assert_eq!(Scope::Accounts(Vec::new()).filter(), Some(Filter::Nothing));
        assert_eq!(
            Scope::Accounts(vec![account(1)]).filter(),
            Some(Filter::Account(account(1)))
        );
        assert_eq!(
            Scope::Accounts(vec![account(1), account(2)]).filter(),
            Some(Filter::Or(vec![
                Filter::Account(account(1)),
                Filter::Account(account(2)),
            ]))
        );
    }

    #[test]
    fn a_filter_over_a_scope_is_the_filter_alone_for_every_account() {
        let within = Filter::Pinned;
        assert_eq!(Scope::All.over(within.clone()), within);
        assert_eq!(
            Scope::Accounts(vec![account(1)]).over(within.clone()),
            Filter::And(vec![Filter::Account(account(1)), within])
        );
    }

    #[test]
    fn a_scope_reads_and_writes_as_it_always_did() {
        let json = serde_json::to_string(&Scope::All).unwrap();
        assert_eq!(json, r#"{"kind":"all"}"#);
        let some = Scope::Accounts(vec![account(1)]);
        let back: Scope = serde_json::from_str(&serde_json::to_string(&some).unwrap()).unwrap();
        assert_eq!(back, some);
    }
}
