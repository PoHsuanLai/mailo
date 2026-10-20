//! Every account is in some Space: one the store has and no Space of its Profile holds (added
//! from a terminal, or before any window was open) joins the Space on screen, so it is never out
//! of sight. An account taken out of its last Space would come straight back, so the Space's
//! menu does not offer that ([`held_elsewhere`]).

use super::{SpaceId, Spaces};
use ds_settings::Fixup;
use mail_core::profile::ProfileId;
use porter_core::AccountId;

/// Put each of `known`, `profile`'s accounts, that no Space of `profile` holds into the current
/// Space, when that Space is `profile`'s. Whether any went in.
pub fn adopt_unplaced(spaces: &mut Spaces, profile: &ProfileId, known: &[AccountId]) -> Fixup {
    let current = spaces.current().id;
    if spaces.current().payload.profile != *profile {
        return Fixup::Kept;
    }
    let unplaced: Vec<AccountId> = known
        .iter()
        .filter(|id| {
            !spaces
                .list()
                .iter()
                .any(|space| space.payload.profile == *profile && space.payload.shows(id))
        })
        .cloned()
        .collect();
    if unplaced.is_empty() {
        return Fixup::Kept;
    }
    spaces.edit(current, |space| {
        for id in unplaced {
            space.payload.widen(profile, id);
        }
    });
    Fixup::Changed
}

/// Whether a Space other than `space`, of the same Profile, holds `account`: if none does,
/// taking it out of `space` would leave it in no Space.
pub fn held_elsewhere(spaces: &Spaces, space: SpaceId, account: &AccountId) -> bool {
    let Some(profile) = spaces.get(space).map(|one| one.payload.profile.clone()) else {
        return false;
    };
    spaces.list().iter().any(|other| {
        other.id != space && other.payload.profile == profile && other.payload.shows(account)
    })
}

#[cfg(test)]
mod tests {
    use super::{adopt_unplaced, held_elsewhere};
    use crate::ui::space::{Mail, SpaceId, Spaces};
    use ds_settings::Fixup;
    use mail_core::profile::ProfileId;
    use porter_core::AccountId;
    use uuid::Uuid;

    fn account(n: u128) -> AccountId {
        mail_domain::id::account_id_from_uuid(Uuid::from_u128(n))
    }

    fn of(ns: &[u128]) -> Mail {
        Mail::of(ns.iter().copied().map(account).collect())
    }

    fn other() -> ProfileId {
        "0123abcd".parse().expect("a profile id")
    }

    fn payloads(spaces: &Spaces) -> Vec<Mail> {
        spaces
            .list()
            .iter()
            .map(|space| space.payload.clone())
            .collect()
    }

    #[test]
    fn an_account_no_space_holds_joins_the_one_on_screen() {
        let profile = ProfileId::default();
        let mut spaces = Spaces::first_run([of(&[1]), of(&[2])], Mail::default);
        spaces.select(SpaceId(1));
        let known = [account(1), account(2), account(3)];
        assert_eq!(
            adopt_unplaced(&mut spaces, &profile, &known),
            Fixup::Changed
        );
        assert_eq!(payloads(&spaces), vec![of(&[1]), of(&[2, 3])]);
        assert_eq!(adopt_unplaced(&mut spaces, &profile, &known), Fixup::Kept);
    }

    #[test]
    fn another_profile_s_space_neither_holds_nor_takes_this_store_s_accounts() {
        let profile = ProfileId::default();
        let foreign = Mail {
            profile: other(),
            ..of(&[1])
        };
        // Account 1 is only in the other Profile's Space, so it is unplaced here.
        let mut spaces = Spaces::first_run([of(&[]), foreign.clone()], Mail::default);
        assert_eq!(
            adopt_unplaced(&mut spaces, &profile, &[account(1)]),
            Fixup::Changed
        );
        assert_eq!(payloads(&spaces), vec![of(&[1]), foreign.clone()]);

        // On screen, the other Profile's Space takes nothing.
        let mut on_foreign = Spaces::first_run([of(&[]), foreign.clone()], Mail::default);
        on_foreign.select(SpaceId(1));
        assert_eq!(
            adopt_unplaced(&mut on_foreign, &profile, &[account(1)]),
            Fixup::Kept
        );
        assert_eq!(payloads(&on_foreign), vec![of(&[]), foreign]);
    }

    #[test]
    fn an_account_is_held_elsewhere_only_by_another_space_of_its_profile() {
        let foreign = Mail {
            profile: other(),
            ..of(&[2])
        };
        let spaces = Spaces::first_run([of(&[1, 2]), of(&[1]), foreign], Mail::default);
        // (the Space, the account, whether another Space holds it)
        let cases = [
            (SpaceId(0), 1, true),
            (SpaceId(1), 1, true),
            (SpaceId(0), 2, false),
            (SpaceId(9), 1, false),
        ];
        for (space, n, want) in cases {
            assert_eq!(
                held_elsewhere(&spaces, space, &account(n)),
                want,
                "{space:?} {n}"
            );
        }
    }
}
