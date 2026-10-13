//! An account removed from this computer leaves every Space: their scopes, their colours and
//! where each was left.
//!
//! A Space limited to that account alone is limited to no account afterwards, and shows nothing
//! until one is added to it: accounts belong to Spaces, so the Space does not widen itself to
//! everyone else's mail. The other accounts' colours stay as they were stored.

use super::{Mail, Scope, SpaceId, Spaces};
use mail_core::profile::ProfileId;
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
    forget_where(spaces, account, |_| true)
}

/// Take `account` out of the Spaces whose mail `of` accepts, and leave the others as they are.
fn forget_where(spaces: &mut Spaces, account: AccountId, of: impl Fn(&Mail) -> bool) -> Forgot {
    let mut forgot = Forgot::Nothing;
    let ids: Vec<SpaceId> = spaces.list().iter().map(|space| space.id).collect();
    for id in ids {
        if spaces.get(id).is_none_or(|space| !of(&space.payload)) {
            continue;
        }
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

/// Take every account not in `known` out of every Space of `profile`: what [`forget_account`]
/// does for one, for each the Spaces name that the store no longer has, removed here or from a
/// terminal. `known` is the accounts of `profile`'s store, so a Space of another Profile, whose
/// accounts are in a store not open here, is left exactly as it is.
pub fn forget_unknown(spaces: &mut Spaces, profile: &ProfileId, known: &[AccountId]) -> Forgot {
    let mut named: Vec<AccountId> = Vec::new();
    for space in spaces
        .list()
        .iter()
        .filter(|s| s.payload.profile == *profile)
    {
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
            match (
                forgot,
                forget_where(spaces, id, |mail| mail.profile == *profile),
            ) {
                (Forgot::Nothing, Forgot::Nothing) => Forgot::Nothing,
                _ => Forgot::Changed,
            }
        })
}

#[cfg(test)]
mod tests {
    use super::{Forgot, forget_account, forget_unknown};
    use crate::ui::space::{Mail, Recall, Scope, Spaces};
    use mail_core::profile::ProfileId;
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

    fn elsewhere(mail: Mail) -> Mail {
        Mail {
            profile: other(),
            ..mail
        }
    }

    fn other() -> ProfileId {
        "0123abcd".parse().expect("a profile id")
    }

    #[test]
    fn accounts_the_store_no_longer_has_leave_and_known_ones_stay() {
        // The Space of another Profile names accounts this store does not have, and keeps them.
        let foreign = elsewhere(scoped(&[3, 4], &[3, 4]));
        let mut all = spaces(vec![
            scoped(&[1, 2], &[1, 2, 3]),
            scoped(&[3], &[3]),
            foreign.clone(),
        ]);
        let left = all.list()[2].id;
        all.edit_recall(left, |recall| recall.account = Some(account(3)));
        let known = [account(1)];
        let profile = ProfileId::default();
        assert_eq!(forget_unknown(&mut all, &profile, &known), Forgot::Changed);
        assert_eq!(
            payloads(&all),
            vec![scoped(&[1], &[1]), scoped(&[], &[]), foreign]
        );
        assert_eq!(all.recall().of(left).account, Some(account(3)));
        assert_eq!(forget_unknown(&mut all, &profile, &known), Forgot::Nothing);
    }
}
