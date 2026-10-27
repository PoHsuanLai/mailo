//! An account removed from this computer leaves every Space.
//!
//! A Space limited to that account alone is limited to no account afterwards, and shows nothing
//! until one is added to it: accounts belong to Spaces, so the Space does not widen itself to
//! everyone else's mail.

use super::{Mail, SpaceId, Spaces};
use mail_client::profile::ProfileId;
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
            if mail.shows(&account) {
                mail.accounts.retain(|id| *id != account);
                forgot = Forgot::Changed;
            }
        });
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
        named.extend(space.payload.accounts.iter().cloned());
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
    use crate::ui::space::{Mail, Spaces};
    use mail_client::profile::ProfileId;
    use porter_core::AccountId;
    use uuid::Uuid;

    fn account(n: u128) -> AccountId {
        mail_domain::id::account_id_from_uuid(Uuid::from_u128(n))
    }

    fn scoped(ids: &[u128]) -> Mail {
        Mail::of(ids.iter().copied().map(account).collect())
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
            (scoped(&[1, 2]), scoped(&[2]), Forgot::Changed),
            // Its only account gone, the Space is of no account, not of every account.
            (scoped(&[1]), scoped(&[]), Forgot::Changed),
            (scoped(&[2, 3]), scoped(&[2, 3]), Forgot::Nothing),
        ];
        for (before, after, expect) in cases {
            let mut all = spaces(vec![before.clone()]);
            assert_eq!(forget_account(&mut all, account(1)), expect, "{before:?}");
            assert_eq!(payloads(&all), vec![after], "{before:?}");
        }
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
        let foreign = elsewhere(scoped(&[3, 4]));
        let mut all = spaces(vec![scoped(&[1, 2]), scoped(&[3]), foreign.clone()]);
        let known = [account(1)];
        let profile = ProfileId::default();
        assert_eq!(forget_unknown(&mut all, &profile, &known), Forgot::Changed);
        assert_eq!(payloads(&all), vec![scoped(&[1]), scoped(&[]), foreign]);
        assert_eq!(forget_unknown(&mut all, &profile, &known), Forgot::Nothing);
    }
}
