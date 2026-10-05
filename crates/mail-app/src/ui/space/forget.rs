//! An account removed from this computer leaves every Space: their scopes, their colours and
//! where each was left.
//!
//! A Space limited to that account alone is limited to nothing afterwards, and nothing is not a
//! scope the window can show (the list reads an empty scope as every account, the sidebar as
//! none), so it becomes a Space over every account: still a look and pins of its own, and the
//! Space editor can narrow it again. The other accounts' colours stay as they were stored.

use super::{Scope, Spaces};
use mail_domain::AccountId;

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
    for space in &mut spaces.spaces {
        if let Scope::Accounts(ids) = &mut space.scope
            && ids.contains(&account)
        {
            ids.retain(|id| *id != account);
            if ids.is_empty() {
                space.scope = Scope::All;
            }
            forgot = Forgot::Changed;
        }
        if space.colors.remove(&account).is_some() {
            forgot = Forgot::Changed;
        }
    }
    for recall in spaces.recall.values_mut() {
        if recall.account == Some(account) {
            recall.account = None;
            forgot = Forgot::Changed;
        }
    }
    forgot
}

#[cfg(test)]
mod tests {
    use super::{Forgot, forget_account};
    use crate::ui::space::{Recall, Scope, Space, Spaces};
    use mail_domain::AccountId;
    use std::collections::BTreeMap;
    use uuid::Uuid;

    fn account(n: u128) -> AccountId {
        AccountId::from_uuid(Uuid::from_u128(n))
    }

    fn scoped(ids: &[u128], colors: &[u128]) -> Space {
        Space {
            scope: Scope::Accounts(ids.iter().copied().map(account).collect()),
            colors: colors
                .iter()
                .map(|n| (account(*n), format!("#{n:06}")))
                .collect(),
            ..Space::default()
        }
    }

    #[test]
    fn the_account_leaves_every_space_and_nothing_else_does() {
        // (before, after, what forgetting account 1 reports)
        let cases: Vec<(Space, Space, Forgot)> = vec![
            (
                scoped(&[1, 2], &[1, 2]),
                scoped(&[2], &[2]),
                Forgot::Changed,
            ),
            // Limited to nothing is no scope: the Space shows every account.
            (
                scoped(&[1], &[1]),
                Space {
                    scope: Scope::All,
                    ..scoped(&[], &[])
                },
                Forgot::Changed,
            ),
            // Over every account, only its colour goes.
            (
                Space {
                    scope: Scope::All,
                    ..scoped(&[], &[1, 2])
                },
                Space {
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
            let mut spaces = Spaces {
                spaces: vec![before.clone()],
                current: 0,
                recall: BTreeMap::new(),
            };
            assert_eq!(
                forget_account(&mut spaces, account(1)),
                expect,
                "{before:?}"
            );
            assert_eq!(spaces.spaces, vec![after], "{before:?}");
        }
    }

    #[test]
    fn a_space_left_on_the_account_comes_back_on_every_account() {
        let recall = |tile: Option<u128>| Recall {
            place: "Inbox".to_owned(),
            open: None,
            account: tile.map(account),
        };
        let mut spaces = Spaces {
            spaces: vec![Space::default(), Space::default()],
            current: 0,
            recall: BTreeMap::from([(0, recall(Some(1))), (1, recall(Some(2)))]),
        };
        assert_eq!(forget_account(&mut spaces, account(1)), Forgot::Changed);
        assert_eq!(
            spaces.recall,
            BTreeMap::from([(0, recall(None)), (1, recall(Some(2)))])
        );
    }
}
