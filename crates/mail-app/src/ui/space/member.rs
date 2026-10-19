//! An account put in a Space, or taken out of it: the Space's menu (Accounts).
//!
//! A Space names its accounts, so this is the list with one more or one fewer; the others stay
//! where they are.

use porter_core::AccountId;

/// Whether an account is to be in a Space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Member {
    In,
    Out,
}

/// `accounts` with `account` put in (last, when it is not there yet) or taken out.
pub fn with_member(accounts: &[AccountId], account: AccountId, member: Member) -> Vec<AccountId> {
    match member {
        Member::In if accounts.contains(&account) => accounts.to_vec(),
        Member::In => accounts.iter().cloned().chain([account]).collect(),
        Member::Out => accounts
            .iter()
            .filter(|&id| *id != account)
            .cloned()
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::{Member, with_member};
    use porter_core::AccountId;
    use uuid::Uuid;

    fn account(n: u128) -> AccountId {
        mail_domain::id::account_id_from_uuid(Uuid::from_u128(n))
    }

    fn of(ns: &[u128]) -> Vec<AccountId> {
        ns.iter().map(|n| account(*n)).collect()
    }

    #[test]
    fn an_account_goes_in_or_out_and_the_others_stay() {
        // (accounts, account, in or out, the accounts after)
        let cases = [
            (of(&[1]), 2, Member::In, of(&[1, 2])),
            (of(&[1, 2]), 2, Member::In, of(&[1, 2])),
            (of(&[1, 2]), 2, Member::Out, of(&[1])),
            // Every account is only a list of them: one added later is not in it.
            (of(&[1, 3]), 2, Member::In, of(&[1, 3, 2])),
            (of(&[2]), 2, Member::Out, of(&[])),
            (of(&[]), 3, Member::In, of(&[3])),
            (of(&[1]), 3, Member::Out, of(&[1])),
        ];
        for (accounts, n, member, expect) in cases {
            assert_eq!(
                with_member(&accounts, account(n), member),
                expect,
                "{accounts:?} {member:?} {n}"
            );
        }
    }
}
