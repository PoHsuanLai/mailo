use super::*;
use crate::ui::space::Scope;

const A: AccountId = AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000d1"));
const B: AccountId = AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000d2"));
const C: AccountId = AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000d3"));
/// Keeps its mail here, so it has no link.
const LOCAL: AccountId = AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000d4"));

/// What is pressed, the Space's scope, and the accounts covered.
type Case<'a> = (&'a str, Option<AccountId>, Scope, &'a [AccountId]);

fn of(ids: &[AccountId]) -> Scope {
    Scope::Accounts(ids.to_vec())
}

#[test]
fn what_a_press_of_sync_covers() {
    let with_links: BTreeSet<AccountId> = [A, B, C].into();
    let cases: &[Case] = &[
        (
            "every account when nothing narrows it",
            None,
            Scope::All,
            &[A, B, C],
        ),
        ("a pressed tile is that account", Some(B), Scope::All, &[B]),
        ("the tile beats the Space", Some(B), of(&[A, C]), &[B]),
        ("a Space is its accounts", None, of(&[A, C]), &[A, C]),
        ("a Space of one", None, of(&[C]), &[C]),
        ("a Space of no accounts covers none", None, of(&[]), &[]),
        (
            "the local account has nothing to fetch",
            Some(LOCAL),
            Scope::All,
            &[],
        ),
        ("a Space of only local mail", None, of(&[LOCAL]), &[]),
        ("local mail beside a server", None, of(&[LOCAL, A]), &[A]),
    ];
    for (name, pressed, scope, expected) in cases {
        assert_eq!(in_scope(&with_links, *pressed, scope), *expected, "{name}");
    }
}

#[test]
fn the_window_asks_its_shell() {
    let with_links: BTreeSet<AccountId> = [A, B].into();
    let shell = Shell {
        scope: of(&[B]),
        ..Shell::default()
    };
    assert_eq!(in_view(&with_links, &shell), [B]);
    let pressed = Shell {
        account: Some(A),
        scope: of(&[B]),
        ..Shell::default()
    };
    assert_eq!(in_view(&with_links, &pressed), [A]);
}
