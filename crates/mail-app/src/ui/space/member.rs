//! An account put in a Space, or taken out of it: the Space editor's Accounts row, and the menu
//! on an account's tile.
//!
//! A Space over every account that loses one becomes a Space of all the others, so taking an
//! account out never also takes the rest; putting it back, so that the Space holds every account
//! again, makes it a Space over every account once more. Putting one into a Space over every
//! account changes nothing: it is already there.

use super::Scope;
use porter_core::AccountId;

/// Whether an account is to be in a Space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Member {
    In,
    Out,
}

/// `scope` with `account` put in or taken out. `all` is every account there is, in order, which
/// a Space over every account becomes when one leaves it.
pub fn with_member(scope: &Scope, account: AccountId, member: Member, all: &[AccountId]) -> Scope {
    match (scope, member) {
        (Scope::All, Member::In) => Scope::All,
        (Scope::All, Member::Out) => {
            Scope::Accounts(all.iter().filter(|&id| *id != account).cloned().collect())
        }
        (Scope::Accounts(ids), Member::In) if ids.contains(&account) => scope.clone(),
        (Scope::Accounts(ids), Member::In) => {
            let ids: Vec<AccountId> = ids.iter().cloned().chain([account]).collect();
            // Every account again is a Space over every account, so one added later shows too.
            if all.iter().all(|id| ids.contains(id)) {
                Scope::All
            } else {
                Scope::Accounts(ids)
            }
        }
        (Scope::Accounts(ids), Member::Out) => {
            Scope::Accounts(ids.iter().filter(|&id| *id != account).cloned().collect())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Member, with_member};
    use crate::ui::space::Scope;
    use porter_core::AccountId;
    use uuid::Uuid;

    fn account(n: u128) -> AccountId {
        mail_domain::id::account_id_from_uuid(Uuid::from_u128(n))
    }

    fn of(ns: &[u128]) -> Scope {
        Scope::Accounts(ns.iter().map(|n| account(*n)).collect())
    }

    #[test]
    fn an_account_goes_in_or_out_and_the_others_stay() {
        let all = [account(1), account(2), account(3)];
        // (scope, account, in or out, the scope after)
        let cases = [
            (Scope::All, 2, Member::In, Scope::All),
            (Scope::All, 2, Member::Out, of(&[1, 3])),
            (of(&[1]), 2, Member::In, of(&[1, 2])),
            (of(&[1, 2]), 2, Member::In, of(&[1, 2])),
            (of(&[1, 2]), 2, Member::Out, of(&[1])),
            // Out and back in: every account again is every account, not a fixed list.
            (of(&[1, 3]), 2, Member::In, Scope::All),
            (of(&[2]), 2, Member::Out, of(&[])),
            (of(&[]), 3, Member::In, of(&[3])),
            (of(&[1]), 3, Member::Out, of(&[1])),
        ];
        for (scope, n, member, expect) in cases {
            assert_eq!(
                with_member(&scope, account(n), member, &all),
                expect,
                "{scope:?} {member:?} {n}"
            );
        }
    }
}
