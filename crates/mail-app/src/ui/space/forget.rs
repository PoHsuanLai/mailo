//! An account removed from this computer leaves every Space: their scopes, their colours and
//! where each was left.
//!
//! A Space limited to that account alone is limited to no account afterwards, and shows nothing
//! until one is added to it: accounts belong to Spaces, so the Space does not widen itself to
//! everyone else's mail. The other accounts' colours stay as they were stored.

use super::{Scope, SpaceId, Spaces};
use porter_core::AccountId;

/// Whether forgetting an account changed the Spaces, and so whether they need writing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Forgot {
    /// No Space named the account.
    Nothing,
    /// At least one Space did, and no longer does.
    Changed,
}

/// Take `account` out of every Space in `spaces`.
pub fn forget_account(spaces: &mut Spaces, account: AccountId) -> Forgot {
    let mut forgot = Forgot::Nothing;
    let ids: Vec<SpaceId> = spaces.list().iter().map(|space| space.id).collect();
    for id in ids {
        spaces.edit(id, |space| {
            let mail = &mut space.payload;
            if let Scope::Accounts(ids) = &mut mail.scope
                && ids.contains(&account)
            {
                ids.retain(|id| *id != account);
                forgot = Forgot::Changed;
            }
            if mail.colors.remove(&account).is_some() {
                forgot = Forgot::Changed;
            }
        });
        // Only a Space left on the account is touched: one never left has no recall to write.
        if spaces.recall().of(id).account.as_ref() == Some(&account) {
            spaces.edit_recall(id, |recall| recall.account = None);
            forgot = Forgot::Changed;
        }
    }
    forgot
}

/// Take every account not in `known` out of every Space: what [`forget_account`] does for one,
/// for each the Spaces name that the store no longer has, removed here or from a terminal.
pub fn forget_unknown(spaces: &mut Spaces, known: &[AccountId]) -> Forgot {
    let mut named: Vec<AccountId> = Vec::new();
    for space in spaces.list() {
        if let Scope::Accounts(ids) = &space.payload.scope {
            named.extend(ids.iter().cloned());
        }
        named.extend(space.payload.colors.keys().cloned());
        named.extend(spaces.recall().of(space.id).account);
    }
    named.sort();
    named.dedup();
    named
        .into_iter()
        .filter(|id| !known.contains(id))
        .fold(Forgot::Nothing, |forgot, id| {
            match (forgot, forget_account(spaces, id)) {
                (Forgot::Nothing, Forgot::Nothing) => Forgot::Nothing,
                _ => Forgot::Changed,
            }
        })
}

#[cfg(test)]
mod tests {
    use super::{Forgot, forget_account, forget_unknown};
    use crate::ui::space::{Mail, Recall, Scope, Spaces};
    use porter_core::AccountId;
    use uuid::Uuid;

    fn account(n: u128) -> AccountId {
        mail_domain::id::account_id_from_uuid(Uuid::from_u128(n))
    }

    fn scoped(ids: &[u128], colors: &[u128]) -> Mail {
        Mail {
            scope: Scope::Accounts(ids.iter().copied().map(account).collect()),
            colors: colors
                .iter()
                .map(|n| (account(*n), format!("#{n:06}")))
                .collect(),
            ..Mail::default()
        }
    }

    fn spaces(of: Vec<Mail>) -> Spaces {
        Spaces::first_run(of, Mail::default)
    }

    fn payloads(spaces: &Spaces) -> Vec<Mail> {
        spaces
            .list()
            .iter()
            .map(|space| space.payload.clone())
            .collect()
    }

    #[test]
    fn the_account_leaves_every_space_and_nothing_else_does() {
        // (before, after, what forgetting account 1 reports)
        let cases: Vec<(Mail, Mail, Forgot)> = vec![
            (
                scoped(&[1, 2], &[1, 2]),
                scoped(&[2], &[2]),
                Forgot::Changed,
            ),
            // Its only account gone, the Space is of no account, not of every account.
            (scoped(&[1], &[1]), scoped(&[], &[]), Forgot::Changed),
            // Over every account, only its colour goes.
            (
                Mail {
                    scope: Scope::All,
                    ..scoped(&[], &[1, 2])
                },
                Mail {
                    scope: Scope::All,
                    ..scoped(&[], &[2])
                },
                Forgot::Changed,
            ),
            (
                scoped(&[2, 3], &[2]),
                scoped(&[2, 3], &[2]),
                Forgot::Nothing,
            ),
        ];
        for (before, after, expect) in cases {
            let mut all = spaces(vec![before.clone()]);
            assert_eq!(forget_account(&mut all, account(1)), expect, "{before:?}");
            assert_eq!(payloads(&all), vec![after], "{before:?}");
        }
    }

    #[test]
    fn a_space_left_on_the_account_comes_back_on_every_account() {
        let mut all = spaces(vec![Mail::default(), Mail::default()]);
        let [first, second] = [all.list()[0].id, all.list()[1].id];
        let left = |tile: u128| Recall {
            place: "Inbox".to_owned(),
            open: None,
            account: Some(account(tile)),
        };
        all.edit_recall(first, |recall| *recall = left(1));
        all.edit_recall(second, |recall| *recall = left(2));
        assert_eq!(forget_account(&mut all, account(1)), Forgot::Changed);
        assert_eq!(all.recall().of(first).account, None);
        assert_eq!(all.recall().of(first).place, "Inbox");
        assert_eq!(all.recall().of(second), left(2));
    }

    #[test]
    fn accounts_the_store_no_longer_has_leave_and_known_ones_stay() {
        let mut all = spaces(vec![scoped(&[1, 2], &[1, 2, 3]), scoped(&[3], &[3])]);
        let known = [account(1)];
        assert_eq!(forget_unknown(&mut all, &known), Forgot::Changed);
        assert_eq!(payloads(&all), vec![scoped(&[1], &[1]), scoped(&[], &[])]);
        assert_eq!(forget_unknown(&mut all, &known), Forgot::Nothing);
    }
}
